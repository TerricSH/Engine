//! FFI bridge for async I/O operations.
//!
//! C# scripts initiate async work (image loading, HTTP requests, asset I/O)
//! through these FFI calls. The actual work runs on Rust's thread pool,
//! and completion callbacks are dispatched on the main thread via
//! [`MAIN_THREAD_QUEUE`].

use std::collections::VecDeque;
use std::ffi::CStr;
use std::sync::{LazyLock, Mutex};

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
/// The actual I/O + decode runs on a thread-pool worker.
/// On completion, `callback` is queued to the main thread.
#[no_mangle]
pub extern "C" fn ffi_async_load_image(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    if url.is_null() {
        return FfiAsyncHandle(0);
    }
    let c_str = unsafe { CStr::from_ptr(url) };
    let url_str = match c_str.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return FfiAsyncHandle(0),
    };

    let handle = FfiAsyncHandle(next_async_id());

    // Spawn on global thread pool (basic std::thread for now)
    std::thread::spawn(move || {
        tracing::debug!(url = %url_str, handle = handle.0, "Async image load started");

        // TODO: actual download + decode logic
        // For now, simulate with an empty result after a short delay
        std::thread::sleep(std::time::Duration::from_millis(100));

        let result = Vec::new(); // decoded image bytes
        queue_main_thread_callback(handle, callback, result, user_data);
    });

    handle
}

/// Initiate an async HTTP GET request.
#[no_mangle]
pub extern "C" fn ffi_async_http_get(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    if url.is_null() {
        return FfiAsyncHandle(0);
    }
    let c_str = unsafe { CStr::from_ptr(url) };
    let url_str = match c_str.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return FfiAsyncHandle(0),
    };

    let handle = FfiAsyncHandle(next_async_id());

    std::thread::spawn(move || {
        tracing::debug!(url = %url_str, handle = handle.0, "Async HTTP GET started");

        // TODO: actual HTTP request via reqwest or similar
        std::thread::sleep(std::time::Duration::from_millis(50));

        let result = Vec::new(); // response bytes
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
        extern "C" fn test_cb(
            _handle: FfiAsyncHandle,
            _data: *mut u8,
            _len: u32,
            _user: u64,
        ) {
        }

        let handle = FfiAsyncHandle(42);
        queue_main_thread_callback(handle, test_cb, vec![1, 2, 3], 0);
        assert_eq!(pending_callback_count(), 1);

        dispatch_main_thread_callbacks();
        assert_eq!(pending_callback_count(), 0);
    }
}
