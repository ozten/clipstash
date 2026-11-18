use clipstash_core::snippets::{default_snippet_name, CollectionExport};
use clipstash_core::*;
use tempfile::TempDir;

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const T0: i64 = 1_800_000_000_000;

fn stores() -> (TempDir, HistoryStore, SnippetStore) {
    let dir = TempDir::new().unwrap();
    let (h, s) = open_stores(dir.path()).unwrap();
    (dir, h, s)
}

/// Minimal valid-looking PNG header with the given dimensions.
fn png(w: u32, h: u32, salt: u8) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend([8, 6, 0, 0, 0, salt]);
    v
}

fn add(h: &HistoryStore, text: &str, now: i64) -> i64 {
    h.capture(&Payload::text(text), Some("Safari"), Some("/Applications/Safari.app"), now).unwrap().id
}

#[test]
fn capture_and_list_in_last_used_order() {
    let (_d, h, _s) = stores();
    add(&h, "first", T0);
    add(&h, "second", T0 + 1);
    add(&h, "https://example.com/a?b=1", T0 + 2);
    let clips = h.list(None, 10).unwrap();
    assert_eq!(clips.iter().map(|c| c.preview.as_str()).collect::<Vec<_>>(), [
        "https://example.com/a?b=1",
        "second",
        "first"
    ]);
    assert!(clips[0].is_url);
    assert!(!clips[1].is_url);
    assert_eq!(clips[1].app_name.as_deref(), Some("Safari"));
    assert_eq!(clips[1].app_bundle.as_deref(), Some("/Applications/Safari.app"));
}

#[test]
fn duplicate_bumps_timestamp_instead_of_inserting() {
    let (_d, h, _s) = stores();
    let a = add(&h, "same", T0);
    add(&h, "other", T0 + 1);
    let again = h.capture(&Payload::text("same"), None, None, T0 + 5).unwrap();
    assert_eq!(again, CaptureOutcome { id: a, inserted: false });
    assert_eq!(h.count().unwrap(), 2);
    let top = &h.list(None, 1).unwrap()[0];
    assert_eq!((top.id, top.created_at, top.last_used), (a, T0, T0 + 5));
}

#[test]
fn kinds_round_trip() {
    let (d, h, _s) = stores();
    let rich = Rich { format: RichFormat::Rtf, data: b"{\\rtf1 hi}".to_vec() };
    let t = h.capture(&Payload::Text { text: "hi".into(), rich: Some(rich.clone()) }, None, None, T0).unwrap().id;
    let img = png(1280, 720, 1);
    let i = h.capture(&Payload::Image { png: img.clone() }, None, None, T0 + 1).unwrap().id;
    let paths = vec!["/tmp/a.pdf".to_string(), "/tmp/b c.txt".to_string()];
    let f = h.capture(&Payload::Files { paths: paths.clone() }, None, None, T0 + 2).unwrap().id;

    let td = h.get(t).unwrap().unwrap();
    assert_eq!(td.text.as_deref(), Some("hi"));
    assert_eq!(td.rich, Some(rich));
    assert!(td.clip.has_rich);

    let id = h.get(i).unwrap().unwrap();
    assert_eq!(id.clip.preview, "[image 1280×720]");
    assert_eq!(h.read_blob(i).unwrap(), img);
    assert!(id.blob_path.unwrap().starts_with(d.path().join("blobs")));

    let fd = h.get(f).unwrap().unwrap();
    assert_eq!(fd.paths(), paths);
    assert_eq!(fd.clip.preview, "/tmp/a.pdf, /tmp/b c.txt");

    assert_eq!(h.list(Some(ClipKind::Image), 10).unwrap().len(), 1);
    // Same bytes as text vs. as a file path are different clips.
    add(&h, "/tmp/a.pdf", T0 + 3);
    assert_eq!(h.count().unwrap(), 4);
}

