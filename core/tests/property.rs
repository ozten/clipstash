//! Property: any sequence of capture, promote, clear and prune leaves
//! snippets intact (design section 13).

use clipstash_core::*;
use proptest::prelude::*;
use tempfile::TempDir;

#[derive(Clone, Debug)]
enum Op {
    CaptureText(String),
    CaptureImage(u8),
    CaptureFiles(u8),
    Promote(usize),
    Clear(u8),
    Prune,
    Delete(usize),
    Advance(u32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => "[a-c ]{1,6}".prop_map(Op::CaptureText),
        2 => any::<u8>().prop_map(Op::CaptureImage),
        1 => any::<u8>().prop_map(Op::CaptureFiles),
        4 => any::<usize>().prop_map(Op::Promote),
        2 => (0u8..3).prop_map(Op::Clear),
        2 => Just(Op::Prune),
        1 => any::<usize>().prop_map(Op::Delete),
        3 => (0u32..200_000_000).prop_map(Op::Advance),
    ]
}

fn png(salt: u8) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x02".to_vec();
    v.push(salt);
    v
}

type Expected = Vec<(i64, String, String, Option<Vec<u8>>)>;

fn snapshot(s: &SnippetStore) -> Expected {
    s.list(None)
        .unwrap()
        .into_iter()
        .map(|x| {
            let blob = s.blob(x.id).unwrap();
            (x.id, x.body, x.kind, blob)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn snippets_survive_any_history_operations(ops in proptest::collection::vec(op(), 1..60)) {
        let dir = TempDir::new().unwrap();
        let (h, s) = open_stores(dir.path()).unwrap();
        let col = s.default_collection(None).unwrap();
        let settings = Settings { retention_text: Retention::Day, retention_images: Retention::Day, ..Default::default() };
        let mut now: i64 = 1_800_000_000_000;
        let mut expected: Expected = Vec::new();

        for op in ops {
            now += 1;
            match op {
                Op::CaptureText(t) => {
                    if !t.trim().is_empty() {
                        h.capture(&Payload::text(t), Some("App"), None, now).unwrap();
                    }
                }
                Op::CaptureImage(salt) => { h.capture(&Payload::Image { png: png(salt) }, None, None, now).unwrap(); }
                Op::CaptureFiles(n) => { h.capture(&Payload::Files { paths: vec![format!("/f/{n}")] }, None, None, now).unwrap(); }
                Op::Promote(i) => {
                    let clips = h.list(None, 1000).unwrap();
                    if !clips.is_empty() {
                        let clip = &clips[i % clips.len()];
                        let detail = h.get(clip.id).unwrap().unwrap();
                        let body = detail.text.clone().unwrap_or_default();
                        let blob = (clip.kind == ClipKind::Image).then(|| h.read_blob(clip.id).unwrap());
                        let draft = SnippetDraft { collection_id: col, name: format!("s{now}"), keyword: None, body: body.clone() };
                        let snip = s.promote(&h, clip.id, &draft, now).unwrap();
                        expected.push((snip.id, body, snip.kind.clone(), blob));
                    }
                }
                Op::Clear(scope) => {
                    let scope = [ClearScope::Last5Min, ClearScope::Last15Min, ClearScope::All][scope as usize];
                    h.clear(scope, now).unwrap();
                }
                Op::Prune => { h.prune(&settings, now).unwrap(); }
                Op::Delete(i) => {
                    let clips = h.list(None, 1000).unwrap();
                    if !clips.is_empty() {
                        h.delete(clips[i % clips.len()].id).unwrap();
                    }
                }
                Op::Advance(ms) => now += ms as i64,
            }
            let mut got = snapshot(&s);
            got.sort_by_key(|x| x.0);
            prop_assert_eq!(&got, &expected);
        }

        // Blob directory never holds files that no clip references.
        let referenced = h.list(Some(ClipKind::Image), 10_000).unwrap().len();
        h.sweep_orphans().unwrap();
        let on_disk = std::fs::read_dir(dir.path().join("blobs")).unwrap().count();
        prop_assert_eq!(on_disk, referenced);

        // Wiping history entirely still leaves every snippet searchable.
        h.clear(ClearScope::All, now).unwrap();
        prop_assert_eq!(s.count().unwrap() as usize, expected.len());
        prop_assert_eq!(s.search("", 10_000).unwrap().len(), expected.len());
    }
}
