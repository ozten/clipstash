use crate::model::ClipKind;
use serde::{Deserialize, Serialize};

/// How long a clip type is kept (feature H3).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Retention {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "1m")]
    Month,
    #[serde(rename = "3m")]
    ThreeMonths,
}

impl Retention {
    pub fn millis(self) -> i64 {
        const DAY: i64 = 24 * 60 * 60 * 1000;
        match self {
            Retention::Day => DAY,
            Retention::Week => 7 * DAY,
            Retention::Month => 30 * DAY,
            Retention::ThreeMonths => 90 * DAY,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MergeSeparator {
    Newline,
    Space,
    None,
}

impl MergeSeparator {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeSeparator::Newline => "\n",
            MergeSeparator::Space => " ",
            MergeSeparator::None => "",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// H1: capture is off until the user turns it on.
    pub capture_enabled: bool,
    pub onboarding_done: bool,

    pub capture_text: bool,
    pub capture_images: bool,
    pub capture_files: bool,

    pub retention_text: Retention,
    pub retention_images: Retention,
    pub retention_files: Retention,

    /// Bundle ids whose copies are dropped (H5). Best-effort: attribution is
    /// the frontmost app at the time the change is seen.
    pub ignore_apps: Vec<String>,

    pub max_text_bytes: usize,
    pub max_image_bytes: usize,
    /// Rich payloads over this size are discarded; the plain text is kept.
    pub max_rich_bytes: usize,

    pub hotkey: String,

    pub merge_enabled: bool,
    pub merge_separator: MergeSeparator,
    pub sound: bool,

    pub launch_at_login: bool,
    pub last_collection_id: Option<i64>,
}

pub const DEFAULT_HOTKEY: &str = "Alt+Super+KeyC";

pub fn default_ignore_apps() -> Vec<String> {
    [
        "com.1password.1password",
        "com.agilebits.onepassword7",
        "com.agilebits.onepassword-osx",
        "com.bitwarden.desktop",
        "com.lastpass.LastPass",
        "com.dashlane.dashlanephonefinal",
        "org.keepassxc.keepassxc",
        "in.sinew.Enpass-Desktop",
        "com.apple.Passwords",
        "com.apple.keychainaccess",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            capture_enabled: false,
            onboarding_done: false,
            capture_text: true,
            capture_images: true,
            capture_files: true,
            retention_text: Retention::Month,
            retention_images: Retention::Week,
            retention_files: Retention::Week,
            ignore_apps: default_ignore_apps(),
            max_text_bytes: 1024 * 1024,
            max_image_bytes: 20 * 1024 * 1024,
            max_rich_bytes: 256 * 1024,
            hotkey: DEFAULT_HOTKEY.to_string(),
            merge_enabled: false,
            merge_separator: MergeSeparator::Newline,
            sound: true,
            launch_at_login: false,
            last_collection_id: None,
        }
    }
}

impl Settings {
    pub fn kind_enabled(&self, kind: ClipKind) -> bool {
        match kind {
            ClipKind::Text => self.capture_text,
            ClipKind::Image => self.capture_images,
            ClipKind::File => self.capture_files,
        }
    }

    pub fn retention(&self, kind: ClipKind) -> Retention {
        match kind {
            ClipKind::Text => self.retention_text,
            ClipKind::Image => self.retention_images,
            ClipKind::File => self.retention_files,
        }
    }
}
