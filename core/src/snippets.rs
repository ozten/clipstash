//! `snippets.db`: durable collections and snippets (design section 6.2).
//! Also holds user settings, since they are durable too.

use crate::history::HistoryStore;
use crate::model::*;
use crate::paths;
use crate::search::{like_pattern, plan, rank, QueryPlan};
use crate::settings::Settings;
use crate::{Error, Result};
use base64::Engine;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS collection (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS snippet (
  id            INTEGER PRIMARY KEY,
  collection_id INTEGER NOT NULL REFERENCES collection(id) ON DELETE CASCADE,
  name          TEXT NOT NULL,
  keyword       TEXT,
  body          TEXT NOT NULL,
  kind          TEXT NOT NULL DEFAULT 'text',
  blob          BLOB,
  origin_app    TEXT,
  use_count     INTEGER NOT NULL DEFAULT 0,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS snippet_keyword ON snippet(keyword) WHERE keyword IS NOT NULL;
CREATE INDEX IF NOT EXISTS snippet_collection ON snippet(collection_id);

CREATE VIRTUAL TABLE IF NOT EXISTS snippet_fts USING fts5(
  name, keyword, body, content='snippet', content_rowid='id',
  tokenize='trigram'
);

CREATE TRIGGER IF NOT EXISTS snippet_ai AFTER INSERT ON snippet BEGIN
  INSERT INTO snippet_fts(rowid, name, keyword, body) VALUES (new.id, new.name, new.keyword, new.body);
END;
CREATE TRIGGER IF NOT EXISTS snippet_ad AFTER DELETE ON snippet BEGIN
  INSERT INTO snippet_fts(snippet_fts, rowid, name, keyword, body) VALUES ('delete', old.id, old.name, old.keyword, old.body);
END;
CREATE TRIGGER IF NOT EXISTS snippet_au AFTER UPDATE OF name, keyword, body ON snippet BEGIN
  INSERT INTO snippet_fts(snippet_fts, rowid, name, keyword, body) VALUES ('delete', old.id, old.name, old.keyword, old.body);
  INSERT INTO snippet_fts(rowid, name, keyword, body) VALUES (new.id, new.name, new.keyword, new.body);
END;

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

const COLS: &str = "s.id, s.collection_id, s.name, s.keyword, s.body, s.kind, s.blob IS NOT NULL, \
    s.origin_app, s.use_count, s.created_at, s.updated_at";

pub const DEFAULT_COLLECTION: &str = "General";

fn snippet_from_row(r: &Row<'_>) -> rusqlite::Result<Snippet> {
    Ok(Snippet {
        id: r.get(0)?,
        collection_id: r.get(1)?,
        name: r.get(2)?,
        keyword: r.get(3)?,
        body: r.get(4)?,
        kind: r.get(5)?,
        has_blob: r.get(6)?,
        origin_app: r.get(7)?,
        use_count: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn clean_keyword(k: Option<String>) -> Option<String> {
    k.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

fn map_constraint(e: rusqlite::Error, what: &str) -> Error {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            Error::Invalid(what.to_string())
        }
        _ => Error::Sqlite(e),
    }
}

/// On-disk format for one collection (design section 8.4).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct CollectionExport {
    pub collection: String,
    pub snippets: Vec<SnippetExport>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct SnippetExport {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(default)]
    pub body: String,
    /// Present only for image snippets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Base64 PNG for image snippets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    /// Leave existing snippets alone and report them as conflicts.
    Skip,
    /// Replace snippets that match on name within the collection.
    Overwrite,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub collection: String,
    pub collection_id: i64,
    pub created: usize,
    pub updated: usize,
    /// Names that already exist and were not overwritten.
    pub conflicts: Vec<String>,
}

pub struct SnippetStore {
    conn: Connection,
}

impl SnippetStore {
    pub fn open(dir: &Path) -> Result<SnippetStore> {
        paths::ensure_private_dir(dir)?;
        let path = dir.join("snippets.db");
        let conn = Connection::open(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        paths::protect_db(&path);
        Ok(SnippetStore { conn })
    }

    // ---- settings -------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'settings'", [], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    // ---- collections ----------------------------------------------------

    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.id, c.name, c.sort_order, (SELECT count(*) FROM snippet s WHERE s.collection_id = c.id)
             FROM collection c ORDER BY c.sort_order, c.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Collection { id: r.get(0)?, name: r.get(1)?, sort_order: r.get(2)?, count: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn collection_by_name(&self, name: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT id FROM collection WHERE name = ?1", [name], |r| r.get(0))
            .optional()?)
    }

    pub fn create_collection(&self, name: &str) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("collection name is empty".into()));
        }
        self.conn
            .execute(
                "INSERT INTO collection (name, sort_order)
                 VALUES (?1, (SELECT coalesce(max(sort_order), -1) + 1 FROM collection))",
                [name],
            )
            .map_err(|e| map_constraint(e, "a collection with that name already exists"))?;
        let id = self.conn.last_insert_rowid();
        let sort_order = self.conn.query_row("SELECT sort_order FROM collection WHERE id = ?1", [id], |r| r.get(0))?;
        Ok(Collection { id, name: name.to_string(), sort_order, count: 0 })
    }

    /// Find a collection by name, creating it if needed.
    pub fn ensure_collection(&self, name: &str) -> Result<i64> {
        match self.collection_by_name(name.trim())? {
            Some(id) => Ok(id),
            None => Ok(self.create_collection(name)?.id),
        }
    }

    /// The collection new snippets go to by default: the last used one if it
    /// still exists, else the first, else a new "General".
    pub fn default_collection(&self, last_used: Option<i64>) -> Result<i64> {
        let all = self.collections()?;
        if let Some(id) = last_used.filter(|id| all.iter().any(|c| c.id == *id)) {
            return Ok(id);
        }
        match all.first() {
            Some(c) => Ok(c.id),
            None => Ok(self.create_collection(DEFAULT_COLLECTION)?.id),
        }
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("collection name is empty".into()));
        }
        let n = self
            .conn
            .execute("UPDATE collection SET name = ?1 WHERE id = ?2", params![name, id])
            .map_err(|e| map_constraint(e, "a collection with that name already exists"))?;
        if n == 0 {
            return Err(Error::NotFound("collection"));
        }
        Ok(())
    }

    /// Deletes the collection and, by cascade, its snippets.
    pub fn delete_collection(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM collection WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Set the display order to the given id order.
    pub fn reorder_collections(&self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE collection SET sort_order = ?1 WHERE id = ?2", params![i as i64, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- snippets -------------------------------------------------------

    pub fn list(&self, collection: Option<i64>) -> Result<Vec<Snippet>> {
        let sql = format!(
            "SELECT {COLS} FROM snippet s WHERE (?1 IS NULL OR s.collection_id = ?1)
             ORDER BY s.name COLLATE NOCASE, s.id"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([collection], snippet_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Snippet>> {
        let sql = format!("SELECT {COLS} FROM snippet s WHERE s.id = ?1");
        Ok(self.conn.query_row(&sql, [id], snippet_from_row).optional()?)
    }

    pub fn blob(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT blob FROM snippet WHERE id = ?1", [id], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    pub fn create(&self, draft: &SnippetDraft, now: i64) -> Result<Snippet> {
        self.insert(draft, "text", None, None, now)
    }

    fn insert(
        &self,
        draft: &SnippetDraft,
        kind: &str,
        blob: Option<&[u8]>,
        origin_app: Option<&str>,
        now: i64,
    ) -> Result<Snippet> {
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("snippet name is empty".into()));
        }
        self.conn
            .execute(
                "INSERT INTO snippet (collection_id, name, keyword, body, kind, blob, origin_app, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    draft.collection_id,
                    name,
                    clean_keyword(draft.keyword.clone()),
                    draft.body,
                    kind,
                    blob,
                    origin_app,
                    now
                ],
            )
            .map_err(|e| map_constraint(e, "that keyword is already used, or the collection does not exist"))?;
        let id = self.conn.last_insert_rowid();
        self.get(id)?.ok_or(Error::NotFound("snippet"))
    }

    pub fn update(&self, id: i64, patch: &SnippetPatch, now: i64) -> Result<Snippet> {
        let cur = self.get(id)?.ok_or(Error::NotFound("snippet"))?;
        let name = patch.name.as_deref().map(str::trim).unwrap_or(&cur.name).to_string();
        if name.is_empty() {
            return Err(Error::Invalid("snippet name is empty".into()));
        }
        let keyword = match &patch.keyword {
            Some(k) => clean_keyword(k.clone()),
            None => cur.keyword.clone(),
        };
        self.conn
            .execute(
                "UPDATE snippet SET collection_id = ?1, name = ?2, keyword = ?3, body = ?4, updated_at = ?5 WHERE id = ?6",
                params![
                    patch.collection_id.unwrap_or(cur.collection_id),
                    name,
                    keyword,
                    patch.body.as_deref().unwrap_or(&cur.body),
                    now,
                    id
                ],
            )
            .map_err(|e| map_constraint(e, "that keyword is already used, or the collection does not exist"))?;
        self.get(id)?.ok_or(Error::NotFound("snippet"))
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM snippet WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Copy a snippet within its collection. The keyword is not copied
    /// because keywords are unique.
    pub fn duplicate(&self, id: i64, now: i64) -> Result<Snippet> {
        let cur = self.get(id)?.ok_or(Error::NotFound("snippet"))?;
        let blob = self.blob(id)?;
        let draft = SnippetDraft {
            collection_id: cur.collection_id,
            name: format!("{} copy", cur.name),
            keyword: None,
            body: cur.body,
        };
        self.insert(&draft, &cur.kind, blob.as_deref(), cur.origin_app.as_deref(), now)
    }

    pub fn bump_use(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE snippet SET use_count = use_count + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT count(*) FROM snippet", [], |r| r.get(0))?)
    }

    /// Promote (H13): **copy** a clip into the snippet library. Nothing links
    /// back to history, so pruning or clearing history cannot break a snippet.
    pub fn promote(&self, history: &HistoryStore, clip_id: i64, draft: &SnippetDraft, now: i64) -> Result<Snippet> {
        let detail = history.get(clip_id)?.ok_or(Error::NotFound("clip"))?;
        let origin = detail.clip.app_name.as_deref();
        match detail.clip.kind {
            ClipKind::Image => {
                let bytes = history.read_blob(clip_id)?;
                let mut draft = draft.clone();
                if draft.body.is_empty() {
                    draft.body = detail.text.clone().unwrap_or_default();
                }
                self.insert(&draft, "image", Some(&bytes), origin, now)
            }
            _ => self.insert(draft, "text", None, origin, now),
        }
    }

    /// Snippets matching the query, best first. An empty query lists all,
    /// most used first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(Snippet, f64)>> {
        let fetch = (limit.saturating_mul(4)).max(50) as i64;
        let mut hits: Vec<(Snippet, f64)> = match plan(query) {
            QueryPlan::Empty => {
                let sql = format!(
                    "SELECT {COLS} FROM snippet s ORDER BY s.use_count DESC, s.name COLLATE NOCASE LIMIT ?1"
                );
                let mut stmt = self.conn.prepare_cached(&sql)?;
                let rows = stmt.query_map([limit as i64], |r| Ok((snippet_from_row(r)?, 0.0)))?;
                return Ok(rows.collect::<rusqlite::Result<_>>()?);
            }
            QueryPlan::Fts(expr) => {
                // Weight name and keyword matches above body matches.
                let sql = format!(
                    "SELECT {COLS}, bm25(snippet_fts, 4.0, 6.0, 1.0) FROM snippet_fts
                     JOIN snippet s ON s.id = snippet_fts.rowid
                     WHERE snippet_fts MATCH ?1 ORDER BY 12 LIMIT ?2"
                );
                let mut stmt = self.conn.prepare_cached(&sql)?;
                let rows = stmt.query_map(params![expr, fetch], |r| {
                    Ok((snippet_from_row(r)?, -r.get::<_, f64>(11)?))
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            }
            QueryPlan::Like(tokens) => {
                let mut sql = format!("SELECT {COLS} FROM snippet s WHERE 1");
                let mut args = Vec::new();
                for t in &tokens {
                    args.push(Value::Text(like_pattern(t)));
                    let n = args.len();
                    sql.push_str(&format!(
                        " AND (s.name LIKE ?{n} ESCAPE '\\' OR s.keyword LIKE ?{n} ESCAPE '\\' OR s.body LIKE ?{n} ESCAPE '\\')"
                    ));
                }
                args.push(Value::Integer(fetch));
                sql.push_str(&format!(" ORDER BY s.use_count DESC, s.name COLLATE NOCASE LIMIT ?{}", args.len()));
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params_from_iter(args), |r| Ok((snippet_from_row(r)?, 1.0)))?;
                rows.collect::<rusqlite::Result<_>>()?
            }
        };
        rank(&mut hits, |s| 1.0 - 1.0 / (1.0 + s.use_count as f64));
        hits.truncate(limit);
        Ok(hits)
    }

    // ---- import / export ------------------------------------------------

    pub fn export_collection(&self, id: i64) -> Result<CollectionExport> {
        let name: String = self
            .conn
            .query_row("SELECT name FROM collection WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or(Error::NotFound("collection"))?;
        let mut snippets = Vec::new();
        for s in self.list(Some(id))? {
            let is_image = s.kind == "image";
            let blob = if is_image { self.blob(s.id)? } else { None };
            snippets.push(SnippetExport {
                name: s.name,
                keyword: s.keyword,
                body: s.body,
                kind: is_image.then(|| "image".to_string()),
                blob: blob.map(|b| base64::engine::general_purpose::STANDARD.encode(b)),
            });
        }
        Ok(CollectionExport { collection: name, snippets })
    }

    /// Import one collection file. Snippets match on name within the
    /// collection. With `ImportMode::Skip` existing ones are reported in
    /// `conflicts` so the caller can ask before re-running with `Overwrite`.
    pub fn import_collection(&self, data: &CollectionExport, mode: ImportMode, now: i64) -> Result<ImportReport> {
        let tx = self.conn.unchecked_transaction()?;
        let collection_id = self.ensure_collection(&data.collection)?;
        let mut report =
            ImportReport { collection: data.collection.trim().to_string(), collection_id, ..Default::default() };
        for item in &data.snippets {
            let name = item.name.trim();
            if name.is_empty() {
                continue;
            }
            let existing: Option<i64> = self
                .conn
                .query_row(
                    "SELECT id FROM snippet WHERE collection_id = ?1 AND name = ?2",
                    params![collection_id, name],
                    |r| r.get(0),
                )
                .optional()?;
            if existing.is_some() && mode == ImportMode::Skip {
                report.conflicts.push(name.to_string());
                continue;
            }
            let blob = match &item.blob {
                Some(b) => Some(
                    base64::engine::general_purpose::STANDARD
                        .decode(b)
                        .map_err(|e| Error::Invalid(format!("snippet {name:?}: bad image data: {e}")))?,
                ),
                None => None,
            };
            let kind = if blob.is_some() { "image" } else { "text" };
            // A keyword held by a different snippet is dropped, not stolen.
            let keyword = clean_keyword(item.keyword.clone()).filter(|k| {
                let owner: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM snippet WHERE keyword = ?1", [k], |r| r.get(0))
                    .optional()
                    .ok()
                    .flatten();
                owner.is_none() || owner == existing
            });
            match existing {
                Some(id) => {
                    self.conn.execute(
                        "UPDATE snippet SET keyword = ?1, body = ?2, kind = ?3, blob = ?4, updated_at = ?5 WHERE id = ?6",
                        params![keyword, item.body, kind, blob, now, id],
                    )?;
                    report.updated += 1;
                }
                None => {
                    self.conn.execute(
                        "INSERT INTO snippet (collection_id, name, keyword, body, kind, blob, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![collection_id, name, keyword, item.body, kind, blob, now],
                    )?;
                    report.created += 1;
                }
            }
        }
        tx.commit()?;
        Ok(report)
    }
}

/// Default snippet name when promoting: first line, at most 60 characters.
pub fn default_snippet_name(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(60).collect()
}
