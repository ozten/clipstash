//! Synthesized key events (design section 7.4, step 4). Needs Accessibility.

use crate::{Error, Result};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::thread::sleep;
use std::time::Duration;

const KEY_V: CGKeyCode = 9;
const KEY_LEFT: CGKeyCode = 123;

fn key(code: CGKeyCode, down: bool, flags: CGEventFlags) -> Result<()> {
    let source =
        CGEventSource::new(CGEventSourceStateID::CombinedSessionState).map_err(|_| Error::EventCreate)?;
    let event = CGEvent::new_keyboard_event(source, code, down).map_err(|_| Error::EventCreate)?;
    event.set_flags(flags);
    event.post(CGEventTapLocation::Session);
    Ok(())
}

/// Send ⌘V to the frontmost app.
pub fn paste() -> Result<()> {
    key(KEY_V, true, CGEventFlags::CGEventFlagCommand)?;
    key(KEY_V, false, CGEventFlags::CGEventFlagCommand)
}

/// Send ← `n` times; implements `{cursor}` after a paste.
pub fn press_left(n: usize) -> Result<()> {
    for _ in 0..n {
        key(KEY_LEFT, true, CGEventFlags::CGEventFlagNull)?;
        key(KEY_LEFT, false, CGEventFlags::CGEventFlagNull)?;
        sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
}

/// Seconds since the last keyboard or mouse input; drives the poll interval.
pub fn idle_seconds() -> f64 {
    const COMBINED_SESSION_STATE: i32 = 0;
    const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;
    unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE) }
}
