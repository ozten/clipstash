//! NSPasteboard access (design sections 5.1, 5.2, 7.4, 10.2).

use crate::{Error, Result};
use clipstash_core::{ClipKind, Payload, Rich, RichFormat};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, sel};
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString, NSURL};

pub const TYPE_TEXT: &str = "public.utf8-plain-text";
pub const TYPE_RTF: &str = "public.rtf";
pub const TYPE_HTML: &str = "public.html";
pub const TYPE_PNG: &str = "public.png";
pub const TYPE_TIFF: &str = "public.tiff";
pub const TYPE_FILE_URL: &str = "public.file-url";

/// `NSPasteboard.accessBehavior` (macOS 15.4+).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessBehavior {
    /// The API does not exist (macOS 14 – 15.3): there is no gate.
    Unavailable,
    Default,
    Ask,
    AlwaysAllow,
    AlwaysDeny,
}

impl AccessBehavior {
    /// Whether background reads are safe: they will neither raise an alert
    /// nor be refused (design section 5.1).
    pub fn may_read(self) -> bool {
        matches!(self, AccessBehavior::Unavailable | AccessBehavior::AlwaysAllow)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AccessBehavior::Unavailable => "unavailable",
            AccessBehavior::Default => "default",
            AccessBehavior::Ask => "ask",
            AccessBehavior::AlwaysAllow => "alwaysAllow",
            AccessBehavior::AlwaysDeny => "alwaysDeny",
        }
    }
}

fn general() -> Retained<NSPasteboard> {
    NSPasteboard::generalPasteboard()
}

/// One integer read; never triggers a pasteboard alert.
pub fn change_count() -> i64 {
    autoreleasepool(|_| general().changeCount() as i64)
}

/// Current access state. Uses a raw message send guarded by
/// `respondsToSelector:` so it works on macOS 14 and with older bindings.
pub fn access_behavior() -> AccessBehavior {
    autoreleasepool(|_| {
        let pb = general();
        let responds: bool = unsafe { msg_send![&*pb, respondsToSelector: sel!(accessBehavior)] };
        if !responds {
            return AccessBehavior::Unavailable;
        }
        let raw: isize = unsafe { msg_send![&*pb, accessBehavior] };
        match raw {
            1 => AccessBehavior::Ask,
            2 => AccessBehavior::AlwaysAllow,
            3 => AccessBehavior::AlwaysDeny,
            _ => AccessBehavior::Default,
        }
    })
}

/// Type identifiers currently on the pasteboard. Reads metadata only.
pub fn types() -> Vec<String> {
    autoreleasepool(|_| {
        general().types().map(|a| a.iter().map(|t| t.to_string()).collect()).unwrap_or_default()
    })
}

