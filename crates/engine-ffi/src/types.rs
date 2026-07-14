//! FFI-safe types for the Rust ↔ C# bridge.
//!
//! All types here use `#[repr(C)]` layout so they can be passed directly
//! across the FFI boundary without serialization.

/// Opaque entity identifier passed between Rust and C#.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiEntityId {
    pub index: u32,
    pub generation: u32,
}

impl FfiEntityId {
    pub const INVALID: Self = Self {
        index: u32::MAX,
        generation: u32::MAX,
    };

    pub fn is_valid(&self) -> bool {
        self.index != u32::MAX
    }
}

/// Numeric ID for a Component type, resolved via runtime registry.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiComponentTypeId(pub u32);

impl FfiComponentTypeId {
    pub const INVALID: Self = Self(0);
}

/// Handle to a running coroutine, returned to C# when one is started.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiCoroutineHandle(pub u64);

impl FfiCoroutineHandle {
    pub const INVALID: Self = Self(0);
}

/// Handle to an async I/O operation.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiAsyncHandle(pub u64);

/// Result of a completed async operation.
#[repr(C)]
pub struct FfiAsyncResult {
    pub handle: FfiAsyncHandle,
    pub data: *mut u8,
    pub len: u32,
    /// C `_Bool`/managed bool width varies by binding; use an explicit byte.
    pub succeeded: u8,
}

/// Enum of yield instructions a coroutine can return.
/// This is the FFI-safe representation — the C# side converts its
/// own yield objects to this when calling MoveNext.
/// ABI version of [`FfiManagedCoroutineDescriptor`].
pub const FFI_MANAGED_COROUTINE_ABI_VERSION: u32 = 1;

/// `move_next` completed the enumerator normally.
pub const FFI_COROUTINE_MOVE_COMPLETED: u32 = 0;
/// `move_next` produced a valid [`FfiYieldInstruction`].
pub const FFI_COROUTINE_MOVE_YIELDED: u32 = 1;
/// `move_next` caught a managed exception or rejected the current value.
pub const FFI_COROUTINE_MOVE_FAILED: u32 = 2;

/// A managed readiness callback reports that the instruction is still waiting.
pub const FFI_COROUTINE_READY_WAITING: u32 = 0;
/// A managed readiness callback reports that the instruction may resume.
pub const FFI_COROUTINE_READY: u32 = 1;
/// A managed readiness callback caught an exception or rejected its token.
pub const FFI_COROUTINE_READY_FAILED: u32 = 2;

/// Resume on the next scheduler tick.
pub const FFI_YIELD_NEXT_FRAME: u32 = 0;
/// Resume after a number of seconds. `payload` contains `f32::to_bits()`.
pub const FFI_YIELD_WAIT_FOR_SECONDS: u32 = 1;
/// Resume when the async handle stored in `payload` is complete.
pub const FFI_YIELD_WAIT_FOR_ASYNC: u32 = 2;
/// Ask the managed readiness callback to evaluate a `WaitUntil` token.
pub const FFI_YIELD_WAIT_UNTIL: u32 = 3;
/// Ask the managed readiness callback to evaluate a `WaitForAll` token.
pub const FFI_YIELD_WAIT_FOR_ALL: u32 = 4;

/// Stable tagged representation of a managed coroutine yield value.
///
/// A Rust data-carrying enum must not cross the C ABI. This explicit tag and
/// payload layout is shared verbatim with `Engine.API`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfiYieldInstruction {
    pub tag: u32,
    pub reserved: u32,
    pub payload: u64,
}

impl FfiYieldInstruction {
    pub const fn next_frame() -> Self {
        Self {
            tag: FFI_YIELD_NEXT_FRAME,
            reserved: 0,
            payload: 0,
        }
    }

    pub fn wait_for_seconds(seconds: f32) -> Self {
        Self {
            tag: FFI_YIELD_WAIT_FOR_SECONDS,
            reserved: 0,
            payload: u64::from(seconds.to_bits()),
        }
    }

