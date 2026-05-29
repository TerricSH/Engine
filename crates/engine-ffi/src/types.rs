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
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiComponentTypeId(pub u32);

impl FfiComponentTypeId {
    pub const INVALID: Self = Self(0);
}

/// Handle to a running coroutine, returned to C# when one is started.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiCoroutineHandle(pub u64);

impl FfiCoroutineHandle {
    pub const INVALID: Self = Self(0);
}

/// Handle to an async I/O operation.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FfiAsyncHandle(pub u64);

/// Result of a completed async operation.
#[repr(C)]
pub struct FfiAsyncResult {
    pub handle: FfiAsyncHandle,
    pub data: *mut u8,
    pub len: u32,
    pub succeeded: bool,
}

/// Enum of yield instructions a coroutine can return.
/// This is the FFI-safe representation — the C# side converts its
/// own yield objects to this when calling MoveNext.
#[repr(C)]
pub enum FfiYieldInstruction {
    /// Resume next frame.
    NextFrame,
    /// Resume after `seconds` have elapsed.
    WaitForSeconds(f32),
    /// Resume when the given async handle completes.
    WaitForAsync(FfiAsyncHandle),
    /// Resume when a condition function returns true (condition_fn_id).
    WaitUntil(u64),
}

/// Callback registration for async I/O completion.
pub type FfiAsyncCallback =
    extern "C" fn(handle: FfiAsyncHandle, data: *mut u8, len: u32, user_data: u64);

/// Condition check callback used by WaitUntil.
pub type FfiConditionFn = extern "C" fn(user_data: u64) -> bool;
