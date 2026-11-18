//! macOS platform layer: pasteboard, frontmost app, event synthesis, event
//! tap, permissions. Everything OS-specific lives here so `core` stays pure.

#![cfg(target_os = "macos")]

pub mod events;
pub mod login;
pub mod pasteboard;
pub mod permissions;
pub mod tap;
pub mod workspace;

pub use pasteboard::AccessBehavior;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("pasteboard write failed")]
    WriteFailed,
    #[error("could not create event source or event")]
    EventCreate,
    #[error("could not install the event tap; grant Accessibility or Input Monitoring")]
    TapDenied,
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// macOS version as (major, minor, patch).
pub fn os_version() -> (i64, i64, i64) {
    let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    (v.majorVersion as i64, v.minorVersion as i64, v.patchVersion as i64)
}
