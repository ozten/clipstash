//! Frontmost app, opening URLs, sounds.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppInfo {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub bundle_path: Option<String>,
    pub pid: i32,
}

/// The frontmost application. Used as the source-app heuristic (design
/// section 5.4): the pasteboard does not say who wrote to it.
pub fn frontmost_app() -> Option<AppInfo> {
    autoreleasepool(|_| {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some(AppInfo {
            name: app.localizedName().map(|s| s.to_string()),
            bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
            bundle_path: app.bundleURL().and_then(|u| u.path()).map(|s| s.to_string()),
            pid: app.processIdentifier(),
        })
    })
}

/// Open a URL (including `x-apple.systempreferences:` deep links).
pub fn open_url(url: &str) -> bool {
    std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Settings deep links for the pasteboard privacy pane, newest form first
/// (design section 10.3). Which one works depends on the OS version.
pub const PASTEBOARD_PANE_URLS: [&str; 2] = [
    "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Pasteboard",
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Pasteboard",
];
pub const ACCESSIBILITY_PANE_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
pub const INPUT_MONITORING_PANE_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent";

pub fn open_pasteboard_pane() -> bool {
    PASTEBOARD_PANE_URLS.iter().any(|u| open_url(u))
}

/// Play a named system sound such as "Pop" or "Tink".
pub fn play_sound(name: &str) {
    autoreleasepool(|_| {
        let name = NSString::from_str(name);
        let sound: Option<Retained<AnyObject>> = unsafe { msg_send![class!(NSSound), soundNamed: &*name] };
        if let Some(sound) = sound {
            let _: bool = unsafe { msg_send![&*sound, play] };
        }
    })
}
