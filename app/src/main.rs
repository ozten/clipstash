//! clipstash menu-bar app: Tauri shell over `core` and `platform-macos`.

mod commands;
mod engine;
mod state;
mod tray;
mod watcher;
mod windows;

use state::AppState;
use tauri::http::{Response, StatusCode};
use tauri::{ActivationPolicy, Manager, RunEvent, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;

fn handle_args(app: &tauri::AppHandle, args: &[String]) {
    if args.iter().any(|a| a == "--library") {
        windows::open_library(app);
    } else if args.iter().any(|a| a == "--viewer") {
        windows::show_viewer(app, windows::Anchor::Center);
    } else if args.iter().any(|a| a == "--settings") {
        windows::open_settings(app, false);
    }
}

/// `stash://localhost/clip/<id>` and `stash://localhost/snippet/<id>` serve
/// image bytes to the webviews without widening filesystem access.
fn serve_image(app: &tauri::AppHandle, path: &str) -> Option<Vec<u8>> {
    let state = app.state::<AppState>();
    let mut parts = path.trim_matches('/').split('/');
    let (kind, id) = (parts.next()?, parts.next()?.parse::<i64>().ok()?);
    match kind {
        "clip" => state.history.lock().unwrap().read_blob(id).ok(),
        "snippet" => state.snippets.lock().unwrap().blob(id).ok().flatten(),
        _ => None,
    }
}

fn main() {
    let dir = clipstash_core::paths::data_dir();
    let (history, snippets) = match clipstash_core::open_stores(&dir) {
        Ok(stores) => stores,
        Err(e) => {
            eprintln!("clipstash: cannot open {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    let settings = snippets.settings().unwrap_or_default();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch (for example `clipstash library`) lands here.
            if args.iter().any(|a| a.starts_with("--")) {
                handle_args(app, &args);
            } else {
                windows::open_settings(app, false);
            }
        }))
        .plugin(tauri_nspanel::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        windows::toggle_viewer(app);
                    }
                })
                .build(),
        )
        .manage(AppState::new(history, snippets, settings))
        .register_uri_scheme_protocol("stash", |ctx, request| match serve_image(ctx.app_handle(), request.uri().path()) {
            Some(bytes) => Response::builder()
                .header("Content-Type", "image/png")
                .header("Cache-Control", "no-store")
                .body(bytes)
                .unwrap(),
            None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()).unwrap(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::get_clip,
            commands::paste,
            commands::copy,
            commands::delete_clip,
            commands::clear_history,
            commands::hide_viewer,
            commands::open_library,
            commands::open_settings,
            commands::promote_defaults,
            commands::promote,
            commands::list_collections,
            commands::create_collection,
            commands::rename_collection,
            commands::delete_collection,
            commands::reorder_collections,
            commands::list_snippets,
            commands::get_snippet,
            commands::create_snippet,
            commands::update_snippet,
            commands::delete_snippet,
            commands::duplicate_snippet,
            commands::export_collection,
            commands::import_collection,
            commands::get_settings,
            commands::default_settings,
            commands::get_status,
            commands::set_settings,
            commands::set_paused,
            commands::request_clipboard_access,
            commands::request_accessibility,
            commands::open_privacy_pane,
            commands::frontmost_app,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                windows::on_window_destroyed(window.app_handle(), window.label());
            }
        })
        .setup(|app| {
            // Menu-bar app: no Dock icon, absent from ⌘Tab.
            app.set_activation_policy(ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            let state = handle.state::<AppState>();
            let settings = state.settings();

            tray::build(&handle)?;
            windows::create_viewer(&handle)?;
            if let Err(e) = commands::register_hotkey(&handle, &settings.hotkey) {
                eprintln!("clipstash: could not register hotkey {}: {e}", settings.hotkey);
            }
            if let Err(e) = commands::sync_merge_tap(&handle) {
                eprintln!("clipstash: {e}");
            }
            watcher::spawn(handle.clone());
            watcher::spawn_retention(handle.clone());

            let args: Vec<String> = std::env::args().collect();
            handle_args(&handle, &args);
            // Onboarding: on first launch and whenever capture is not set up.
            if !settings.onboarding_done {
                windows::open_settings(&handle, true);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building clipstash");

    app.run(|_app, event| {
        // Closing the last window must not quit a menu-bar app.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
