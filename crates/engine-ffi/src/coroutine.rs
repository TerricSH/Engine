//! FFI bridge for the coroutine system.
//!
//! C# scripts create coroutines by returning `IEnumerator` objects.
//! Each `yield return` produces a value that the Rust `CoroutineSystem`
//! interprets as a `YieldInstruction`, determining when to resume.
//!
//! All functions dispatch through the runtime callback registry
//! ([`crate::registry`]), so they work immediately once the engine has
//! initialised the registry.

use crate::registry;
use crate::types::{FfiAsyncHandle, FfiCoroutineHandle, FfiYieldInstruction};

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
pub extern "C" fn ffi_coroutine_start(enumerator_ptr: *mut std::ffi::c_void) -> FfiCoroutineHandle {
    if !registry::is_initialized() {
        tracing::warn!("ffi_coroutine_start: engine not initialised");
        return FfiCoroutineHandle::INVALID;
    }
    (registry::get().coroutine_start)(enumerator_ptr)
}

/// Cancel a running coroutine by handle.
#[no_mangle]
pub extern "C" fn ffi_coroutine_cancel(handle: FfiCoroutineHandle) {
    if !registry::is_initialized() {
        tracing::warn!("ffi_coroutine_cancel: engine not initialised");
        return;
    }
    (registry::get().coroutine_cancel)(handle)
}

/// Advance a coroutine to its next `yield` and return the instruction.
///
/// Called by `CoroutineSystem::tick()` — **not** directly from C#.
#[no_mangle]
pub extern "C" fn ffi_coroutine_move_next(
    enumerator_ptr: *mut std::ffi::c_void,
    instruction_out: &mut FfiYieldInstruction,
) -> bool {
    if !registry::is_initialized() {
        return false;
    }
    (registry::get().coroutine_move_next)(enumerator_ptr, instruction_out)
}

/// Check whether an async handle has completed (used by WaitForAsync).
#[no_mangle]
pub extern "C" fn ffi_async_is_complete(handle: FfiAsyncHandle) -> bool {
    if !registry::is_initialized() {
        return false;
    }
    (registry::get().async_is_complete)(handle)
}

/// Evaluate a WaitUntil condition function.
#[no_mangle]
pub extern "C" fn ffi_condition_check(condition_id: u64) -> bool {
    if !registry::is_initialized() {
        return false;
    }
    (registry::get().condition_check)(condition_id)
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

    #[test]
    fn start_before_init_returns_invalid() {
        let handle = ffi_coroutine_start(std::ptr::null_mut());
        assert_eq!(handle, FfiCoroutineHandle::INVALID);
    }

    #[test]
    fn cancel_before_init_is_noop() {
        // Should not panic
        ffi_coroutine_cancel(FfiCoroutineHandle(1));
    }

    #[test]
    fn move_next_before_init_returns_false() {
        let mut instr = FfiYieldInstruction::NextFrame;
        assert!(!ffi_coroutine_move_next(std::ptr::null_mut(), &mut instr));
    }

    #[test]
    fn async_complete_before_init_returns_false() {
        assert!(!ffi_async_is_complete(FfiAsyncHandle(1)));
    }

    #[test]
    fn condition_check_before_init_returns_false() {
        assert!(!ffi_condition_check(42));
    }
}
