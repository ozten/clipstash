//! Tauri commands (design section 9.1).

use crate::engine::{self, Target};
use crate::state::{AppState, Status, PAUSED_INDEFINITELY};
use crate::{tray, windows};
use clipstash_core::snippets::{default_snippet_name, CollectionExport};
use clipstash_core::*;
use clipstash_platform_macos::tap::{MergeTap, TapEvent};
use clipstash_platform_macos::{login, pasteboard, permissions, workspace};
use serde_json::{json, Value};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

type CmdResult<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Longest text sent to the preview pane.
const PREVIEW_TEXT_CHARS: usize = 20_000;

// ---- viewer ---------------------------------------------------------------

#[tauri::command]
pub fn search(state: State<AppState>, query: String, filter: SearchFilter, limit: usize) -> CmdResult<Vec<SearchHit>> {
    let history = state.history.lock().unwrap();
    let snippets = state.snippets.lock().unwrap();
    clipstash_core::search(&history, &snippets, &query, filter, limit.clamp(1, 500), now_ms()).map_err(err)
}

#[tauri::command]
pub fn get_clip(state: State<AppState>, id: i64) -> CmdResult<Value> {
    let d = state.history.lock().unwrap().get(id).map_err(err)?.ok_or("that clip no longer exists")?;
    let mut v = serde_json::to_value(&d.clip).map_err(err)?;
    match d.clip.kind {
        ClipKind::Text => {
            let text = d.text.clone().unwrap_or_default();
            v["charCount"] = json!(text.chars().count());
            v["text"] = json!(truncate_chars(&text, PREVIEW_TEXT_CHARS));
        }
        ClipKind::Image => {
            v["text"] = json!(d.text);
            v["imageUrl"] = json!(format!("stash://localhost/clip/{id}"));
        }
        ClipKind::File => {
            v["paths"] = json!(d.paths());
        }
    }
    Ok(v)
}

#[tauri::command]
pub async fn paste(app: AppHandle, target: Target, plain: bool) -> CmdResult<bool> {
    engine::paste(&app, target, plain)
}

#[tauri::command]
pub async fn copy(app: AppHandle, target: Target, plain: Option<bool>) -> CmdResult<()> {
    engine::copy(&app, target, plain.unwrap_or(false))
}

#[tauri::command]
pub fn delete_clip(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    state.history.lock().unwrap().delete(id).map_err(err)?;
    let _ = app.emit("clip-removed", ());
    Ok(())
}

#[tauri::command]
pub fn clear_history(app: AppHandle, state: State<AppState>, scope: String) -> CmdResult<usize> {
    let scope = ClearScope::parse(&scope).ok_or("scope must be 5m, 15m or all")?;
    let n = state.history.lock().unwrap().clear(scope, now_ms()).map_err(err)?;
    let _ = app.emit("clip-removed", ());
    Ok(n)
}

#[tauri::command]
pub async fn hide_viewer(app: AppHandle) {
    windows::hide_viewer(&app);
}

#[tauri::command]
pub async fn open_library(app: AppHandle) {
    windows::open_library(&app);
}

#[tauri::command]
pub async fn open_settings(app: AppHandle, onboarding: Option<bool>) {
    windows::hide_viewer(&app);
    windows::open_settings(&app, onboarding.unwrap_or(false));
}

// ---- promote --------------------------------------------------------------

/// Pre-filled values for the promote form (design section 8.1).
#[tauri::command]
pub fn promote_defaults(state: State<AppState>, clip_id: i64) -> CmdResult<Value> {
    let d = state.history.lock().unwrap().get(clip_id).map_err(err)?.ok_or("that clip no longer exists")?;
    let last = state.settings.read().unwrap().last_collection_id;
    let collection_id = state.snippets.lock().unwrap().default_collection(last).map_err(err)?;
    let (name, body, image) = match d.clip.kind {
        ClipKind::Text => {
            let text = d.text.clone().unwrap_or_default();
            (default_snippet_name(&text), text, None)
        }
        ClipKind::File => {
            let paths = d.paths();
            let name = paths.first().and_then(|p| p.rsplit('/').next()).unwrap_or("Files").to_string();
            (name, paths.join("\n"), None)
        }
        ClipKind::Image => {
            ("Image".to_string(), d.text.clone().unwrap_or_default(), Some(format!("stash://localhost/clip/{clip_id}")))
        }
    };
    Ok(json!({ "name": name, "collectionId": collection_id, "keyword": "", "body": body, "imageUrl": image }))
}

#[tauri::command]
pub fn promote(app: AppHandle, state: State<AppState>, clip_id: i64, draft: SnippetDraft) -> CmdResult<Snippet> {
    let snippet = {
        let history = state.history.lock().unwrap();
        state.snippets.lock().unwrap().promote(&history, clip_id, &draft, now_ms()).map_err(err)?
    };
    remember_collection(&state, draft.collection_id);
    let _ = app.emit("snippets-changed", ());
    Ok(snippet)
}