#[test]
fn retention_boundaries_per_kind() {
    let (_d, h, _s) = stores();
    let settings = Settings {
        retention_text: Retention::Day,
        retention_images: Retention::Week,
        retention_files: Retention::ThreeMonths,
        ..Default::default()
    };
    let now = T0 + 100 * DAY;
    let keep_text = add(&h, "exactly at the boundary", now - DAY);
    add(&h, "one ms too old", now - DAY - 1);
    let old_img = h.capture(&Payload::Image { png: png(1, 1, 1) }, None, None, now - 7 * DAY - 1).unwrap().id;
    let new_img = h.capture(&Payload::Image { png: png(1, 1, 2) }, None, None, now - 6 * DAY).unwrap().id;
    let file = h.capture(&Payload::Files { paths: vec!["/x".into()] }, None, None, now - 89 * DAY).unwrap().id;
    let old_blob = h.get(old_img).unwrap().unwrap().blob_path.unwrap();
    let new_blob = h.get(new_img).unwrap().unwrap().blob_path.unwrap();

    assert_eq!(h.prune(&settings, now).unwrap(), 2);
    let ids: Vec<i64> = h.list(None, 10).unwrap().iter().map(|c| c.id).collect();
    assert_eq!(ids, [keep_text, new_img, file]);
    assert!(!old_blob.exists(), "orphaned blob is removed");
    assert!(new_blob.exists());
}

#[test]
fn retention_uses_last_used_not_created() {
    let (_d, h, _s) = stores();
    let settings = Settings { retention_text: Retention::Day, ..Default::default() };
    let now = T0 + 10 * DAY;
    let id = add(&h, "old but reused", now - 5 * DAY);
    h.touch(id, now - HOUR).unwrap();
    assert_eq!(h.prune(&settings, now).unwrap(), 0);
}

#[test]
fn clear_scopes_filter_on_created_at() {
    let (_d, h, _s) = stores();
    let now = T0 + DAY;
    let old = add(&h, "old clip reused just now", now - HOUR);
    h.touch(old, now).unwrap();
    let ten = add(&h, "ten minutes ago", now - 10 * MIN);
    add(&h, "two minutes ago", now - 2 * MIN);
    let img = h.capture(&Payload::Image { png: png(2, 2, 9) }, None, None, now - MIN).unwrap().id;
    let blob = h.get(img).unwrap().unwrap().blob_path.unwrap();

    assert_eq!(h.clear(ClearScope::Last5Min, now).unwrap(), 2);
    assert!(!blob.exists());
    let ids: Vec<i64> = h.list(None, 10).unwrap().iter().map(|c| c.id).collect();
    assert_eq!(ids, [old, ten]);

    assert_eq!(h.clear(ClearScope::Last15Min, now).unwrap(), 1);
    assert_eq!(h.clear(ClearScope::All, now).unwrap(), 1);
    assert_eq!(h.count().unwrap(), 0);
}

#[test]
fn delete_removes_row_blob_and_fts_entry() {
    let (_d, h, _s) = stores();
    let id = add(&h, "findable needle", T0);
    assert_eq!(h.search("needle", None, 10, T0).unwrap().len(), 1);
    assert!(h.delete(id).unwrap());
    assert!(h.search("needle", None, 10, T0).unwrap().is_empty());
    assert!(!h.delete(id).unwrap());
}

#[test]
fn search_is_substring_and_case_insensitive() {
    let (_d, h, _s) = stores();
    add(&h, "SELECT item, app FROM clipboard", T0);
    add(&h, "unrelated", T0 + 1);
    for q in ["lipboa", "select ITEM", "from clip", "cl", "S"] {
        let hits = h.search(q, None, 10, T0 + 2).unwrap();
        assert!(hits.iter().any(|(c, _)| c.preview.starts_with("SELECT")), "query {q:?}");
    }
    assert_eq!(h.search("item zzz", None, 10, T0).unwrap().len(), 0);
    // Source app name is searchable too.
    assert_eq!(h.search("safari", None, 10, T0).unwrap().len(), 2);
    // LIKE wildcards and FTS syntax in the query are literal.
    assert!(h.search("100%", None, 10, T0).unwrap().is_empty());
    assert!(h.search("\"quoted\" OR (x", None, 10, T0).unwrap().is_empty());
}

