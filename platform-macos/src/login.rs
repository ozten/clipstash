//! Launch at login via `SMAppService.mainApp` (design section 4.1).
//! Only meaningful when running from a bundled, signed `.app`.

use crate::{Error, Result};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_foundation::NSError;

#[link(name = "ServiceManagement", kind = "framework")]
extern "C" {}

fn main_app_service() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    unsafe { msg_send![cls, mainAppService] }
}

pub fn is_enabled() -> bool {
    const SM_APP_SERVICE_STATUS_ENABLED: isize = 1;
    autoreleasepool(|_| match main_app_service() {
        Some(svc) => {
            let status: isize = unsafe { msg_send![&*svc, status] };
            status == SM_APP_SERVICE_STATUS_ENABLED
        }
        None => false,
    })
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    autoreleasepool(|_| {
        let svc = main_app_service().ok_or_else(|| Error::Other("SMAppService is unavailable".into()))?;
        let mut err: *mut NSError = std::ptr::null_mut();
        let ok: bool = unsafe {
            if enabled {
                msg_send![&*svc, registerAndReturnError: &mut err]
            } else {
                msg_send![&*svc, unregisterAndReturnError: &mut err]
            }
        };
        if ok {
            return Ok(());
        }
        let msg = unsafe { err.as_ref() }
            .map(|e| e.localizedDescription().to_string())
            .unwrap_or_else(|| "launch-at-login change failed".into());
        Err(Error::Other(msg))
    })
}
