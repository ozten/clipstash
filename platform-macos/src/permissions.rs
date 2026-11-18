//! Accessibility trust and secure input (design section 10).

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

/// Is this process allowed to post events (direct paste)?
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Check trust and have macOS show its "grant access" prompt if missing.
pub fn request_accessibility() -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}

/// True while a password field (or a terminal's secure keyboard entry) has
/// secure input on. The paste engine and event tap stand down.
pub fn secure_input_active() -> bool {
    unsafe { IsSecureEventInputEnabled() }
}

/// The Settings list that holds the Accessibility grant, by OS version.
pub fn accessibility_pane_name(os_major: i64) -> &'static str {
    if os_major >= 27 {
        "Device Control and Data Access"
    } else {
        "Accessibility"
    }
}
