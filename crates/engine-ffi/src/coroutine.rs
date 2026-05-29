//! FFI bridge for the coroutine system.
//!
//! C# scripts create coroutines by returning `IEnumerator` objects.
//! Each `yield return` produces a value that the Rust `CoroutineSystem`
//! interprets as a `YieldInstruction`, determining when to resume.

use std::ffi::c_void;

use crate::types::{FfiAsyncHandle, FfiCoroutineHandle, FfiYieldInstruction};

// ---------------------------------------------------------------------------
// Global coroutine system pointer — set once by EngineRuntime
// ---------------------------------------------------------------------------

static mut COROUTINE_SYSTEM: *mut c_void = std::ptr::null_mut();

/// Set the global CoroutineSystem pointer (called once at startup).
///
/// # Safety
///
/// Must be called exactly once, before any other FFI coroutine function.
pub unsafe fn set_coroutine_system(ptr: *mut c_void) {
    COROUTINE_SYSTEM = ptr;
}

/// Get the global CoroutineSystem pointer.
fn system_ptr() -> *mut c_void {
    unsafe { COROUTINE_SYSTEM }
}

// ---------------------------------------------------------------------------
// Yield instruction interpretation helpers
// ---------------------------------------------------------------------------

/// Map an `FfiYieldInstruction` to a human-readable description (for tracing).
pub fn yield_instruction_name(instr: &FfiYieldInstruction) -> &'static str {
    match instr {
        FfiYieldInstruction::NextFrame => "NextFrame",
        FfiYieldInstruction::WaitForSeconds(_) => "WaitForSeconds",
        FfiYieldInstruction::WaitForAsync(_) => "WaitForAsync",
        FfiYieldInstruction::WaitUntil(_) => "WaitUntil",
    }
}

// ---------------------------------------------------------------------------
// Extern "C" exports
// ---------------------------------------------------------------------------

/// Start a coroutine from C#.
///
/// `enumerator_ptr` is an opaque handle to the ILRuntime `IEnumerator` object.
/// Returns a handle that can be used to cancel the coroutine.
#[no_mangle]
pub extern "C" fn ffi_coroutine_start(enumerator_ptr: *mut c_void) -> FfiCoroutineHandle {
    let _ = enumerator_ptr;
    // TODO: delegate to CoroutineSystem when integrated
    tracing::warn!(
        "ffi_coroutine_start: CoroutineSystem not yet wired (ptr={:?})",
        enumerator_ptr
    );
    FfiCoroutineHandle::INVALID
}

/// Cancel a running coroutine by handle.
#[no_mangle]
pub extern "C" fn ffi_coroutine_cancel(handle: FfiCoroutineHandle) {
    let _ = handle;
    tracing::warn!("ffi_coroutine_cancel: not yet implemented");
}

/// Advance a coroutine to its next `yield` and return the instruction.
///
/// Called by `CoroutineSystem::tick()` — **not** directly from C#.
#[no_mangle]
pub extern "C" fn ffi_coroutine_move_next(
    enumerator_ptr: *mut c_void,
    instruction_out: *mut FfiYieldInstruction,
) -> bool {
    let _ = (enumerator_ptr, instruction_out);
    // TODO: implement when ILRuntime binding is live
    false
}

/// Check whether an async handle has completed (used by WaitForAsync).
#[no_mangle]
pub extern "C" fn ffi_async_is_complete(handle: FfiAsyncHandle) -> bool {
    let _ = handle;
    // TODO: delegate to async system
    false
}

/// Evaluate a WaitUntil condition function.
#[no_mangle]
pub extern "C" fn ffi_condition_check(condition_id: u64) -> bool {
    let _ = condition_id;
    // TODO: delegate to registered condition table
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yield_instruction_names() {
        assert_eq!(yield_instruction_name(&FfiYieldInstruction::NextFrame), "NextFrame");
        assert_eq!(
            yield_instruction_name(&FfiYieldInstruction::WaitForSeconds(1.0)),
            "WaitForSeconds"
        );
        assert_eq!(
            yield_instruction_name(&FfiYieldInstruction::WaitForAsync(FfiAsyncHandle(42))),
            "WaitForAsync"
        );
        assert_eq!(
            yield_instruction_name(&FfiYieldInstruction::WaitUntil(7)),
            "WaitUntil"
        );
    }

    #[test]
    fn invalid_handle_checks() {
        assert_eq!(FfiCoroutineHandle::INVALID, FfiCoroutineHandle(0));
        assert_eq!(FfiAsyncHandle(0), FfiAsyncHandle(0));
    }
}
