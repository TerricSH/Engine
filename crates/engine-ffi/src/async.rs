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

use std::collections::{HashSet, VecDeque};
use std::io::Read;
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
    /// `None` is an I/O/decode failure and is delivered as `(null, 0)`.
    /// `Some(Vec::new())` is a successful empty HTTP response.
    data: Option<Vec<u8>>,
    user_data: u64,
}

static MAIN_THREAD_QUEUE: LazyLock<Mutex<VecDeque<PendingCallback>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

const MAX_ASYNC_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct AsyncStates {
    pending: HashSet<FfiAsyncHandle>,
    /// Handles are allocated monotonically. Every issued handle at or below
    /// this watermark that is no longer pending is terminal, so completion
    /// never expires merely because many later requests finished.
    highest_issued: u64,
}

static ASYNC_STATES: LazyLock<Mutex<AsyncStates>> =
    LazyLock::new(|| Mutex::new(AsyncStates::default()));

/// Dispatch all queued async callbacks on the main thread.
/// Called once per frame by `EngineRuntime::tick()`.
pub fn dispatch_main_thread_callbacks() {
    // Never invoke foreign code while holding the queue lock. Managed
    // callbacks are allowed to start another request synchronously.
    let callbacks: Vec<_> = {
        let mut queue = MAIN_THREAD_QUEUE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.drain(..).collect()
    };

    for cb in callbacks {
        mark_complete(cb.handle);
        match cb.data {
            Some(data) => (cb.callback)(
                cb.handle,
                data.as_ptr() as *mut u8,
                data.len() as u32,
                cb.user_data,
            ),
            None => (cb.callback)(cb.handle, std::ptr::null_mut(), 0, cb.user_data),
        }
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
    queue_callback(handle, callback, Some(result_data), user_data);
}

/// Queue a failed operation for main-thread notification.
pub fn queue_main_thread_error(handle: FfiAsyncHandle, callback: FfiAsyncCallback, user_data: u64) {
    queue_callback(handle, callback, None, user_data);
}

fn queue_callback(
    handle: FfiAsyncHandle,
    callback: FfiAsyncCallback,
    data: Option<Vec<u8>>,
    user_data: u64,
) {
    // Registry-backed hosts may enqueue through this public bridge without
    // having called our local request allocator. Treat every queued callback
    // as pending until main-thread dispatch marks it terminal.
    mark_pending(handle);
    let mut queue = MAIN_THREAD_QUEUE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queue.push_back(PendingCallback {
        handle,
        callback,
        data,
        user_data,
    });
}

/// Return the number of pending main-thread callbacks.
pub fn pending_callback_count() -> usize {
    MAIN_THREAD_QUEUE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
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

    host_async_load_image(url, callback, user_data)
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

    host_async_http_get(url, callback, user_data)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Runtime-registry implementation used by the host `rlib` when C# calls the
/// separately loaded `cdylib`.
pub(crate) extern "C" fn host_async_load_image(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    start_async_request(url, callback, user_data, AsyncRequestKind::Image)
}

/// Runtime-registry implementation for raw HTTP GET requests.
pub(crate) extern "C" fn host_async_http_get(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
) -> FfiAsyncHandle {
    start_async_request(url, callback, user_data, AsyncRequestKind::Http)
}

pub(crate) extern "C" fn host_async_is_complete(handle: FfiAsyncHandle) -> bool {
    let states = ASYNC_STATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    handle.0 != 0 && handle.0 <= states.highest_issued && !states.pending.contains(&handle)
}

#[derive(Clone, Copy)]
enum AsyncRequestKind {
    Image,
    Http,
}

fn start_async_request(
    url: *const std::ffi::c_char,
    callback: FfiAsyncCallback,
    user_data: u64,
    kind: AsyncRequestKind,
) -> FfiAsyncHandle {
    if url.is_null() {
        return FfiAsyncHandle(0);
    }

    // SAFETY: Registry and exported FFI callers guarantee a valid
    // NUL-terminated string for the duration of this call.
    let url = unsafe { std::ffi::CStr::from_ptr(url) };
    let Ok(url) = url.to_str() else {
        return FfiAsyncHandle(0);
    };
    if url.trim().is_empty() {
        return FfiAsyncHandle(0);
    }
    let url = url.to_string();
    let handle = FfiAsyncHandle(next_async_id());
    mark_pending(handle);

    std::thread::spawn(move || {
        let result = match kind {
            AsyncRequestKind::Image => load_image_bytes(&url),
            AsyncRequestKind::Http => load_http_bytes(&url),
        };
        match result {
            Ok(bytes) => queue_main_thread_callback(handle, callback, bytes, user_data),
            Err(error) => {
                tracing::warn!(
                    request = %url,
                    handle = handle.0,
                    error = %error,
                    "asynchronous FFI request failed"
                );
                queue_main_thread_error(handle, callback, user_data);
            }
        }
    });

    handle
}

fn load_image_bytes(location: &str) -> Result<Vec<u8>, String> {
    let bytes = if location.starts_with("http://") || location.starts_with("https://") {
        load_http_bytes(location)?
    } else {
        let path = location.strip_prefix("file://").unwrap_or(location);
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("image metadata read failed: {error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_ASYNC_RESPONSE_BYTES {
            return Err("image must be a regular file no larger than 64 MiB".to_string());
        }
        std::fs::read(path).map_err(|error| format!("image read failed: {error}"))?
    };
    if bytes.is_empty() {
        return Err("image response was empty".to_string());
    }
    let format = image::guess_format(&bytes)
        .map_err(|error| format!("image format detection failed: {error}"))?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    reader.limits(image::Limits::default());
    reader
        .decode()
        .map_err(|error| format!("image decode failed: {error}"))?;
    Ok(bytes)
}

fn load_http_bytes(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("HTTP request requires an http:// or https:// URL".to_string());
    }
    let response = ureq::get(url)
        .call()
        .map_err(|error| format!("HTTP request failed: {error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_ASYNC_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("HTTP response read failed: {error}"))?;
    if bytes.len() as u64 > MAX_ASYNC_RESPONSE_BYTES {
        return Err("HTTP response exceeded 64 MiB".to_string());
    }
    Ok(bytes)
}

fn mark_pending(handle: FfiAsyncHandle) {
    let mut states = ASYNC_STATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    states.highest_issued = states.highest_issued.max(handle.0);
    states.pending.insert(handle);
}

fn mark_complete(handle: FfiAsyncHandle) {
    let mut states = ASYNC_STATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    states.highest_issued = states.highest_issued.max(handle.0);
    states.pending.remove(&handle);
}

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

    static ASYNC_TEST_LOCK: Mutex<()> = Mutex::new(());
    static CALLBACK_RESULT: Mutex<Option<(FfiAsyncHandle, bool, u32)>> = Mutex::new(None);

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        ASYNC_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    extern "C" fn record_callback(handle: FfiAsyncHandle, data: *mut u8, len: u32, _user: u64) {
        *CALLBACK_RESULT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((handle, !data.is_null(), len));
    }

    #[test]
    fn callback_queue_empty_initially() {
        let _guard = test_guard();
        dispatch_main_thread_callbacks();
        assert_eq!(pending_callback_count(), 0);
    }

    #[test]
    fn dispatch_empty_queue_no_panic() {
        let _guard = test_guard();
        dispatch_main_thread_callbacks(); // should not panic
    }

    #[test]
    fn queue_and_dispatch() {
        let _guard = test_guard();
        extern "C" fn test_cb(_handle: FfiAsyncHandle, _data: *mut u8, _len: u32, _user: u64) {}

        let handle = FfiAsyncHandle(42);
        queue_main_thread_callback(handle, test_cb, vec![1, 2, 3], 0);
        assert_eq!(pending_callback_count(), 1);

        dispatch_main_thread_callbacks();
        assert_eq!(pending_callback_count(), 0);
    }

    #[test]
    fn dispatch_marks_handle_complete_before_callback_returns() {
        let _guard = test_guard();
        let handle = FfiAsyncHandle(4242);
        queue_main_thread_callback(handle, record_callback, vec![1, 2, 3], 0);
        assert!(!host_async_is_complete(handle));

        dispatch_main_thread_callbacks();

        assert!(host_async_is_complete(handle));
        assert_eq!(*CALLBACK_RESULT.lock().unwrap(), Some((handle, true, 3)));
    }

    #[test]
    fn completed_handle_does_not_expire_after_many_later_completions() {
        let _guard = test_guard();
        let base = ASYNC_STATES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .highest_issued
            .saturating_add(1);
        let first = FfiAsyncHandle(base);
        for offset in 0..5_000u64 {
            let handle = FfiAsyncHandle(base + offset);
            mark_pending(handle);
            mark_complete(handle);
        }

        assert!(host_async_is_complete(first));
        assert!(!host_async_is_complete(FfiAsyncHandle(base + 5_000)));
    }

    #[test]
    fn local_image_load_returns_real_validated_bytes() {
        let _guard = test_guard();
        *CALLBACK_RESULT.lock().unwrap() = None;
        let path = std::env::temp_dir().join(format!(
            "engine-ffi-image-{}-{}.png",
            std::process::id(),
            next_async_id()
        ));
        image::RgbaImage::from_pixel(1, 1, image::Rgba([7, 8, 9, 255]))
            .save(&path)
            .unwrap();
        let path_string = path.to_string_lossy().into_owned();
        let path_c = std::ffi::CString::new(path_string).unwrap();

        let handle = host_async_load_image(path_c.as_ptr(), record_callback, 0);
        assert_ne!(handle, FfiAsyncHandle(0));
        for _ in 0..200 {
            if pending_callback_count() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(pending_callback_count(), 1);
        dispatch_main_thread_callbacks();

        let result = *CALLBACK_RESULT.lock().unwrap();
        assert!(matches!(result, Some((id, true, len)) if id == handle && len > 0));
        assert!(host_async_is_complete(handle));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn deferred_image_failure_uses_null_zero_callback() {
        let _guard = test_guard();
        *CALLBACK_RESULT.lock().unwrap() = None;
        let missing = std::env::temp_dir().join(format!(
            "engine-ffi-missing-{}-{}.png",
            std::process::id(),
            next_async_id()
        ));
        let missing_c = std::ffi::CString::new(missing.to_string_lossy().as_bytes()).unwrap();

        let handle = host_async_load_image(missing_c.as_ptr(), record_callback, 0);
        assert_ne!(handle, FfiAsyncHandle(0));
        for _ in 0..200 {
            if pending_callback_count() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(pending_callback_count(), 1);
        dispatch_main_thread_callbacks();

        assert_eq!(*CALLBACK_RESULT.lock().unwrap(), Some((handle, false, 0)));
        assert!(host_async_is_complete(handle));
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
