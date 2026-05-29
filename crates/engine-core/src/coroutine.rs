//! # Coroutine System
//!
//! Manages coroutines created by C# scripts (via ILRuntime). Each coroutine
//! is backed by a managed `IEnumerator` running on the ILRuntime side.
//!
//! # How it works
//!
//! 1. C# script starts a coroutine via `Coroutine.Start(enumerator)`.
//! 2. A `Coroutine` struct is created in Rust, holding an FFI handle to the
//!    managed `IEnumerator`.
//! 3. Each `yield return` produces a `YieldInstruction` that determines when
//!    the coroutine should resume.
//! 4. `CoroutineSystem::tick()` checks each coroutine's yield condition and
//!    advances it if the condition is met.
//! 5. When the `IEnumerator` signals completion, the coroutine is removed.

use std::time::Instant;

/// Handle for a running coroutine, returned to C# for cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CoroutineHandle(pub u64);

impl CoroutineHandle {
    pub const INVALID: Self = Self(0);
}

/// The yield instruction from the last `MoveNext` call on the IEnumerator.
#[derive(Clone, Debug)]
pub enum YieldInstruction {
    /// Resume next frame.
    NextFrame,
    /// Resume after the given number of seconds.
    WaitForSeconds(f32),
    /// Resume when the async operation completes.
    WaitForAsync(u64 /* async_handle_id */),
    /// Resume when a condition evaluates to true.
    WaitUntil(u64 /* condition_fn_id */),
}

/// Internal state of a running coroutine.
struct Coroutine {
    handle: CoroutineHandle,
    /// Opaque pointer to the ILRuntime IEnumerator object.
    enumerator_ptr: usize,
    /// The current yield instruction the coroutine is waiting on.
    instruction: Option<YieldInstruction>,
    /// When the coroutine started waiting (for WaitForSeconds).
    wait_start: Instant,
    /// Whether the coroutine has completed.
    is_done: bool,
}

/// Manages all active coroutines.
pub struct CoroutineSystem {
    coroutines: Vec<Coroutine>,
    next_handle: u64,
}

impl CoroutineSystem {
    /// Create a new, empty coroutine system.
    pub fn new() -> Self {
        Self {
            coroutines: Vec::new(),
            next_handle: 1,
        }
    }

    /// Start a new coroutine with the given IEnumerator handle.
    ///
    /// `enumerator_ptr` is an opaque handle to the ILRuntime IEnumerator
    /// object that the C# side passed through FFI.
    pub fn start(&mut self, enumerator_ptr: usize) -> CoroutineHandle {
        let handle = CoroutineHandle(self.next_handle);
        self.next_handle += 1;

        // Perform the initial MoveNext to get the first yield instruction
        let instruction = ffi_move_next(enumerator_ptr);
        let is_done = instruction.is_none();

        self.coroutines.push(Coroutine {
            handle,
            enumerator_ptr,
            instruction,
            wait_start: Instant::now(),
            is_done,
        });

        handle
    }

    /// Cancel a running coroutine.
    pub fn cancel(&mut self, handle: CoroutineHandle) {
        if let Some(c) = self.coroutines.iter_mut().find(|c| c.handle == handle) {
            c.is_done = true;
        }
    }

    /// Cancel all coroutines associated with a given IEnumerator ptr
    /// (used when a script instance is destroyed).
    pub fn cancel_by_enumerator(&mut self, enumerator_ptr: usize) {
        for c in &mut self.coroutines {
            if c.enumerator_ptr == enumerator_ptr {
                c.is_done = true;
            }
        }
    }

