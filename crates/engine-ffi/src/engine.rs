//! FFI functions for engine-level services.
//!
//! These are general-purpose services exposed to C# scripts:
//! logging, sound, timing, etc.

use std::ffi::CStr;

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

/// Log an info message from the scripting layer.
///
/// # Safety
///
/// `msg` must be a valid, null-terminated C string pointer or null.
#[no_mangle]
pub unsafe extern "C" fn ffi_log_info(msg: *const std::ffi::c_char) {
    if msg.is_null() {
        return;
    }
    let c_str = unsafe { CStr::from_ptr(msg) };
    if let Ok(s) = c_str.to_str() {
        tracing::info!(target: "script", "{s}");
    }
}

/// Log a warning from the scripting layer.
///
/// # Safety
///
/// `msg` must be a valid, null-terminated C string pointer or null.
#[no_mangle]
pub unsafe extern "C" fn ffi_log_warn(msg: *const std::ffi::c_char) {
    if msg.is_null() {
        return;
    }
    let c_str = unsafe { CStr::from_ptr(msg) };
    if let Ok(s) = c_str.to_str() {
        tracing::warn!(target: "script", "{s}");
    }
}

/// Log an error from the scripting layer.
///
/// # Safety
///
/// `msg` must be a valid, null-terminated C string pointer or null.
#[no_mangle]
pub unsafe extern "C" fn ffi_log_error(msg: *const std::ffi::c_char) {
    if msg.is_null() {
        return;
    }
    let c_str = unsafe { CStr::from_ptr(msg) };
    if let Ok(s) = c_str.to_str() {
        tracing::error!(target: "script", "{s}");
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Return a high-resolution timestamp in seconds (for scripting).
#[no_mangle]
pub extern "C" fn ffi_time_seconds() -> f64 {
    // Simple monotonic time
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn log_null_safe() {
        unsafe {
            ffi_log_info(std::ptr::null());
            ffi_log_warn(std::ptr::null());
            ffi_log_error(std::ptr::null());
        }
        // Should not panic
    }

    #[test]
    fn log_message() {
        let msg = CString::new("test message").unwrap();
        unsafe {
            ffi_log_info(msg.as_ptr());
        }
        // Smoke test — just verifies no crash
    }

    #[test]
    fn time_seconds_increases() {
        let t1 = ffi_time_seconds();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t2 = ffi_time_seconds();
        assert!(t2 > t1);
    }
}
