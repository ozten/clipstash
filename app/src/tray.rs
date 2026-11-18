//! Menu-bar shell (design section 4.1).

use crate::state::{AppState, CaptureState, PAUSED_INDEFINITELY};
use crate::windows::{self, Anchor};
use clipstash_core::{now_ms, ClearScope};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager};

const ICON_NORMAL: &[u8] = include_bytes!("../icons/tray-normal.png");
const ICON_PAUSED: &[u8] = include_bytes!("../icons/tray-paused.png");
const ICON_BLOCKED: &[u8] = include_bytes!("../icons/tray-blocked.png");

pub struct TrayHandles {
    pub tray: TrayIcon,
    status: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    pause5: MenuItem<tauri::Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let status = item("status", "clipstash")?;
    let pause = item("pause", "Pause")?;
    let pause5 = item("pause5", "Pause for 5 Minutes")?;
    let clear = Submenu::with_items(
        app,
        "Clear",
        true,
        &[&item("clear_5m", "Last 5 Minutes")?, &item("clear_15m", "Last 15 Minutes")?, &item("clear_all", "All History")?],
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &item("open_viewer", "Open Clipboard History")?,
            &item("open_library", "Open Snippet Library")?,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &pause5,
            &clear,
            &PredefinedMenuItem::separator(app)?,
            &item("settings", "Settings…")?,
            &item("quit", "Quit clipstash")?,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_NORMAL)?)
        .icon_as_template(true)
        .tooltip("clipstash")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(app)?;

    *app.state::<AppState>().tray.lock().unwrap() = Some(TrayHandles { tray, status, pause, pause5 });
    refresh(app);
    Ok(())
}

fn on_menu(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match id {
        "status" => {
            // Only actionable when something needs the user's attention.
            let cs = state.status().state;
            if matches!(cs, CaptureState::Blocked | CaptureState::Off) {
                windows::open_settings(app, true);
            }
        }
        "open_viewer" => windows::show_viewer(app, Anchor::Tray),
        "open_library" => windows::open_library(app),
        "pause" => {
            let mut rt = state.rt.lock().unwrap();
            rt.paused_until = match rt.paused_until {
                Some(_) => None,
                None => Some(PAUSED_INDEFINITELY),
            };
            drop(rt);
            status_changed(app);
        }
        "pause5" => {
            state.rt.lock().unwrap().paused_until = Some(now_ms() + 5 * 60_000);
            status_changed(app);
        }
        "clear_5m" | "clear_15m" | "clear_all" => {
            let scope = ClearScope::parse(id.trim_start_matches("clear_")).unwrap_or(ClearScope::Last5Min);
            let _ = state.history.lock().unwrap().clear(scope, now_ms());
            let _ = app.emit("clip-removed", ());
        }
        "settings" => windows::open_settings(app, false),
        "quit" => app.exit(0),
        _ => {}
    }
}

pub fn status_changed(app: &AppHandle) {
    refresh(app);
    let _ = app.emit("status-changed", ());
}

/// Update the status line, pause items and icon. The icon changes when
/// capture is paused or blocked so a loss of access is visible at a glance.
pub fn refresh(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = handle.state::<AppState>();
        let status = state.status();
        let guard = state.tray.lock().unwrap();
        let Some(t) = guard.as_ref() else { return };
        let (line, icon, tooltip) = match status.state {
            CaptureState::Capturing => ("Capturing".to_string(), ICON_NORMAL, "clipstash — capturing"),
            CaptureState::Paused => {
                let line = match status.paused_until {
                    Some(until) => {
                        let mins = ((until - now_ms()).max(0) + 59_999) / 60_000;
                        format!("Paused for {mins} min")
                    }
                    None => "Paused".to_string(),
                };
                (line, ICON_PAUSED, "clipstash — paused")
            }
            CaptureState::Blocked => {
                ("Clipboard access needed…".to_string(), ICON_BLOCKED, "clipstash — clipboard access needed")
            }
            CaptureState::Off => ("Capture is off — set up…".to_string(), ICON_PAUSED, "clipstash — capture is off"),
        };
        let _ = t.status.set_text(line);
        let _ = t.status.set_enabled(matches!(status.state, CaptureState::Blocked | CaptureState::Off));
        let paused = state.rt.lock().unwrap().paused_until.is_some();
        let _ = t.pause.set_text(if paused { "Resume" } else { "Pause" });
        let can_pause = status.state != CaptureState::Off;
        let _ = t.pause.set_enabled(can_pause);
        let _ = t.pause5.set_enabled(can_pause);
        if let Ok(image) = Image::from_bytes(icon) {
            let _ = t.tray.set_icon(Some(image));
            let _ = t.tray.set_icon_as_template(true);
        }
        let _ = t.tray.set_tooltip(Some(tooltip));
    });
}
