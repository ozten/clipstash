//! Viewer panel (design section 7.1) and the two ordinary windows.
#![allow(clippy::unused_unit)] // the panel_event! macro syntax requires `-> ()`

use crate::state::AppState;
use tauri::{ActivationPolicy, AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri_nspanel::{tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask};

pub const VIEWER: &str = "viewer";
pub const LIBRARY: &str = "library";
pub const SETTINGS: &str = "settings";

const VIEWER_W: f64 = 780.0;
const VIEWER_H: f64 = 470.0;

tauri_panel! {
    panel!(ViewerPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })

    panel_event!(ViewerPanelEvents {
        window_did_resign_key(notification: &NSNotification) -> ()
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// Centered on the screen under the pointer (global hotkey).
    Center,
    /// Under the menu-bar icon.
    Tray,
}

/// Create the hidden viewer panel. A borderless, non-activating NSPanel: it
/// can take keyboard focus while the app underneath stays frontmost, which
/// is what lets the synthesized ⌘V land in that app.
pub fn create_viewer(app: &AppHandle) -> tauri::Result<()> {
    let panel = PanelBuilder::<_, ViewerPanel>::new(app, VIEWER)
        .url(WebviewUrl::App("viewer.html".into()))
        .title("Clipboard History")
        .size(tauri::Size::Logical(LogicalSize::new(VIEWER_W, VIEWER_H)))
        .level(PanelLevel::Floating)
        .floating(true)
        .has_shadow(true)
        .hides_on_deactivate(false)
        .no_activate(true)
        .transparent(true)
        .corner_radius(14.0)
        .style_mask(StyleMask::empty().nonactivating_panel())
        .collection_behavior(CollectionBehavior::new().move_to_active_space().full_screen_auxiliary())
        .with_window(|w| w.decorations(false).transparent(true).visible(false).resizable(false).skip_taskbar(true))
        .build()?;

    // Clicking elsewhere dismisses the viewer.
    let handler = ViewerPanelEvents::new();
    let handle = app.clone();
    handler.window_did_resign_key(move |_| {
        if let Ok(panel) = handle.get_webview_panel(VIEWER) {
            panel.hide();
        }
    });
    panel.set_event_handler(Some(handler.as_ref()));
    // The panel holds its delegate weakly; keep the handler for the app's lifetime.
    std::mem::forget(handler);
    Ok(())
}

fn position_viewer(app: &AppHandle, anchor: Anchor) {
    let Some(window) = app.get_webview_window(VIEWER) else { return };
    let Ok(cursor) = app.cursor_position() else { return };
    let monitors = app.available_monitors().unwrap_or_default();
    let contains = |m: &tauri::Monitor, x: f64, y: f64| {
        let (p, s) = (m.position(), m.size());
        x >= p.x as f64 && x < p.x as f64 + s.width as f64 && y >= p.y as f64 && y < p.y as f64 + s.height as f64
    };

    let tray_rect = match anchor {
        Anchor::Tray => {
            let state = app.state::<AppState>();
            let guard = state.tray.lock().unwrap();
            guard.as_ref().and_then(|t| t.tray.rect().ok().flatten())
        }
        Anchor::Center => None,
    };

    if let Some(rect) = tray_rect {
        let pos: PhysicalPosition<f64> = rect.position.to_physical(1.0);
        let size: tauri::PhysicalSize<f64> = rect.size.to_physical(1.0);
        if let Some(m) = monitors.iter().find(|m| contains(m, pos.x, pos.y)) {
            let scale = m.scale_factor();
            let w = VIEWER_W * scale;
            let left = m.position().x as f64;
            let right = left + m.size().width as f64;
            let x = (pos.x + size.width / 2.0 - w / 2.0).clamp(left + 8.0 * scale, right - w - 8.0 * scale);
            let y = pos.y + size.height + 6.0 * scale;
            let _ = window.set_position(PhysicalPosition::new(x, y));
            return;
        }
    }

    let monitor = monitors.iter().find(|m| contains(m, cursor.x, cursor.y)).or(monitors.first());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let (w, h) = (VIEWER_W * scale, VIEWER_H * scale);
        let x = m.position().x as f64 + (m.size().width as f64 - w) / 2.0;
        let y = m.position().y as f64 + ((m.size().height as f64 - h) * 0.38).max(0.0);
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

pub fn show_viewer(app: &AppHandle, anchor: Anchor) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Ok(panel) = handle.get_webview_panel(VIEWER) else { return };
        position_viewer(&handle, anchor);
        panel.show_and_make_key();
        let _ = handle.emit_to(VIEWER, "viewer-shown", ());
    });
}

/// Hide the viewer and wait until it is hidden.
pub fn hide_viewer(app: &AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(panel) = handle.get_webview_panel(VIEWER) {
            panel.hide();
        }
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
}

pub fn toggle_viewer(app: &AppHandle) {
    let visible = app.get_webview_panel(VIEWER).map(|p| p.is_visible()).unwrap_or(false);
    if visible {
        hide_viewer(app);
    } else {
        show_viewer(app, Anchor::Center);
    }
}

/// Open (or focus) an ordinary window. While one is open the app uses the
/// Regular activation policy so it can be reached with ⌘Tab.
fn open_window(app: &AppHandle, label: &'static str, title: &'static str, url: String, size: (f64, f64), min: (f64, f64)) {
    let handle = app.clone();
    // Windows are created off the main thread; Tauri marshals the work.
    std::thread::spawn(move || {
        let _ = handle.set_activation_policy(ActivationPolicy::Regular);
        let window = match handle.get_webview_window(label) {
            Some(w) => Some(w),
            None => WebviewWindowBuilder::new(&handle, label, WebviewUrl::App(url.into()))
                .title(title)
                .inner_size(size.0, size.1)
                .min_inner_size(min.0, min.1)
                // Tauri's file-drop handler would swallow HTML5 drag and drop in the library.
                .disable_drag_drop_handler()
                .build()
                .map_err(|e| eprintln!("clipstash: could not open {label}: {e}"))
                .ok(),
        };
        if let Some(w) = window {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    });
}

pub fn open_library(app: &AppHandle) {
    hide_viewer(app);
    open_window(app, LIBRARY, "Snippet Library", "library.html".into(), (980.0, 620.0), (720.0, 420.0));
}

pub fn open_settings(app: &AppHandle, onboarding: bool) {
    if onboarding {
        // An already-open settings window switches to the onboarding view.
        let _ = app.emit_to(SETTINGS, "show-onboarding", ());
    }
    let url = if onboarding { "settings.html#onboarding" } else { "settings.html" };
    open_window(app, SETTINGS, "clipstash Settings", url.into(), (640.0, 660.0), (560.0, 480.0));
}

/// Back to Accessory (no Dock icon, absent from ⌘Tab) once the last
/// ordinary window has closed.
pub fn on_window_destroyed(app: &AppHandle, label: &str) {
    let other = match label {
        LIBRARY => SETTINGS,
        SETTINGS => LIBRARY,
        _ => return,
    };
    if app.get_webview_window(other).is_none() {
        let _ = app.set_activation_policy(ActivationPolicy::Accessory);
    }
}
