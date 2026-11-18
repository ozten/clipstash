//! `history.db` + `blobs/`: disposable clipboard history (design section 6.1).

use crate::model::*;
use crate::paths;
use crate::search::{like_pattern, plan, rank, QueryPlan};
use crate::settings::Settings;
use crate::{Error, Result};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS clip (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('text','image','file')),
  text        TEXT,
  rich        BLOB,
  blob_path   TEXT,
  is_url      INTEGER NOT NULL DEFAULT 0,
  byte_size   INTEGER NOT NULL,
  hash        TEXT NOT NULL UNIQUE,
  app_name    TEXT,
  app_bundle  TEXT,
  created_at  INTEGER NOT NULL,
  last_used   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS clip_last_used ON clip(last_used DESC);
CREATE INDEX IF NOT EXISTS clip_kind ON clip(kind);

CREATE VIRTUAL TABLE IF NOT EXISTS clip_fts USING fts5(
  text, app_name, content='clip', content_rowid='id',
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS clip_ai AFTER INSERT ON clip BEGIN
  INSERT INTO clip_fts(rowid, text, app_name) VALUES (new.id, new.text, new.app_name);
END;
CREATE TRIGGER IF NOT EXISTS clip_ad AFTER DELETE ON clip BEGIN
  INSERT INTO clip_fts(clip_fts, rowid, text, app_name) VALUES ('delete', old.id, old.text, old.app_name);
END;
CREATE TRIGGER IF NOT EXISTS clip_au AFTER UPDATE OF text, app_name ON clip BEGIN
  INSERT INTO clip_fts(clip_fts, rowid, text, app_name) VALUES ('delete', old.id, old.text, old.app_name);
  INSERT INTO clip_fts(rowid, text, app_name) VALUES (new.id, new.text, new.app_name);
END;
"#;

/// Columns needed to build a `Clip`. Text clips only need a prefix.
const COLS: &str = "c.id, c.kind, \
    CASE WHEN c.kind = 'text' THEN substr(c.text, 1, 400) ELSE c.text END, \
    c.is_url, c.byte_size, c.app_name, c.app_bundle, c.created_at, c.last_used, c.rich IS NOT NULL";

const PREVIEW_CHARS: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CaptureOutcome {
    pub id: i64,
    /// False when an existing clip with the same content was refreshed.
    pub inserted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClearScope {
    Last5Min,
    Last15Min,
    All,
}

impl ClearScope {
    pub fn parse(s: &str) -> Option<ClearScope> {
        match s {
            "5m" => Some(ClearScope::Last5Min),
            "15m" => Some(ClearScope::Last15Min),
            "all" => Some(ClearScope::All),
            _ => None,
        }
    }
}

pub struct HistoryStore {
    conn: Connection,
    dir: PathBuf,
}

fn kind_from_sql(s: String) -> ClipKind {
    ClipKind::parse(&s).unwrap_or(ClipKind::Text)
}

fn preview_for(kind: ClipKind, text: Option<&str>) -> String {
    let text = text.unwrap_or("");
    match kind {
        ClipKind::Text => truncate_chars(text.trim(), PREVIEW_CHARS),
        ClipKind::Image => text.to_string(),
        ClipKind::File => {
            let paths: Vec<String> = serde_json::from_str(text).unwrap_or_default();
            let joined = paths.iter().map(|p| paths::abbreviate_home(p)).collect::<Vec<_>>().join(", ");
            truncate_chars(&joined, PREVIEW_CHARS)
        }
    }
}

fn clip_from_row(row: &Row<'_>) -> rusqlite::Result<Clip> {
    let kind = kind_from_sql(row.get(1)?);
    let text: Option<String> = row.get(2)?;
    Ok(Clip {
        id: row.get(0)?,
        kind,
        preview: preview_for(kind, text.as_deref()),
        is_url: row.get::<_, i64>(3)? != 0,
        byte_size: row.get(4)?,
        app_name: row.get(5)?,
        app_bundle: row.get(6)?,
        created_at: row.get(7)?,
        last_used: row.get(8)?,
        has_rich: row.get(9)?,
    })
}

impl HistoryStore {
    pub fn open(dir: &Path) -> Result<HistoryStore> {
        paths::ensure_private_dir(dir)?;
        paths::ensure_private_dir(&dir.join("blobs"))?;
        let path = dir.join("history.db");
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(SCHEMA)?;
        paths::protect_db(&path);
        Ok(HistoryStore { conn, dir: dir.to_path_buf() })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Store a clip. If the content hash already exists, bump that row's
    /// `last_used` instead of inserting (filter 6, design section 5.3).
    pub fn capture(
        &self,
        payload: &Payload,
        app_name: Option<&str>,
        app_bundle: Option<&str>,
        now: i64,
    ) -> Result<CaptureOutcome> {
        let hash = payload.hash();
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM clip WHERE hash = ?1", [&hash], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute("UPDATE clip SET last_used = ?1 WHERE id = ?2", params![now, id])?;
            return Ok(CaptureOutcome { id, inserted: false });
        }

        let (text, rich, blob_path, is_url): (String, Option<&[u8]>, Option<String>, bool) = match payload {
            Payload::Text { text, rich } => {
                (text.clone(), rich.as_ref().map(|r| r.data.as_slice()), None, looks_like_url(text))
            }
            Payload::Image { png } => {
                let rel = format!("blobs/{hash}.png");
                let abs = self.dir.join(&rel);
                std::fs::write(&abs, png)?;
                paths::set_mode(&abs, 0o600);
                let label = match png_dimensions(png) {
                    Some((w, h)) => format!("[image {w}×{h}]"),
                    None => "[image]".to_string(),
                };
                (label, None, Some(rel), false)
            }
            Payload::Files { paths } => (serde_json::to_string(paths)?, None, None, false),
        };

        self.conn.execute(
            "INSERT INTO clip (kind, text, rich, blob_path, is_url, byte_size, hash, app_name, app_bundle, created_at, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                payload.kind().as_str(),
                text,
                rich,
                blob_path,
                is_url as i64,
                payload.byte_size() as i64,
                hash,
                app_name,
                app_bundle,
                now
            ],
        )?;
        Ok(CaptureOutcome { id: self.conn.last_insert_rowid(), inserted: true })
    }

    /// Most recently used clips first.
    pub fn list(&self, kind: Option<ClipKind>, limit: usize) -> Result<Vec<Clip>> {
        let sql = format!(
            "SELECT {COLS} FROM clip c WHERE (?1 IS NULL OR c.kind = ?1)
             ORDER BY c.last_used DESC, c.id DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![kind.map(ClipKind::as_str), limit as i64], clip_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Clips matching the query with a relevance score in `0.0..=1.3`
    /// (text relevance plus a recency boost), best first.
    pub fn search(&self, query: &str, kind: Option<ClipKind>, limit: usize, now: i64) -> Result<Vec<(Clip, f64)>> {
        let kind_sql: Value = match kind {
            Some(k) => Value::Text(k.as_str().to_string()),
            None => Value::Null,
        };
        // Over-fetch so the recency boost can reorder candidates.
        let fetch = (limit.saturating_mul(4)).max(50) as i64;
        let mut hits: Vec<(Clip, f64)> = match plan(query) {
            QueryPlan::Empty => self.list(kind, limit)?.into_iter().map(|c| (c, 0.0)).collect(),
            QueryPlan::Fts(expr) => {
                let sql = format!(
                    "SELECT {COLS}, bm25(clip_fts) FROM clip_fts JOIN clip c ON c.id = clip_fts.rowid
                     WHERE clip_fts MATCH ?1 AND (?2 IS NULL OR c.kind = ?2)
                     ORDER BY bm25(clip_fts) LIMIT ?3"
                );
                let mut stmt = self.conn.prepare_cached(&sql)?;
                let rows = stmt.query_map(params![expr, kind_sql, fetch], |r| {
                    Ok((clip_from_row(r)?, -r.get::<_, f64>(10)?))
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            }
            QueryPlan::Like(tokens) => {
                let mut sql = format!("SELECT {COLS} FROM clip c WHERE (?1 IS NULL OR c.kind = ?1)");
                let mut args = vec![kind_sql];
                for t in &tokens {
                    args.push(Value::Text(like_pattern(t)));
                    let n = args.len();
                    sql.push_str(&format!(
                        " AND (c.text LIKE ?{n} ESCAPE '\\' OR c.app_name LIKE ?{n} ESCAPE '\\')"
                    ));
                }
                args.push(Value::Integer(fetch));
                sql.push_str(&format!(" ORDER BY c.last_used DESC, c.id DESC LIMIT ?{}", args.len()));
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params_from_iter(args), |r| Ok((clip_from_row(r)?, 1.0)))?;
                rows.collect::<rusqlite::Result<_>>()?
            }
        };
        if !matches!(plan(query), QueryPlan::Empty) {
            const WEEK_MS: f64 = 7.0 * 24.0 * 3600.0 * 1000.0;
            rank(&mut hits, |c| (-((now - c.last_used).max(0) as f64) / WEEK_MS).exp());
        }
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn get(&self, id: i64) -> Result<Option<ClipDetail>> {
        let sql = format!("SELECT {COLS}, c.text, c.rich, c.blob_path FROM clip c WHERE c.id = ?1");
        let detail = self
            .conn
            .query_row(&sql, [id], |r| {
                let clip = clip_from_row(r)?;
                let text: Option<String> = r.get(10)?;
                let rich: Option<Vec<u8>> = r.get(11)?;
                let blob: Option<String> = r.get(12)?;
                Ok((clip, text, rich, blob))
            })
            .optional()?;
        Ok(detail.map(|(clip, text, rich, blob)| ClipDetail {
            clip,
            text,
            rich: rich.map(Rich::sniff),
            blob_path: blob.map(|b| self.dir.join(b)),
        }))
    }

    /// Text of the nth most recent text clip (0 = newest). Backs `{clipboard:n}`.
    pub fn nth_text(&self, n: usize) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT text FROM clip WHERE kind = 'text' ORDER BY last_used DESC, id DESC LIMIT 1 OFFSET ?1",
                [n as i64],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Bump `last_used` after a paste or copy-back.
    pub fn touch(&self, id: i64, now: i64) -> Result<()> {
        self.conn.execute("UPDATE clip SET last_used = ?1 WHERE id = ?2", params![now, id])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<bool> {
        let blob: Option<Option<String>> = self
            .conn
            .query_row("SELECT blob_path FROM clip WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        let n = self.conn.execute("DELETE FROM clip WHERE id = ?1", [id])?;
        if let Some(Some(rel)) = blob {
            let _ = std::fs::remove_file(self.dir.join(rel));
        }
        Ok(n > 0)
    }

    /// Clear by `created_at`, so an old clip that was merely re-used recently
    /// is not caught by "last 5 minutes". Returns the number of clips removed.
    pub fn clear(&self, scope: ClearScope, now: i64) -> Result<usize> {
        let n = match scope {
            ClearScope::All => self.conn.execute("DELETE FROM clip", [])?,
            ClearScope::Last5Min => self.clear_since(now - 5 * 60_000)?,
            ClearScope::Last15Min => self.clear_since(now - 15 * 60_000)?,
        };
        self.sweep_orphans()?;
        Ok(n)
    }

    fn clear_since(&self, since: i64) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM clip WHERE created_at >= ?1", [since])?)
    }

    /// Retention job (design section 6.3): drop clips whose `last_used` is
    /// older than their type's retention period, then remove orphaned blobs.
    pub fn prune(&self, settings: &Settings, now: i64) -> Result<usize> {
        let mut removed = 0;
        for kind in ClipKind::ALL {
            let cutoff = now - settings.retention(kind).millis();
            removed += self
                .conn
                .execute("DELETE FROM clip WHERE kind = ?1 AND last_used < ?2", params![kind.as_str(), cutoff])?;
        }
        self.sweep_orphans()?;
        Ok(removed)
    }

    /// Delete files under `blobs/` that no clip references.
    pub fn sweep_orphans(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare("SELECT blob_path FROM clip WHERE blob_path IS NOT NULL")?;
        let live: HashSet<PathBuf> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .map(|rel| self.dir.join(rel))
            .collect();
        let mut removed = 0;
        for entry in std::fs::read_dir(self.dir.join("blobs"))? {
            let path = entry?.path();
            if path.is_file() && !live.contains(&path) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Merge (H12): append `extra` to the most recent text clip. Returns the
    /// clip id and the merged text, or `None` if there is no text clip yet.
    pub fn merge_append(&self, extra: &str, separator: &str, now: i64) -> Result<Option<(i64, String)>> {
        let prev: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, text FROM clip WHERE kind = 'text' ORDER BY last_used DESC, id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, old)) = prev else { return Ok(None) };
        let merged = format!("{old}{separator}{extra}");
        let payload = Payload::text(merged.clone());
        let hash = payload.hash();
        // The merged text may equal some other existing clip; keep one row.
        self.conn.execute("DELETE FROM clip WHERE hash = ?1 AND id != ?2", params![hash, id])?;
        self.conn.execute(
            "UPDATE clip SET text = ?1, rich = NULL, is_url = ?2, byte_size = ?3, hash = ?4, last_used = ?5 WHERE id = ?6",
            params![merged, looks_like_url(&merged) as i64, merged.len() as i64, hash, now, id],
        )?;
        Ok(Some((id, merged)))
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM clip", [], |r| r.get(0))?)
    }

    /// Read the image bytes of an image clip.
    pub fn read_blob(&self, id: i64) -> Result<Vec<u8>> {
        let detail = self.get(id)?.ok_or(Error::NotFound("clip"))?;
        let path = detail.blob_path.ok_or_else(|| Error::Invalid("clip has no image".into()))?;
        Ok(std::fs::read(path)?)
    }
}