    /// Advance all waiting coroutines whose yield conditions are met.
    ///
    /// Should be called once per frame, before `script_engine.update()`.
    pub fn tick(&mut self) {
        let now = Instant::now();

        for c in &mut self.coroutines {
            if c.is_done {
                continue;
            }

            let can_advance = match &c.instruction {
                None => true,
                Some(YieldInstruction::NextFrame) => true,
                Some(YieldInstruction::WaitForSeconds(s)) => {
                    now.duration_since(c.wait_start).as_secs_f32() >= *s
                }
                Some(YieldInstruction::WaitForAsync(handle)) => {
                    ffi_async_is_complete(*handle)
                }
                Some(YieldInstruction::WaitUntil(cond_id)) => {
                    ffi_condition_check(*cond_id)
                }
            };

            if can_advance {
                // Call MoveNext on the IEnumerator
                let next = ffi_move_next(c.enumerator_ptr);
                match next {
                    Some(instr) => {
                        c.instruction = Some(instr);
                        c.wait_start = now;
                    }
                    None => {
                        // Coroutine completed
                        c.is_done = true;
                    }
                }
            }
        }

        // Clean up completed coroutines
        self.coroutines.retain(|c| !c.is_done);
    }

    /// Number of currently active coroutines.
    pub fn active_count(&self) -> usize {
        self.coroutines.len()
    }

    /// Remove all coroutines (e.g., on scene unload).
    pub fn clear(&mut self) {
        self.coroutines.clear();
    }
}

impl Default for CoroutineSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// FFI stubs — these call into the managed (ILRuntime) side
// ---------------------------------------------------------------------------

/// Call MoveNext on the IEnumerator and return the next YieldInstruction.
/// Returns None if the coroutine has completed.
fn ffi_move_next(_enumerator_ptr: usize) -> Option<YieldInstruction> {
    // TODO: Implement when ILRuntime FFI bridge is wired.
    // For the stub: always treat as NextFrame (continues each tick).
    Some(YieldInstruction::NextFrame)
}

/// Check whether an async handle is complete.
fn ffi_async_is_complete(_handle: u64) -> bool {
    // TODO: delegate to async system
    false
}

/// Evaluate a WaitUntil condition.
fn ffi_condition_check(_condition_id: u64) -> bool {
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
    fn coroutine_system_new_is_empty() {
        let system = CoroutineSystem::new();
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn coroutine_system_default_is_empty() {
        let system = CoroutineSystem::default();
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn start_coroutine_increases_count() {
        let mut system = CoroutineSystem::new();
        let handle = system.start(42); // 42 = mock enumerator ptr
        assert_ne!(handle, CoroutineHandle::INVALID);
        assert_eq!(system.active_count(), 1);
    }

    #[test]
    fn tick_without_coroutines() {
        let mut system = CoroutineSystem::new();
        system.tick(); // should not panic
    }

    #[test]
    fn cancel_removes_coroutine() {
        let mut system = CoroutineSystem::new();
        let handle = system.start(42);
        assert_eq!(system.active_count(), 1);
        system.cancel(handle);
        system.tick();
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn clear_removes_all() {
        let mut system = CoroutineSystem::new();
        system.start(42);
        system.start(43);
        assert_eq!(system.active_count(), 2);
        system.clear();
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn cancel_by_enumerator() {
        let mut system = CoroutineSystem::new();
        system.start(42);
        system.start(99);
        assert_eq!(system.active_count(), 2);
        system.cancel_by_enumerator(42);
        system.tick();
        assert_eq!(system.active_count(), 1);
    }

    #[test]
    fn multiple_coroutines_tick_independently() {
        let mut system = CoroutineSystem::new();
        system.start(1);
        system.start(2);
        system.start(3);
        assert_eq!(system.active_count(), 3);

        // Tick (stub always returns NextFrame, so all advance)
        system.tick();
        // In stub mode, MoveNext always returns Some(NextFrame)
        // so coroutines never complete. Count stays same.
        assert_eq!(system.active_count(), 3);

        // Cancel all
        system.clear();
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn handle_invalid_check() {
        assert_eq!(CoroutineHandle::INVALID, CoroutineHandle(0));
        assert_ne!(CoroutineHandle::INVALID, CoroutineHandle(1));
    }

    #[test]
    fn tick_updates_wait_start_on_resume() {
        let mut system = CoroutineSystem::new();
        let _handle = system.start(42);

        // Initially the coroutine has NextFrame, so tick advances it
        system.tick();

        // After tick, the coroutine advanced and got a new instruction
        // (also NextFrame from stub). Should still be alive.
        assert_eq!(system.active_count(), 1);
    }
}