    pub const fn wait_for_async(handle: FfiAsyncHandle) -> Self {
        Self {
            tag: FFI_YIELD_WAIT_FOR_ASYNC,
            reserved: 0,
            payload: handle.0,
        }
    }

    pub const fn wait_until(token: u64) -> Self {
        Self {
            tag: FFI_YIELD_WAIT_UNTIL,
            reserved: 0,
            payload: token,
        }
    }

    pub const fn wait_for_all(token: u64) -> Self {
        Self {
            tag: FFI_YIELD_WAIT_FOR_ALL,
            reserved: 0,
            payload: token,
        }
    }
}

/// Advance a managed `IEnumerator` and write its next tagged yield value.
pub type FfiCoroutineMoveNextFn =
    extern "C" fn(context: *mut std::ffi::c_void, instruction_out: *mut FfiYieldInstruction) -> u32;

/// Evaluate a managed wait token. `delta_seconds` is the current frame delta.
pub type FfiCoroutineReadinessFn =
    extern "C" fn(context: *mut std::ffi::c_void, token: u64, delta_seconds: f32) -> u32;

/// Release the managed context after completion, cancellation, or shutdown.
pub type FfiCoroutineReleaseFn = extern "C" fn(context: *mut std::ffi::c_void);

/// Managed coroutine ownership descriptor copied by the native scheduler.
///
/// On a successful start the scheduler owns `context` and invokes `release`
/// exactly once. On a rejected start ownership remains with the caller.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiManagedCoroutineDescriptor {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context: *mut std::ffi::c_void,
    pub move_next: Option<FfiCoroutineMoveNextFn>,
    pub readiness: Option<FfiCoroutineReadinessFn>,
    pub release: Option<FfiCoroutineReleaseFn>,
}

/// Callback registration for async I/O completion.
pub type FfiAsyncCallback =
    extern "C" fn(handle: FfiAsyncHandle, data: *mut u8, len: u32, user_data: u64);

/// Condition check callback used by WaitUntil.
pub type FfiConditionFn = extern "C" fn(user_data: u64) -> bool;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_ffi_layouts_are_stable() {
        assert_eq!(std::mem::size_of::<FfiEntityId>(), 8);
        assert_eq!(std::mem::align_of::<FfiEntityId>(), 4);
        assert_eq!(std::mem::offset_of!(FfiEntityId, index), 0);
        assert_eq!(std::mem::offset_of!(FfiEntityId, generation), 4);

        assert_eq!(std::mem::size_of::<FfiComponentTypeId>(), 4);
        assert_eq!(std::mem::align_of::<FfiComponentTypeId>(), 4);
        assert_eq!(std::mem::size_of::<FfiCoroutineHandle>(), 8);
        assert_eq!(std::mem::size_of::<FfiAsyncHandle>(), 8);
        assert_eq!(std::mem::size_of::<FfiYieldInstruction>(), 16);
        assert_eq!(std::mem::align_of::<FfiYieldInstruction>(), 8);
        assert_eq!(std::mem::offset_of!(FfiYieldInstruction, tag), 0);
        assert_eq!(std::mem::offset_of!(FfiYieldInstruction, payload), 8);
        assert_eq!(
            std::mem::size_of::<FfiManagedCoroutineDescriptor>(),
            8 + 4 * std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn seconds_payload_roundtrips_exactly() {
        let instruction = FfiYieldInstruction::wait_for_seconds(1.25);
        assert_eq!(instruction.tag, FFI_YIELD_WAIT_FOR_SECONDS);
        assert_eq!(f32::from_bits(instruction.payload as u32), 1.25);
    }

    #[test]
    fn async_result_uses_explicit_one_byte_status() {
        assert_eq!(
            std::mem::size_of_val(
                &FfiAsyncResult {
                    handle: FfiAsyncHandle(1),
                    data: std::ptr::null_mut(),
                    len: 0,
                    succeeded: 1,
                }
                .succeeded
            ),
            1
        );
    }
}
