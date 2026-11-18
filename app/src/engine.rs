//! PasteEngine (design section 7.4).

use crate::state::AppState;
use crate::windows;
use clipstash_core::{now_ms, placeholder, ClipKind, Payload};
use clipstash_platform_macos::{events, pasteboard, permissions};
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// `{ clip: id }` or `{ snippet: id }`.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Clip(i64),
    Snippet(i64),
}

struct Prepared {
    payload: Payload,
    cursor_back: Option<usize>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Step 1: build the payload, resolving placeholders for snippets.
fn prepare(state: &AppState, target: Target) -> Result<Prepared, String> {
    match target {
        Target::Clip(id) => {
            let history = state.history.lock().unwrap();
            let d = history.get(id).map_err(err)?.ok_or("that clip no longer exists")?;
            let payload = match d.clip.kind {
                ClipKind::Text => Payload::Text { text: d.text.clone().unwrap_or_default(), rich: d.rich.clone() },
                ClipKind::File => Payload::Files { paths: d.paths() },
                ClipKind::Image => Payload::Image { png: history.read_blob(id).map_err(err)? },
            };
            Ok(Prepared { payload, cursor_back: None })
        }
        Target::Snippet(id) => {
            let snippets = state.snippets.lock().unwrap();
            let s = snippets.get(id).map_err(err)?.ok_or("that snippet no longer exists")?;
            if s.kind == "image" {
                let png = snippets.blob(id).map_err(err)?.ok_or("snippet has no image")?;
                return Ok(Prepared { payload: Payload::Image { png }, cursor_back: None });
            }
            drop(snippets);
            // {clipboard} is the live clipboard when we may read it without
            // an alert; otherwise the newest clip in history.
            let live = if pasteboard::access_behavior().may_read() { pasteboard::read_text() } else { None };
            let history = state.history.lock().unwrap();
            let expanded = placeholder::expand(&s.body, chrono::Local::now().naive_local(), &|n| match (n, &live) {
                (0, Some(text)) => Some(text.clone()),
                _ => history.nth_text(n).ok().flatten(),
            });
            Ok(Prepared { payload: Payload::text(expanded.text), cursor_back: expanded.cursor_back })
        }
    }
}

/// Steps 2 and 5: write the pasteboard, remember the change count so the
/// watcher skips it, and bump usage.
fn write(state: &AppState, target: Target, prepared: &Prepared, plain: bool) -> Result<(), String> {
    let count = pasteboard::write(&prepared.payload, plain).map_err(err)?;
    state.rt.lock().unwrap().own_change_count = Some(count);
    match target {
        Target::Clip(id) => state.history.lock().unwrap().touch(id, now_ms()).map_err(err),
        Target::Snippet(id) => state.snippets.lock().unwrap().bump_use(id).map_err(err),
    }
}

/// H9: put the item on the clipboard without pasting.
pub fn copy(app: &AppHandle, target: Target, plain: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let prepared = prepare(&state, target)?;
    write(&state, target, &prepared, plain)
}

/// Returns true if ⌘V was synthesized. False is the copy-only fallback: the
/// item is on the clipboard and the caller shows "Copied — press ⌘V".
pub fn paste(app: &AppHandle, target: Target, plain: bool) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let prepared = prepare(&state, target)?;
    write(&state, target, &prepared, plain)?;

    // Without Accessibility we cannot post events; during secure input we
    // stand down (design section 10.5).
    if !permissions::accessibility_trusted() || permissions::secure_input_active() {
        return Ok(false);
    }

    // Step 3: hide the panel. The app underneath never lost frontmost
    // status, so its window becomes key again and receives the paste.
    windows::hide_viewer(app);
    std::thread::sleep(Duration::from_millis(80));
    // Step 4.
    events::paste().map_err(err)?;
    if let Some(n) = prepared.cursor_back.filter(|n| *n > 0) {
        // Let the target app handle the paste before moving the caret.
        std::thread::sleep(Duration::from_millis(150));
        events::press_left(n).map_err(err)?;
    }
    Ok(true)
}
