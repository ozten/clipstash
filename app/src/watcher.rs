//! PasteboardWatcher (design section 5): poll `changeCount`, gate on
//! `accessBehavior`, run the capture filters, store.

use crate::state::{compute_state, AppState, CaptureState};
use crate::tray;
use clipstash_core::{evaluate, now_ms, precheck, CaptureMeta, Payload, Settings};
use clipstash_platform_macos::{events, pasteboard, workspace};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const ACTIVE_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_INTERVAL: Duration = Duration::from_secs(1);
const IDLE_AFTER_SECS: f64 = 60.0;
/// A little longer than the double-tap window, so a first ⌘C is not
/// captured on its own when a second one is about to turn it into a merge.
const MERGE_HOLD: Duration = Duration::from_millis(450);
const MERGE_VALID: Duration = Duration::from_secs(2);

pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("clipstash-watcher".into())
        .spawn(move || run(app))
        .expect("spawn watcher");
}

fn run(app: AppHandle) {
    // Whatever is on the pasteboard at launch was copied before we were
    // watching; do not capture it.
    let mut last_count = pasteboard::change_count();
    let mut last_seen = None;

    loop {
        std::thread::sleep(if events::idle_seconds() > IDLE_AFTER_SECS { IDLE_INTERVAL } else { ACTIVE_INTERVAL });

        let state = app.state::<AppState>();
        let settings = state.settings();
        let now = now_ms();
        // Cheap property read. Checking it every tick (not only on change)
        // lets the tray and onboarding react as soon as the user grants access.
        let access = pasteboard::access_behavior();
        let capture_state = {
            let mut rt = state.rt.lock().unwrap();
            if rt.paused_until.is_some_and(|until| until <= now) {
                rt.paused_until = None;
            }
            rt.access = access;
            rt.capture_state = compute_state(&settings, rt.paused_until, access, now);
            rt.capture_state
        };
        if last_seen != Some((capture_state, access)) {
            last_seen = Some((capture_state, access));
            tray::refresh(&app);
            let _ = app.emit("status-changed", ());
        }

        let count = pasteboard::change_count();
        if count == last_count {
            continue;
        }
        if settings.merge_enabled {
            let held = state.rt.lock().unwrap().last_cmd_c.is_some_and(|t| t.elapsed() < MERGE_HOLD);
            if held {
                continue;
            }
        }
        last_count = count;
        // Off, paused or blocked: the change is skipped, never read.
        if capture_state != CaptureState::Capturing {
            continue;
        }
        process(&app, &state, &settings, count, now);
    }
}

fn process(app: &AppHandle, state: &AppState, settings: &Settings, count: i64, now: i64) {
    let types = pasteboard::types();
    let Some(kind) = pasteboard::classify(&types) else { return };
    let front = workspace::frontmost_app().unwrap_or_default();
    let meta = CaptureMeta {
        change_count: count,
        own_change_count: state.rt.lock().unwrap().own_change_count,
        types,
        app_bundle_id: front.bundle_id.clone(),
    };
    // Filters 1–4 need no content, so concealed or ignored data is never read.
    if precheck(settings, &meta, kind).is_err() {
        return;
    }
    let Some(payload) = pasteboard::read(kind, settings.max_rich_bytes) else { return };
    if evaluate(settings, &meta, &payload).is_err() {
        return;
    }

    let merge = state.rt.lock().unwrap().merge_pending.take().is_some_and(|t| t.elapsed() < MERGE_VALID);
    if let (true, Payload::Text { text, .. }) = (merge, &payload) {
        let merged = state.history.lock().unwrap().merge_append(text, settings.merge_separator.as_str(), now);
        if let Ok(Some((id, merged))) = merged {
            // Put the merged text on the clipboard so ⌘V pastes all of it.
            if let Ok(c) = pasteboard::write(&Payload::text(merged), true) {
                state.rt.lock().unwrap().own_change_count = Some(c);
            }
            if settings.sound {
                workspace::play_sound("Pop");
            }
            let _ = app.emit("clip-added", id);
            return;
        }
    }

    let outcome = state.history.lock().unwrap().capture(
        &payload,
        front.name.as_deref(),
        front.bundle_path.as_deref(),
        now,
    );
    match outcome {
        Ok(o) => {
            let _ = app.emit("clip-added", o.id);
        }
        Err(e) => eprintln!("clipstash: capture failed: {e}"),
    }
}

/// RetentionJob (design section 6.3): at launch and hourly.
pub fn spawn_retention(app: AppHandle) {
    std::thread::Builder::new()
        .name("clipstash-retention".into())
        .spawn(move || loop {
            let state = app.state::<AppState>();
            let settings = state.settings();
            let pruned = state.history.lock().unwrap().prune(&settings, now_ms());
            match pruned {
                Ok(n) if n > 0 => {
                    let _ = app.emit("clip-removed", ());
                }
                Err(e) => eprintln!("clipstash: retention failed: {e}"),
                _ => {}
            }
            std::thread::sleep(Duration::from_secs(3600));
        })
        .expect("spawn retention");
}
