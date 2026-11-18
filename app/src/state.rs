use clipstash_core::{HistoryStore, Settings, SnippetStore};
use clipstash_platform_macos::tap::MergeTap;
use clipstash_platform_macos::{pasteboard, permissions, AccessBehavior};
use serde::Serialize;
use std::sync::{Mutex, RwLock};
use std::time::Instant;

/// `paused_until` value meaning "until the user resumes".
pub const PAUSED_INDEFINITELY: i64 = i64::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureState {
    /// H1: the user has not enabled capture.
    Off,
    /// Pasteboard access is missing; reading would alert or be refused.
    Blocked,
    Paused,
    Capturing,
}

pub struct Runtime {
    pub paused_until: Option<i64>,
    /// Change count produced by our own last pasteboard write.
    pub own_change_count: Option<i64>,
    pub capture_state: CaptureState,
    pub access: AccessBehavior,
    /// Last ⌘C seen by the merge tap; the watcher waits out the double-tap window.
    pub last_cmd_c: Option<Instant>,
    /// Set by a ⌘C double tap; the next text change is appended.
    pub merge_pending: Option<Instant>,
}

pub struct AppState {
    pub history: Mutex<HistoryStore>,
    pub snippets: Mutex<SnippetStore>,
    pub settings: RwLock<Settings>,
    pub rt: Mutex<Runtime>,
    pub merge_tap: Mutex<Option<MergeTap>>,
    pub tray: Mutex<Option<crate::tray::TrayHandles>>,
}

impl AppState {
    pub fn new(history: HistoryStore, snippets: SnippetStore, settings: Settings) -> AppState {
        AppState {
            history: Mutex::new(history),
            snippets: Mutex::new(snippets),
            settings: RwLock::new(settings),
            rt: Mutex::new(Runtime {
                paused_until: None,
                own_change_count: None,
                capture_state: CaptureState::Off,
                access: pasteboard::access_behavior(),
                last_cmd_c: None,
                merge_pending: None,
            }),
            merge_tap: Mutex::new(None),
            tray: Mutex::new(None),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }
}

pub fn compute_state(settings: &Settings, paused_until: Option<i64>, access: AccessBehavior, now: i64) -> CaptureState {
    if !settings.capture_enabled {
        CaptureState::Off
    } else if !access.may_read() {
        CaptureState::Blocked
    } else if paused_until.is_some_and(|until| until > now) {
        CaptureState::Paused
    } else {
        CaptureState::Capturing
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: CaptureState,
    /// Unix ms, or null when not paused or paused indefinitely.
    pub paused_until: Option<i64>,
    pub access_behavior: &'static str,
    pub accessibility: bool,
    pub accessibility_pane_name: &'static str,
    pub os_version: String,
    pub merge_tap_active: bool,
    pub launch_at_login: bool,
    pub clip_count: i64,
    pub snippet_count: i64,
}

impl AppState {
    pub fn status(&self) -> Status {
        let (maj, min, patch) = clipstash_platform_macos::os_version();
        let access = pasteboard::access_behavior();
        let settings = self.settings();
        let now = clipstash_core::now_ms();
        let paused_until = self.rt.lock().unwrap().paused_until;
        Status {
            state: compute_state(&settings, paused_until, access, now),
            paused_until: paused_until.filter(|u| *u != PAUSED_INDEFINITELY && *u > now),
            access_behavior: access.as_str(),
            accessibility: permissions::accessibility_trusted(),
            accessibility_pane_name: permissions::accessibility_pane_name(maj),
            os_version: format!("{maj}.{min}.{patch}"),
            merge_tap_active: self.merge_tap.lock().unwrap().is_some(),
            launch_at_login: clipstash_platform_macos::login::is_enabled(),
            clip_count: self.history.lock().unwrap().count().unwrap_or(0),
            snippet_count: self.snippets.lock().unwrap().count().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_precedence() {
        let off = Settings::default();
        let on = Settings { capture_enabled: true, ..Default::default() };
        let now = 1_000;
        assert_eq!(compute_state(&off, None, AccessBehavior::AlwaysAllow, now), CaptureState::Off);
        assert_eq!(compute_state(&on, None, AccessBehavior::AlwaysAllow, now), CaptureState::Capturing);
        assert_eq!(compute_state(&on, None, AccessBehavior::Unavailable, now), CaptureState::Capturing);
        for blocked in [AccessBehavior::Default, AccessBehavior::Ask, AccessBehavior::AlwaysDeny] {
            assert_eq!(compute_state(&on, None, blocked, now), CaptureState::Blocked);
            assert_eq!(compute_state(&on, Some(PAUSED_INDEFINITELY), blocked, now), CaptureState::Blocked);
        }
        assert_eq!(compute_state(&on, Some(now + 1), AccessBehavior::AlwaysAllow, now), CaptureState::Paused);
        assert_eq!(compute_state(&on, Some(now), AccessBehavior::AlwaysAllow, now), CaptureState::Capturing);
    }
}