#[test]
fn fts_ranking_prefers_recent_among_equal_matches() {
    let (_d, h, _s) = stores();
    let now = T0 + 60 * DAY;
    let old = add(&h, "deploy the staging server", now - 50 * DAY);
    let new = add(&h, "deploy the staging cluster", now - MIN);
    let ids: Vec<i64> = h.search("staging", None, 10, now).unwrap().iter().map(|(c, _)| c.id).collect();
    assert_eq!(ids, [new, old]);
}

#[test]
fn fts_ranking_relevance_beats_recency() {
    let (_d, h, _s) = stores();
    let now = T0 + 60 * DAY;
    let strong = add(&h, "kubectl kubectl kubectl", now - 20 * DAY);
    let weak = add(
        &h,
        &format!("{} kubectl once in a very long and mostly unrelated document", "filler words ".repeat(80)),
        now - MIN,
    );
    add(&h, "noise one", now);
    add(&h, "noise two", now);
    let ids: Vec<i64> = h.search("kubectl", None, 10, now).unwrap().iter().map(|(c, _)| c.id).collect();
    assert_eq!(ids, [strong, weak]);
}

#[test]
fn unified_search_puts_snippets_first() {
    let (_d, h, s) = stores();
    let col = s.create_collection("Email").unwrap();
    let draft = SnippetDraft { collection_id: col.id, name: "Signature".into(), keyword: Some(";sig".into()), body: "Thanks,\n{cursor}".into() };
    let snip = s.create(&draft, T0).unwrap();
    add(&h, "my signature block", T0);
    add(&h, "something else", T0 + 1);

    let hits = search(&h, &s, "signature", SearchFilter::All, 10, T0 + 2).unwrap();
    assert!(matches!(&hits[0], SearchHit::Snippet(x) if x.id == snip.id));
    assert!(matches!(&hits[1], SearchHit::Clip(c) if c.preview == "my signature block"));
    assert_eq!(hits.len(), 2);

    // Empty filter: history only, in last_used order.
    let hits = search(&h, &s, "", SearchFilter::All, 10, T0 + 2).unwrap();
    assert!(hits.iter().all(|x| matches!(x, SearchHit::Clip(_))));
    assert_eq!(hits.len(), 2);

    // Type menu.
    assert_eq!(search(&h, &s, "", SearchFilter::Snippet, 10, T0).unwrap().len(), 1);
    assert_eq!(search(&h, &s, "signature", SearchFilter::Text, 10, T0).unwrap().len(), 1);
    assert_eq!(search(&h, &s, ";sig", SearchFilter::Snippet, 10, T0).unwrap().len(), 1);

    let json = serde_json::to_value(&hits[0]).unwrap();
    assert_eq!(json["type"], "clip");
    assert!(json["item"]["lastUsed"].is_i64());
}

#[test]
fn promote_copies_and_survives_history_wipe() {
    let (_d, h, s) = stores();
    let clip = add(&h, "SELECT 1;\nmore", T0);
    let img = png(640, 480, 3);
    let img_clip = h.capture(&Payload::Image { png: img.clone() }, Some("Preview"), None, T0).unwrap().id;
    let col = s.default_collection(None).unwrap();

    let name = default_snippet_name("SELECT 1;\nmore");
    assert_eq!(name, "SELECT 1;");
    let a = s
        .promote(&h, clip, &SnippetDraft { collection_id: col, name, keyword: None, body: "SELECT 1;\nmore".into() }, T0)
        .unwrap();
    let b = s
        .promote(&h, img_clip, &SnippetDraft { collection_id: col, name: "Logo".into(), ..Default::default() }, T0)
        .unwrap();
    assert_eq!(a.origin_app.as_deref(), Some("Safari"));
    assert_eq!((b.kind.as_str(), b.has_blob), ("image", true));
    assert_eq!(h.count().unwrap(), 2, "the clip stays in history");

    h.clear(ClearScope::All, T0).unwrap();
    assert_eq!(s.get(a.id).unwrap().unwrap().body, "SELECT 1;\nmore");
    assert_eq!(s.blob(b.id).unwrap().unwrap(), img);
}

#[test]
fn default_name_is_truncated_to_60_chars() {
    assert_eq!(default_snippet_name(&"é".repeat(100)).chars().count(), 60);
    assert_eq!(default_snippet_name("\n\n  hello  \nworld"), "hello");
}