fn remember_collection(state: &AppState, id: i64) {
    let mut settings = state.settings.write().unwrap();
    if settings.last_collection_id != Some(id) {
        settings.last_collection_id = Some(id);
        let _ = state.snippets.lock().unwrap().save_settings(&settings);
    }
}

// ---- collections ----------------------------------------------------------

#[tauri::command]
pub fn list_collections(state: State<AppState>) -> CmdResult<Vec<Collection>> {
    state.snippets.lock().unwrap().collections().map_err(err)
}

#[tauri::command]
pub fn create_collection(app: AppHandle, state: State<AppState>, name: String) -> CmdResult<Collection> {
    let c = state.snippets.lock().unwrap().create_collection(&name).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(c)
}

#[tauri::command]
pub fn rename_collection(app: AppHandle, state: State<AppState>, id: i64, name: String) -> CmdResult<()> {
    state.snippets.lock().unwrap().rename_collection(id, &name).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(())
}

#[tauri::command]
pub fn delete_collection(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    state.snippets.lock().unwrap().delete_collection(id).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(())
}

#[tauri::command]
pub fn reorder_collections(app: AppHandle, state: State<AppState>, ids: Vec<i64>) -> CmdResult<()> {
    state.snippets.lock().unwrap().reorder_collections(&ids).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(())
}

// ---- snippets -------------------------------------------------------------

fn snippet_json(s: &Snippet) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if s.has_blob {
        v["imageUrl"] = json!(format!("stash://localhost/snippet/{}?v={}", s.id, s.updated_at));
    }
    v
}

#[tauri::command]
pub fn list_snippets(state: State<AppState>, collection_id: Option<i64>, query: Option<String>) -> CmdResult<Vec<Value>> {
    let snippets = state.snippets.lock().unwrap();
    let query = query.unwrap_or_default();
    let list = if query.trim().is_empty() {
        snippets.list(collection_id).map_err(err)?
    } else {
        snippets
            .search(query.trim(), 500)
            .map_err(err)?
            .into_iter()
            .map(|(s, _)| s)
            .filter(|s| collection_id.is_none_or(|c| s.collection_id == c))
            .collect()
    };
    Ok(list.iter().map(snippet_json).collect())
}

#[tauri::command]
pub fn get_snippet(state: State<AppState>, id: i64) -> CmdResult<Value> {
    let s = state.snippets.lock().unwrap().get(id).map_err(err)?.ok_or("that snippet no longer exists")?;
    Ok(snippet_json(&s))
}

#[tauri::command]
pub fn create_snippet(app: AppHandle, state: State<AppState>, draft: SnippetDraft) -> CmdResult<Value> {
    let s = state.snippets.lock().unwrap().create(&draft, now_ms()).map_err(err)?;
    remember_collection(&state, draft.collection_id);
    let _ = app.emit("snippets-changed", ());
    Ok(snippet_json(&s))
}

#[tauri::command]
pub fn update_snippet(app: AppHandle, state: State<AppState>, id: i64, patch: SnippetPatch) -> CmdResult<Value> {
    let s = state.snippets.lock().unwrap().update(id, &patch, now_ms()).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(snippet_json(&s))
}

