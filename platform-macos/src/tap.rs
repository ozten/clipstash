//! Listen-only event tap that detects "⌘ held, C tapped twice" for merge
//! (design section 5.5). Installed only while merge is enabled.

use crate::{Error, Result};
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type TapCallback = unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(event: CGEventRef) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: CFMachPortRef, order: isize) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: *const c_void);
}

const SESSION_EVENT_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_FLAGS_CHANGED: u32 = 12;
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const EVENT_TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;
const FIELD_AUTOREPEAT: u32 = 8;
const FIELD_KEYCODE: u32 = 9;
const FLAG_COMMAND: u64 = 0x0010_0000;
const KEY_C: i64 = 8;

pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);

/// Pure double-tap detector, separated from the tap so it can be tested.
#[derive(Default)]
pub struct DoubleCopyDetector {
    last_c: Option<Instant>,
}

impl DoubleCopyDetector {
    /// Feed a non-repeat ⌘C key-down. Returns true on the second tap within
    /// the window while ⌘ stayed held.
    pub fn command_c(&mut self, at: Instant) -> bool {
        match self.last_c.take() {
            Some(prev) if at.duration_since(prev) <= DOUBLE_TAP_WINDOW => true,
            _ => {
                self.last_c = Some(at);
                false
            }
        }
    }

    /// ⌘ was released, or another key was pressed: start over.
    pub fn reset(&mut self) {
        self.last_c = None;
    }
}

/// What the tap reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TapEvent {
    /// A first ⌘C. The watcher holds off briefly in case a second follows.
    Copy,
    /// The second ⌘C of a double tap: merge the next pasteboard change.
    DoubleCopy,
}

struct TapState {
    detector: DoubleCopyDetector,
    port: CFMachPortRef,
    on_event: Box<dyn Fn(TapEvent) + Send>,
}

unsafe extern "C" fn callback(_proxy: CGEventTapProxy, ty: u32, event: CGEventRef, info: *mut c_void) -> CGEventRef {
    let state = &mut *(info as *mut TapState);
    match ty {
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER => {
            if !state.port.is_null() {
                CGEventTapEnable(state.port, true);
            }
        }
        EVENT_FLAGS_CHANGED => {
            if CGEventGetFlags(event) & FLAG_COMMAND == 0 {
                state.detector.reset();
            }
        }
        EVENT_KEY_DOWN => {
            let command = CGEventGetFlags(event) & FLAG_COMMAND != 0;
            let code = CGEventGetIntegerValueField(event, FIELD_KEYCODE);
            let repeat = CGEventGetIntegerValueField(event, FIELD_AUTOREPEAT) != 0;
            if command && code == KEY_C {
                if !repeat {
                    let double = state.detector.command_c(Instant::now());
                    (state.on_event)(if double { TapEvent::DoubleCopy } else { TapEvent::Copy });
                }
            } else {
                state.detector.reset();
            }
        }
        _ => {}
    }
    event
}

/// A running tap. Dropping it removes the tap and stops its thread.
pub struct MergeTap {
    run_loop: Arc<AtomicPtr<c_void>>,
    thread: Option<JoinHandle<()>>,
}

impl MergeTap {
    /// Install the tap on its own thread. Fails with `TapDenied` when neither
    /// Accessibility nor Input Monitoring is granted.
    pub fn install(on_event: impl Fn(TapEvent) + Send + 'static) -> Result<MergeTap> {
        let run_loop = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let rl = run_loop.clone();
        let (tx, rx) = mpsc::channel::<bool>();
        let thread = std::thread::Builder::new()
            .name("clipstash-merge-tap".into())
            .spawn(move || unsafe {
                let state = Box::into_raw(Box::new(TapState {
                    detector: DoubleCopyDetector::default(),
                    port: std::ptr::null_mut(),
                    on_event: Box::new(on_event),
                }));
                let mask = (1u64 << EVENT_KEY_DOWN) | (1u64 << EVENT_FLAGS_CHANGED);
                let port =
                    CGEventTapCreate(SESSION_EVENT_TAP, HEAD_INSERT, LISTEN_ONLY, mask, callback, state as *mut c_void);
                if port.is_null() {
                    drop(Box::from_raw(state));
                    let _ = tx.send(false);
                    return;
                }
                (*state).port = port;
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                let current = CFRunLoopGetCurrent();
                CFRunLoopAddSource(current, source, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);
                rl.store(current, Ordering::SeqCst);
                let _ = tx.send(true);
                CFRunLoopRun();
                CGEventTapEnable(port, false);
                CFRelease(source);
                CFRelease(port);
                drop(Box::from_raw(state));
            })
            .map_err(|e| Error::Other(e.to_string()))?;
        match rx.recv() {
            Ok(true) => Ok(MergeTap { run_loop, thread: Some(thread) }),
            _ => {
                let _ = thread.join();
                Err(Error::TapDenied)
            }
        }
    }
}

impl Drop for MergeTap {
    fn drop(&mut self) {
        let rl = self.run_loop.load(Ordering::SeqCst);
        if !rl.is_null() {
            unsafe { CFRunLoopStop(rl) };
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tap_within_window() {
        let mut d = DoubleCopyDetector::default();
        let t = Instant::now();
        assert!(!d.command_c(t));
        assert!(d.command_c(t + Duration::from_millis(200)));
        // The pair is consumed: a third tap starts a new pair.
        assert!(!d.command_c(t + Duration::from_millis(300)));
    }

    #[test]
    fn slow_second_tap_does_not_merge() {
        let mut d = DoubleCopyDetector::default();
        let t = Instant::now();
        assert!(!d.command_c(t));
        assert!(!d.command_c(t + Duration::from_millis(401)));
        assert!(d.command_c(t + Duration::from_millis(500)));
    }

    #[test]
    fn releasing_command_resets() {
        let mut d = DoubleCopyDetector::default();
        let t = Instant::now();
        assert!(!d.command_c(t));
        d.reset();
        assert!(!d.command_c(t + Duration::from_millis(100)));
    }
}