#[test]
fn snippet_crud_and_keyword_uniqueness() {
    let (_d, _h, s) = stores();
    let a = s.create_collection("A").unwrap();
    let b = s.create_collection("B").unwrap();
    assert!(s.create_collection("A").is_err());
    let one = s.create(&SnippetDraft { collection_id: a.id, name: "One".into(), keyword: Some(";k".into()), body: "1".into() }, T0).unwrap();
    let dup = s.create(&SnippetDraft { collection_id: a.id, name: "Two".into(), keyword: Some(";k".into()), body: "2".into() }, T0);
    assert!(matches!(dup, Err(Error::Invalid(_))));
    // Empty keywords are stored as NULL, so many snippets can have none.
    s.create(&SnippetDraft { collection_id: a.id, name: "Two".into(), keyword: Some("".into()), body: "2".into() }, T0).unwrap();
    s.create(&SnippetDraft { collection_id: a.id, name: "Three".into(), keyword: Some(" ".into()), body: "3".into() }, T0).unwrap();

    let moved = s.update(one.id, &SnippetPatch { collection_id: Some(b.id), body: Some("uno".into()), ..Default::default() }, T0 + 1).unwrap();
    assert_eq!((moved.collection_id, moved.body.as_str(), moved.keyword.as_deref(), moved.updated_at), (b.id, "uno", Some(";k"), T0 + 1));
    let cleared = s.update(one.id, &SnippetPatch { keyword: Some(None), ..Default::default() }, T0 + 2).unwrap();
    assert_eq!(cleared.keyword, None);

    let copy = s.duplicate(one.id, T0 + 3).unwrap();
    assert_eq!(copy.name, "One copy");
    s.bump_use(one.id).unwrap();
    assert_eq!(s.get(one.id).unwrap().unwrap().use_count, 1);

    let cols = s.collections().unwrap();
    assert_eq!(cols.iter().map(|c| (c.name.as_str(), c.count)).collect::<Vec<_>>(), [("A", 2), ("B", 2)]);
    s.reorder_collections(&[b.id, a.id]).unwrap();
    assert_eq!(s.collections().unwrap()[0].name, "B");

    s.delete_collection(b.id).unwrap();
    assert!(s.get(one.id).unwrap().is_none(), "cascade");
    assert!(s.search("uno", 10).unwrap().is_empty(), "fts follows cascade delete");
}