#[tauri::command]
pub fn delete_snippet(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<()> {
    state.snippets.lock().unwrap().delete(id).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(())
}

#[tauri::command]
pub fn duplicate_snippet(app: AppHandle, state: State<AppState>, id: i64) -> CmdResult<Value> {
    let s = state.snippets.lock().unwrap().duplicate(id, now_ms()).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(snippet_json(&s))
}

#[tauri::command]
pub fn export_collection(state: State<AppState>, id: i64, path: String) -> CmdResult<usize> {
    let data = state.snippets.lock().unwrap().export_collection(id).map_err(err)?;
    let json = serde_json::to_string_pretty(&data).map_err(err)?;
    std::fs::write(&path, json + "\n").map_err(err)?;
    Ok(data.snippets.len())
}

/// Import a collection file. Call with `mode: "skip"` first; if the report
/// lists conflicts, ask the user, then call again with `"overwrite"`.
#[tauri::command]
pub fn import_collection(app: AppHandle, state: State<AppState>, path: String, mode: ImportMode) -> CmdResult<ImportReport> {
    let raw = std::fs::read_to_string(&path).map_err(err)?;
    let data: CollectionExport = serde_json::from_str(&raw).map_err(|e| format!("not a clipstash collection file: {e}"))?;
    let report = state.snippets.lock().unwrap().import_collection(&data, mode, now_ms()).map_err(err)?;
    let _ = app.emit("snippets-changed", ());
    Ok(report)
}

// ---- settings, status, permissions ----------------------------------------

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> Status {
    state.status()
}

#[tauri::command]
pub fn default_settings() -> Settings {
    Settings::default()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<Settings> {
    let old = state.settings();
    let mut new = settings;
    new.ignore_apps = new.ignore_apps.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    new.max_text_bytes = new.max_text_bytes.max(1);
    new.max_image_bytes = new.max_image_bytes.max(1);

    let mut warning = None;
    if new.hotkey != old.hotkey {
        if let Err(e) = register_hotkey(&app, &new.hotkey) {
            let _ = register_hotkey(&app, &old.hotkey);
            return Err(format!("that shortcut cannot be used: {e}"));
        }
    }
    if new.launch_at_login != old.launch_at_login {
        if let Err(e) = login::set_enabled(new.launch_at_login) {
            new.launch_at_login = login::is_enabled();
            warning = Some(format!("launch at login: {e}"));
        }
    }

    state.snippets.lock().unwrap().save_settings(&new).map_err(err)?;
    *state.settings.write().unwrap() = new.clone();

    if let Err(e) = sync_merge_tap(&app) {
        warning = Some(e);
    }
    if new.retention_text != old.retention_text
        || new.retention_images != old.retention_images
        || new.retention_files != old.retention_files
    {
        if let Ok(n) = state.history.lock().unwrap().prune(&new, now_ms()) {
            if n > 0 {
                let _ = app.emit("clip-removed", ());
            }
        }
    }
    tray::status_changed(&app);
    let _ = app.emit("settings-changed", ());
    match warning {
        Some(w) => Err(w),
        None => Ok(new),
    }
}

/// Register the viewer hotkey, replacing any previous one.
pub fn register_hotkey(app: &AppHandle, hotkey: &str) -> CmdResult<()> {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    shortcuts.register(hotkey).map_err(err)
}

/// Install or remove the merge event tap to match the setting. The tap is
/// only present while merge is on (design section 5.5).
pub fn sync_merge_tap(app: &AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let wanted = state.settings.read().unwrap().merge_enabled;
    let mut tap = state.merge_tap.lock().unwrap();
    if !wanted {
        *tap = None;
        return Ok(());
    }
    if tap.is_some() {
        return Ok(());
    }
    let handle = app.clone();
    let installed = MergeTap::install(move |event| {
        let state = handle.state::<AppState>();
        let mut rt = state.rt.lock().unwrap();
        rt.last_cmd_c = Some(Instant::now());
        if event == TapEvent::DoubleCopy {
            rt.merge_pending = Some(Instant::now());
        }
    });
    match installed {
        Ok(t) => {
            *tap = Some(t);
            Ok(())
        }
        Err(e) => Err(format!("merge needs Accessibility or Input Monitoring: {e}")),
    }
}

#[tauri::command]
pub fn set_paused(app: AppHandle, state: State<AppState>, mode: String) -> CmdResult<()> {
    state.rt.lock().unwrap().paused_until = match mode.as_str() {
        "pause" => Some(PAUSED_INDEFINITELY),
        "5m" => Some(now_ms() + 5 * 60_000),
        "resume" => None,
        _ => return Err("mode must be pause, 5m or resume".into()),
    };
    tray::status_changed(&app);
    Ok(())
}

/// Onboarding step 2 (design section 10.3): one deliberate read so macOS
/// shows its alert and lists the app in Settings, then open the pane.
#[tauri::command]
pub async fn request_clipboard_access() -> CmdResult<&'static str> {
    let behavior = pasteboard::access_behavior();
    if !behavior.may_read() {
        // A denied read may block; never wait on it.
        std::thread::spawn(|| {
            let _ = pasteboard::read_text();
        });
        std::thread::sleep(std::time::Duration::from_millis(400));
        workspace::open_pasteboard_pane();
    }
    Ok(pasteboard::access_behavior().as_str())
}

#[tauri::command]
pub fn request_accessibility() -> bool {
    permissions::request_accessibility()
}

#[tauri::command]
pub fn open_privacy_pane(pane: String) -> bool {
    match pane.as_str() {
        "pasteboard" => workspace::open_pasteboard_pane(),
        "accessibility" => workspace::open_url(workspace::ACCESSIBILITY_PANE_URL),
        "inputMonitoring" => workspace::open_url(workspace::INPUT_MONITORING_PANE_URL),
        _ => false,
    }
}

/// The app in front right now: offered as "ignore this app" in settings.
#[tauri::command]
pub fn frontmost_app() -> Option<Value> {
    workspace::frontmost_app().map(|a| json!({ "name": a.name, "bundleId": a.bundle_id }))
}