/// Classify by the first matching type (design section 5.2): file URLs, then
/// images, then plain text.
///
/// One deliberate refinement: word processors and spreadsheets put a
/// rendered image next to the text they copy. When plain text and RTF are
/// both present the clip is text, not a picture of text.
pub fn classify<S: AsRef<str>>(types: &[S]) -> Option<ClipKind> {
    let has = |t: &str| types.iter().any(|x| x.as_ref() == t);
    if has(TYPE_FILE_URL) || has("NSFilenamesPboardType") {
        return Some(ClipKind::File);
    }
    let has_text = has(TYPE_TEXT) || has("NSStringPboardType");
    if has(TYPE_PNG) || has(TYPE_TIFF) {
        if has_text && has(TYPE_RTF) {
            return Some(ClipKind::Text);
        }
        return Some(ClipKind::Image);
    }
    has_text.then_some(ClipKind::Text)
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn data_for(pb: &NSPasteboard, ty: &str) -> Option<Vec<u8>> {
    pb.dataForType(&ns(ty)).map(|d| d.to_vec())
}

/// Read the pasteboard content as the given kind. This is a content read:
/// on macOS 15.4+ call it only when `access_behavior().may_read()`, or
/// deliberately during onboarding.
///
/// `max_rich` caps the optional RTF/HTML payload; a larger one is discarded.
pub fn read(kind: ClipKind, max_rich: usize) -> Option<Payload> {
    autoreleasepool(|_| {
        let pb = general();
        match kind {
            ClipKind::File => {
                let mut paths = Vec::new();
                if let Some(items) = pb.pasteboardItems() {
                    for item in items.iter() {
                        let Some(s) = item.stringForType(&ns(TYPE_FILE_URL)) else { continue };
                        let url: Option<Retained<NSURL>> = NSURL::URLWithString(&s);
                        if let Some(path) = url.and_then(|u| u.path()) {
                            paths.push(path.to_string());
                        }
                    }
                }
                (!paths.is_empty()).then_some(Payload::Files { paths })
            }
            ClipKind::Image => {
                if let Some(png) = data_for(&pb, TYPE_PNG) {
                    return Some(Payload::Image { png });
                }
                let tiff = pb.dataForType(&ns(TYPE_TIFF))?;
                tiff_to_png(&tiff).map(|png| Payload::Image { png })
            }
            ClipKind::Text => {
                let text = pb.stringForType(&ns(TYPE_TEXT))?.to_string();
                let rich = [(TYPE_RTF, RichFormat::Rtf), (TYPE_HTML, RichFormat::Html)]
                    .into_iter()
                    .find_map(|(ty, format)| data_for(&pb, ty).map(|data| Rich { format, data }))
                    .filter(|r| r.data.len() <= max_rich);
                Some(Payload::Text { text, rich })
            }
        }
    })
}

/// Current plain text, if any. A content read, like `read`.
pub fn read_text() -> Option<String> {
    autoreleasepool(|_| general().stringForType(&ns(TYPE_TEXT)).map(|s| s.to_string()))
}

fn bitmap_rep(data: &NSData) -> Option<Retained<AnyObject>> {
    unsafe { msg_send![class!(NSBitmapImageRep), imageRepWithData: data] }
}

fn tiff_to_png(tiff: &NSData) -> Option<Vec<u8>> {
    const NS_BITMAP_IMAGE_FILE_TYPE_PNG: usize = 4;
    let rep = bitmap_rep(tiff)?;
    let props = NSDictionary::<NSString, AnyObject>::new();
    let png: Option<Retained<NSData>> = unsafe {
        msg_send![&*rep, representationUsingType: NS_BITMAP_IMAGE_FILE_TYPE_PNG, properties: &*props]
    };
    png.map(|d| d.to_vec())
}

fn png_to_tiff(png: &NSData) -> Option<Retained<NSData>> {
    let rep = bitmap_rep(png)?;
    unsafe { msg_send![&*rep, TIFFRepresentation] }
}

/// Replace the pasteboard contents and return the resulting change count, so
/// the watcher can skip our own write. Writes are not gated by pasteboard
/// privacy. With `plain`, a text clip's rich payload is left out.
pub fn write(payload: &Payload, plain: bool) -> Result<i64> {
    autoreleasepool(|_| {
        let pb = general();
        pb.clearContents();
        let ok = match payload {
            Payload::Text { text, rich } => {
                let ok = pb.setString_forType(&ns(text), &ns(TYPE_TEXT));
                if let (Some(rich), false) = (rich, plain) {
                    let ty = match rich.format {
                        RichFormat::Rtf => TYPE_RTF,
                        RichFormat::Html => TYPE_HTML,
                    };
                    pb.setData_forType(Some(&NSData::with_bytes(&rich.data)), &ns(ty));
                }
                ok
            }
            Payload::Image { png } => {
                let data = NSData::with_bytes(png);
                let ok = pb.setData_forType(Some(&data), &ns(TYPE_PNG));
                if let Some(tiff) = png_to_tiff(&data) {
                    pb.setData_forType(Some(&tiff), &ns(TYPE_TIFF));
                }
                ok
            }
            Payload::Files { paths } => {
                let urls: Vec<Retained<NSURL>> = paths.iter().map(|p| NSURL::fileURLWithPath(&ns(p))).collect();
                let array = NSArray::from_retained_slice(&urls);
                unsafe { msg_send![&*pb, writeObjects: &*array] }
            }
        };
        if ok {
            Ok(pb.changeCount() as i64)
        } else {
            Err(Error::WriteFailed)
        }
    })
}

/// Write text flagged as transient/concealed, for tests and tooling that
/// must not be recorded by clipboard managers (including this one).
pub fn write_concealed_text(text: &str) -> Result<i64> {
    autoreleasepool(|_| {
        let pb = general();
        pb.clearContents();
        let ok = pb.setString_forType(&ns(text), &ns(TYPE_TEXT));
        pb.setString_forType(&ns(""), &ns("org.nspasteboard.ConcealedType"));
        if ok {
            Ok(pb.changeCount() as i64)
        } else {
            Err(Error::WriteFailed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_priority() {
        assert_eq!(classify(&[TYPE_TEXT, TYPE_FILE_URL, TYPE_TIFF]), Some(ClipKind::File));
        assert_eq!(classify(&[TYPE_HTML, TYPE_PNG]), Some(ClipKind::Image));
        assert_eq!(classify(&[TYPE_TEXT, TYPE_TIFF]), Some(ClipKind::Image));
        assert_eq!(classify(&[TYPE_TEXT, TYPE_RTF, TYPE_PNG]), Some(ClipKind::Text));
        assert_eq!(classify(&[TYPE_TEXT, TYPE_HTML]), Some(ClipKind::Text));
        assert_eq!(classify(&["com.example.custom"]), None);
        assert_eq!(classify::<&str>(&[]), None);
    }

    #[test]
    fn gate_states() {
        assert!(AccessBehavior::Unavailable.may_read());
        assert!(AccessBehavior::AlwaysAllow.may_read());
        assert!(!AccessBehavior::Default.may_read());
        assert!(!AccessBehavior::Ask.may_read());
        assert!(!AccessBehavior::AlwaysDeny.may_read());
    }
}