#[test]
fn patch_json_distinguishes_absent_and_null_keyword() {
    let p: SnippetPatch = serde_json::from_str(r#"{"name":"x"}"#).unwrap();
    assert_eq!(p.keyword, None);
    let p: SnippetPatch = serde_json::from_str(r#"{"keyword":null}"#).unwrap();
    assert_eq!(p.keyword, Some(None));
    let p: SnippetPatch = serde_json::from_str(r#"{"keyword":";a"}"#).unwrap();
    assert_eq!(p.keyword, Some(Some(";a".into())));
}

#[test]
fn snippet_search_prefers_used_and_name_matches() {
    let (_d, _h, s) = stores();
    let c = s.create_collection("C").unwrap().id;
    let mk = |name: &str, body: &str| s.create(&SnippetDraft { collection_id: c, name: name.into(), keyword: None, body: body.into() }, T0).unwrap().id;
    let a = mk("Address home", "1 Main St");
    let b = mk("Address work", "2 Main St");
    for _ in 0..5 {
        s.bump_use(b).unwrap();
    }
    let ids: Vec<i64> = s.search("address", 10).unwrap().iter().map(|(x, _)| x.id).collect();
    assert_eq!(ids, [b, a]);
}

#[test]
fn export_import_round_trip_and_conflicts() {
    let (_d, h, s) = stores();
    let c = s.create_collection("SQL").unwrap().id;
    s.create(&SnippetDraft { collection_id: c, name: "Recent clips".into(), keyword: Some(";rc".into()), body: "SELECT …".into() }, T0).unwrap();
    let img_clip = h.capture(&Payload::Image { png: png(3, 3, 3) }, None, None, T0).unwrap().id;
    s.promote(&h, img_clip, &SnippetDraft { collection_id: c, name: "Pic".into(), ..Default::default() }, T0).unwrap();

    let exported = s.export_collection(c).unwrap();
    let json = serde_json::to_string_pretty(&exported).unwrap();
    assert!(json.contains("\"collection\": \"SQL\""));

    // Spec format (no kind/blob fields) parses.
    let spec: CollectionExport = serde_json::from_str(
        r#"{"collection":"SQL","snippets":[{"name":"Recent clips","keyword":";rc","body":"SELECT 2"},{"name":"New","body":"n"}]}"#,
    )
    .unwrap();
    let r = s.import_collection(&spec, ImportMode::Skip, T0 + 1).unwrap();
    assert_eq!((r.created, r.updated, r.conflicts.clone()), (1, 0, vec!["Recent clips".to_string()]));
    assert_eq!(s.search("Recent", 10).unwrap()[0].0.body, "SELECT …", "skip leaves existing alone");
    let r = s.import_collection(&spec, ImportMode::Overwrite, T0 + 2).unwrap();
    assert_eq!((r.created, r.updated), (0, 2));
    let recent = &s.search("Recent", 10).unwrap()[0].0;
    assert_eq!((recent.body.as_str(), recent.keyword.as_deref()), ("SELECT 2", Some(";rc")));

    // Round trip into a fresh store.
    let other = TempDir::new().unwrap();
    let s2 = SnippetStore::open(other.path()).unwrap();
    let r = s2.import_collection(&exported, ImportMode::Skip, T0).unwrap();
    assert_eq!(r.created, 2);
    assert_eq!(s2.export_collection(r.collection_id).unwrap(), exported);
}

#[test]
fn merge_appends_to_previous_text_clip() {
    let (_d, h, _s) = stores();
    assert_eq!(h.merge_append("x", "\n", T0).unwrap(), None);
    let a = add(&h, "alpha", T0);
    h.capture(&Payload::Image { png: png(1, 1, 1) }, None, None, T0 + 1).unwrap();
    let (id, merged) = h.merge_append("beta", "\n", T0 + 2).unwrap().unwrap();
    assert_eq!((id, merged.as_str()), (a, "alpha\nbeta"));
    assert_eq!(h.get(a).unwrap().unwrap().text.as_deref(), Some("alpha\nbeta"));
    assert_eq!(h.search("beta", None, 10, T0 + 3).unwrap().len(), 1);
    assert!(h.search("alpha\nbeta", None, 10, T0 + 3).unwrap().len() == 1);
    // Re-copying the merged text dedupes against the merged row.
    assert_eq!(h.capture(&Payload::text("alpha\nbeta"), None, None, T0 + 4).unwrap().id, a);
    assert_eq!(h.nth_text(0).unwrap().as_deref(), Some("alpha\nbeta"));
    assert_eq!(h.nth_text(1).unwrap(), None);
}

#[test]
fn settings_round_trip_and_defaults() {
    let (_d, _h, s) = stores();
    let d = s.settings().unwrap();
    assert!(!d.capture_enabled, "H1: off by default");
    assert!(!d.merge_enabled);
    assert_eq!((d.max_text_bytes, d.max_image_bytes, d.max_rich_bytes), (1 << 20, 20 << 20, 256 << 10));
    let mut n = d.clone();
    n.capture_enabled = true;
    n.retention_text = Retention::ThreeMonths;
    s.save_settings(&n).unwrap();
    assert_eq!(s.settings().unwrap(), n);
    let json = serde_json::to_value(&n).unwrap();
    assert_eq!(json["retentionText"], "3m");
}

#[cfg(unix)]
#[test]
fn files_are_user_only() {
    use std::os::unix::fs::PermissionsExt;
    let (d, h, _s) = stores();
    let id = h.capture(&Payload::Image { png: png(1, 1, 1) }, None, None, T0).unwrap().id;
    let blob = h.get(id).unwrap().unwrap().blob_path.unwrap();
    for p in [d.path().join("history.db"), d.path().join("snippets.db"), blob] {
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600, "{p:?}");
    }
}
