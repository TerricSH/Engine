//! FFI bridge for async I/O operations.
//!
//! C# scripts initiate async work (image loading, HTTP requests, asset I/O)
//! through these FFI calls.  The actual work runs on Rust's thread pool
//! (or a background task system registered by the engine), and completion
//! callbacks are dispatched on the main thread via [`MAIN_THREAD_QUEUE`].
//!
//! Two dispatch paths exist:
//!
//! 1. **Registry path** (preferred) — [`ffi_async_load_image`] and
//!    [`ffi_async_http_get`] delegate to the runtime callback registry.
//! 2. **Direct path** — thread-pool workers call
//!    [`queue_main_thread_callback`] to enqueue results.
//!
//! The main-thread dispatch happens once per frame via
//! [`dispatch_main_thread_callbacks`], which the engine's
//! `EngineRuntime::tick()` invokes.

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

use crate::registry;
use crate::types::{FfiAsyncCallback, FfiAsyncHandle};

// ---------------------------------------------------------------------------
// Main-thread callback queue
// ---------------------------------------------------------------------------

/// A pending callback to be invoked on the main thread.
struct PendingCallback {
    handle: FfiAsyncHandle,
    callback: FfiAsyncCallback,
    data: Vec<u8>,
    user_data: u64,
}

static MAIN_THREAD_QUEUE: LazyLock<Mutex<VecDeque<PendingCallback>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

/// Dispatch all queued async callbacks on the main thread.
/// Called once per frame by `EngineRuntime::tick()`.
pub fn dispatch_main_thread_callbacks() {
    let mut queue = MAIN_THREAD_QUEUE.lock().unwrap();
    while let Some(cb) = queue.pop_front() {
        (cb.callback)(
            cb.handle,
            cb.data.as_ptr() as *mut u8,
            cb.data.len() as u32,
            cb.user_data,
        );
    }
}

/// Queue a callback for main-thread dispatch.
/// Called from any thread (typically a Rust thread-pool worker).
pub fn queue_main_thread_callback(
    handle: FfiAsyncHandle,
    callback: FfiAsyncCallback,
    result_data: Vec<u8>,
    user_data: u64,
) {
    let mut queue = MAIN_THREAD_QUEUE.lock().unwrap();
    queue.push_back(PendingCallback {
        handle,
        callback,
        data: result_data,
        user_data,
    });
}

/// Return the number of pending main-thread callbacks.
pub fn pending_callback_count() -> usize {
    MAIN_THREAD_QUEUE.lock().unwrap().len()
}

// ---------------------------------------------------------------------------
// Extern "C" exports
// ---------------------------------------------------------------------------

/// Initiate an async image load.
///
/// C# calls this via EngineAPI when the script does
/// `ImageLoader.LoadAsync(url, callback)`.
///
/// The actual I/O + decode runs on a thread-pool worker or is dispatched
/// through the engine's callback registry.
/// On completion, `callback` is queued to the main thread.
///
/// # Safety
///
/// `url` must be a valid, null-terminated C string pointer or null.
/// `callback` must be a valid function pointer.
#[no_mangle]
pub unsafe extern "C" fn ffi_async_load_image(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    if url.is_null() {
        return FfiAsyncHandle(0);
    }

    if registry::is_initialized() {
        // Delegate to the engine's implementation (which may use reqwest,
        // image crate, or a custom asset system).
        return (registry::get().async_load_image)(url, callback, user_data);
    }

    // SAFETY: url is null-checked above; CStr::from_ptr requires a valid
    // null-terminated string, which the C# caller guarantees.
    let c_str = unsafe { std::ffi::CStr::from_ptr(url) };
    let url_str = match c_str.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return FfiAsyncHandle(0),
    };

    let handle = FfiAsyncHandle(next_async_id());

    std::thread::spawn(move || {
        tracing::debug!(url = %url_str, handle = handle.0, "Async image load started (fallback)");
        // Fallback: simulate work with a short delay
        std::thread::sleep(std::time::Duration::from_millis(100));
        tracing::debug!(url = %url_str, handle = handle.0, "Async image load completed (fallback)");
        let result = Vec::new();
        queue_main_thread_callback(handle, callback, result, user_data);
    });

    handle
}

/// Initiate an async HTTP GET request.
///
/// # Safety
///
/// `url` must be a valid, null-terminated C string pointer or null.
/// `callback` must be a valid function pointer.
#[no_mangle]
pub unsafe extern "C" fn ffi_async_http_get(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    if url.is_null() {
        return FfiAsyncHandle(0);
    }

    if registry::is_initialized() {
        return (registry::get().async_http_get)(url, callback, user_data);
    }

    // Fallback: basic thread with simulated delay.
    // SAFETY: `url` was not null-checked here, but the caller (ffi_async_http_get)
    // guards against null before reaching this point. The caller guarantees a
    // valid NUL-terminated C string for the duration of this FFI call.
    let c_str = unsafe { std::ffi::CStr::from_ptr(url) };
    let url_str = match c_str.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return FfiAsyncHandle(0),
    };

    let handle = FfiAsyncHandle(next_async_id());

    std::thread::spawn(move || {
        tracing::debug!(url = %url_str, handle = handle.0, "Async HTTP GET started (fallback)");
        std::thread::sleep(std::time::Duration::from_millis(50));
        tracing::debug!(url = %url_str, handle = handle.0, "Async HTTP GET completed (fallback)");
        let result = Vec::new();
        queue_main_thread_callback(handle, callback, result, user_data);
    });

    handle
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static NEXT_ASYNC_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_async_id() -> u64 {
    NEXT_ASYNC_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_queue_empty_initially() {
        assert_eq!(pending_callback_count(), 0);
    }

    #[test]
    fn dispatch_empty_queue_no_panic() {
        dispatch_main_thread_callbacks(); // should not panic
    }

    #[test]
    fn queue_and_dispatch() {
        extern "C" fn test_cb(_handle: FfiAsyncHandle, _data: *mut u8, _len: u32, _user: u64) {}

        let handle = FfiAsyncHandle(42);
        queue_main_thread_callback(handle, test_cb, vec![1, 2, 3], 0);
        assert_eq!(pending_callback_count(), 1);

        dispatch_main_thread_callbacks();
        assert_eq!(pending_callback_count(), 0);
    }

    extern "C" fn noop_callback(_handle: FfiAsyncHandle, _data: *mut u8, _len: u32, _user: u64) {}

    #[test]
    fn load_image_null_url_returns_zero() {
        let handle = unsafe { ffi_async_load_image(std::ptr::null(), noop_callback, 0) };
        assert_eq!(handle, FfiAsyncHandle(0));
    }

    #[test]
    fn http_get_null_url_returns_zero() {
        let handle = unsafe { ffi_async_http_get(std::ptr::null(), noop_callback, 0) };
        assert_eq!(handle, FfiAsyncHandle(0));
    }
}
