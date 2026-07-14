//! Managed coroutine scheduler and its stable C ABI.
//!
//! The scheduler owns a managed context after a successful start. Foreign
//! callbacks are always invoked without holding the scheduler mutex, so a
//! coroutine may start or stop coroutines re-entrantly. Completion,
//! cancellation, callback failure, and runtime shutdown all converge on the
//! same RAII release path.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::registry;
use crate::types::{
    FfiAsyncHandle, FfiCoroutineHandle, FfiCoroutineMoveNextFn, FfiCoroutineReadinessFn,
    FfiCoroutineReleaseFn, FfiManagedCoroutineDescriptor, FfiYieldInstruction,
    FFI_COROUTINE_MOVE_COMPLETED, FFI_COROUTINE_MOVE_FAILED, FFI_COROUTINE_MOVE_YIELDED,
    FFI_COROUTINE_READY, FFI_COROUTINE_READY_FAILED, FFI_COROUTINE_READY_WAITING,
    FFI_MANAGED_COROUTINE_ABI_VERSION, FFI_YIELD_NEXT_FRAME, FFI_YIELD_WAIT_FOR_ALL,
    FFI_YIELD_WAIT_FOR_ASYNC, FFI_YIELD_WAIT_FOR_SECONDS, FFI_YIELD_WAIT_UNTIL,
};

struct ManagedContext {
    context: usize,
    move_next: FfiCoroutineMoveNextFn,
    readiness: FfiCoroutineReadinessFn,
    release: Option<FfiCoroutineReleaseFn>,
    epoch: u64,
}

impl ManagedContext {
    fn pointer(&self) -> *mut std::ffi::c_void {
        self.context as *mut std::ffi::c_void
    }
}

impl Drop for ManagedContext {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let pointer = self.pointer();
            with_callback_epoch(self.epoch, || release(pointer));
        }
    }
}

enum Waiting {
    Start,
    NextFrame,
    Seconds(f32),
    Async(FfiAsyncHandle),
    Managed(u64),
}

struct CoroutineEntry {
    managed: ManagedContext,
    waiting: Waiting,
}

struct SchedulerState {
    entries: HashMap<FfiCoroutineHandle, CoroutineEntry>,
    in_flight: HashSet<FfiCoroutineHandle>,
    cancelled: HashSet<FfiCoroutineHandle>,
    next_handle: u64,
    epoch: u64,
    clear_depth: usize,
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            in_flight: HashSet::new(),
            cancelled: HashSet::new(),
            next_handle: 1,
            epoch: 1,
            clear_depth: 0,
        }
    }
}

static SCHEDULER: LazyLock<Mutex<SchedulerState>> =
    LazyLock::new(|| Mutex::new(SchedulerState::default()));
static TICK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

std::thread_local! {
    /// Epoch of the managed callback currently executing on this thread.
    /// A callback from an entry invalidated by `clear` may not resurrect work
    /// into the new runtime/scene epoch.
    static CALLBACK_EPOCH: Cell<Option<u64>> = const { Cell::new(None) };
}

fn with_callback_epoch<R>(epoch: u64, callback: impl FnOnce() -> R) -> R {
    CALLBACK_EPOCH.with(|cell| {
        struct RestoreEpoch<'a> {
            cell: &'a Cell<Option<u64>>,
            previous: Option<u64>,
        }

        impl Drop for RestoreEpoch<'_> {
            fn drop(&mut self) {
                self.cell.set(self.previous);
            }
        }

        let previous = cell.replace(Some(epoch));
        let _restore = RestoreEpoch { cell, previous };
        callback()
    })
}

struct TickGuard;

impl Drop for TickGuard {
    fn drop(&mut self) {
        TICK_IN_PROGRESS.store(false, Ordering::Release);
    }
}

fn lock_scheduler() -> std::sync::MutexGuard<'static, SchedulerState> {
    SCHEDULER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn allocate_handle(state: &mut SchedulerState) -> FfiCoroutineHandle {
    loop {
        let raw = state.next_handle.max(1);
        state.next_handle = raw.checked_add(1).unwrap_or(1);
        let handle = FfiCoroutineHandle(raw);
        if !state.entries.contains_key(&handle) && !state.in_flight.contains(&handle) {
            return handle;
        }
    }
}

/// Validate and schedule a copied managed coroutine descriptor.
///
/// Ownership of `descriptor.context` transfers only when a non-zero handle is
/// returned. The caller retains ownership on every validation failure.
///
/// # Safety
///
/// A non-null `descriptor` must point to a readable descriptor for this call.
pub unsafe fn schedule_managed_coroutine(
    descriptor: *const FfiManagedCoroutineDescriptor,
) -> FfiCoroutineHandle {
    if descriptor.is_null() {
        return FfiCoroutineHandle::INVALID;
    }

    // SAFETY: The C ABI requires `descriptor` to remain readable for this
    // call. It is copied before the function returns.
    let descriptor = unsafe { *descriptor };
    let expected_size = match u32::try_from(std::mem::size_of::<FfiManagedCoroutineDescriptor>()) {
        Ok(size) => size,
        Err(_) => return FfiCoroutineHandle::INVALID,
    };
    if descriptor.abi_version != FFI_MANAGED_COROUTINE_ABI_VERSION
        || descriptor.struct_size != expected_size
        || descriptor.context.is_null()
    {
        return FfiCoroutineHandle::INVALID;
    }
    let (Some(move_next), Some(readiness), Some(release)) = (
        descriptor.move_next,
        descriptor.readiness,
        descriptor.release,
    ) else {
        return FfiCoroutineHandle::INVALID;
    };

    let callback_epoch = CALLBACK_EPOCH.with(Cell::get);
    let mut state = lock_scheduler();
    if state.clear_depth != 0 || callback_epoch.is_some_and(|epoch| epoch != state.epoch) {
        return FfiCoroutineHandle::INVALID;
    }
    let handle = allocate_handle(&mut state);
    let epoch = state.epoch;
    state.entries.insert(
        handle,
        CoroutineEntry {
            managed: ManagedContext {
                context: descriptor.context as usize,
                move_next,
                readiness,
                release: Some(release),
                epoch,
            },
            waiting: Waiting::Start,
        },
    );
    handle
}

/// Cancel a managed coroutine. Repeated cancellation is harmless.
pub fn cancel_managed_coroutine(handle: FfiCoroutineHandle) {
    if handle == FfiCoroutineHandle::INVALID {
        return;
    }
    let removed = {
        let mut state = lock_scheduler();
        if state.in_flight.contains(&handle) {
            state.cancelled.insert(handle);
            None
        } else {
            state.entries.remove(&handle)
        }
    };
    // Dropping invokes managed release; never do that while holding the lock.
    drop(removed);
}

/// Advance every managed coroutine by at most one `MoveNext` call.
pub fn tick_managed_coroutines(delta_seconds: f32) {
    if TICK_IN_PROGRESS
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        tracing::warn!("ignored a concurrent or re-entrant managed coroutine tick");
        return;
    }
    let _tick_guard = TickGuard;

    let delta_seconds = if delta_seconds.is_finite() && delta_seconds > 0.0 {
        delta_seconds
    } else {
        0.0
    };
    let handles = {
        let state = lock_scheduler();
        state.entries.keys().copied().collect::<Vec<_>>()
    };

    for handle in handles {
        let Some(mut entry) = ({
            let mut state = lock_scheduler();
            let entry = state.entries.remove(&handle);
            if entry.is_some() {
                state.in_flight.insert(handle);
            }
            entry
        }) else {
            continue;
        };

        let keep = advance(&mut entry, delta_seconds);
        let mut entry = Some(entry);
        let removed = {
            let mut state = lock_scheduler();
            state.in_flight.remove(&handle);
            let cancelled = state.cancelled.remove(&handle);
            if keep && !cancelled {
                state
                    .entries
                    .insert(handle, entry.take().expect("entry is available"));
            }
            entry
        };
        // Completion/cancellation invokes release outside the scheduler lock.
        drop(removed);
    }
}

fn advance(entry: &mut CoroutineEntry, delta_seconds: f32) -> bool {
    let ready = match &mut entry.waiting {
        Waiting::Start | Waiting::NextFrame => true,
        Waiting::Seconds(remaining) => {
            *remaining -= delta_seconds;
            *remaining <= 0.0
        }
        Waiting::Async(handle) => crate::r#async::host_async_is_complete(*handle),
        Waiting::Managed(token) => {
            let status = with_callback_epoch(entry.managed.epoch, || {
                (entry.managed.readiness)(entry.managed.pointer(), *token, delta_seconds)
            });
            match status {
                FFI_COROUTINE_READY_WAITING => false,
                FFI_COROUTINE_READY => true,
                FFI_COROUTINE_READY_FAILED => {
                    tracing::error!("managed coroutine readiness callback failed");
                    return false;
                }
                status => {
                    tracing::error!(
                        status,
                        "managed coroutine readiness returned an invalid status"
                    );
                    return false;
                }
            }
        }
    };
    if !ready {
        return true;
    }

    let mut instruction = FfiYieldInstruction::next_frame();
    let status = with_callback_epoch(entry.managed.epoch, || {
        (entry.managed.move_next)(entry.managed.pointer(), &mut instruction)
    });
    match status {
        FFI_COROUTINE_MOVE_COMPLETED => false,
        FFI_COROUTINE_MOVE_FAILED => {
            tracing::error!("managed coroutine MoveNext callback failed");
            false
        }
        FFI_COROUTINE_MOVE_YIELDED => match parse_instruction(instruction) {
            Some(waiting) => {
                entry.waiting = waiting;
                true
            }
            None => {
                tracing::error!(
                    tag = instruction.tag,
                    payload = instruction.payload,
                    "managed coroutine returned an invalid yield instruction"
                );
                false
            }
        },
        status => {
            tracing::error!(
                status,
                "managed coroutine MoveNext returned an invalid status"
            );
            false
        }
    }
}

fn parse_instruction(instruction: FfiYieldInstruction) -> Option<Waiting> {
    if instruction.reserved != 0 {
        return None;
    }
    match instruction.tag {
        FFI_YIELD_NEXT_FRAME if instruction.payload == 0 => Some(Waiting::NextFrame),
        FFI_YIELD_WAIT_FOR_SECONDS if instruction.payload >> 32 == 0 => {
            let seconds = f32::from_bits(instruction.payload as u32);
            (seconds.is_finite() && seconds >= 0.0).then_some(Waiting::Seconds(seconds))
        }
        FFI_YIELD_WAIT_FOR_ASYNC if instruction.payload != 0 => {
            Some(Waiting::Async(FfiAsyncHandle(instruction.payload)))
        }
        FFI_YIELD_WAIT_UNTIL | FFI_YIELD_WAIT_FOR_ALL if instruction.payload != 0 => {
            Some(Waiting::Managed(instruction.payload))
        }
        _ => None,
    }
}

/// Release every queued coroutine and mark callbacks currently in flight for
/// release when they return.
pub fn clear_managed_coroutines() {
    let removed = {
        let mut state = lock_scheduler();
        state.epoch = state.epoch.wrapping_add(1).max(1);
        state.clear_depth = state.clear_depth.saturating_add(1);
        let in_flight = state.in_flight.iter().copied().collect::<Vec<_>>();
        state.cancelled.extend(in_flight);
        state
            .entries
            .drain()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
    };
    drop(removed);
    let mut state = lock_scheduler();
    state.clear_depth = state.clear_depth.saturating_sub(1);
}

pub fn active_managed_coroutine_count() -> usize {
    let state = lock_scheduler();
    state.entries.len() + state.in_flight.len()
}

/// Start a coroutine through the host registry. Before registry installation,
/// use the local scheduler so the standalone native ABI can be smoke-tested.
///
/// # Safety
///
/// A non-null `descriptor` must point to a readable descriptor for this call.
#[no_mangle]
pub unsafe extern "C" fn ffi_coroutine_start(
    descriptor: *const FfiManagedCoroutineDescriptor,
) -> FfiCoroutineHandle {
    if registry::is_initialized() {
        // SAFETY: Forwarded unchanged under this function's pointer contract.
        unsafe { (registry::get().coroutine_start)(descriptor) }
    } else {
        // SAFETY: Forwarded unchanged under this function's pointer contract.
        unsafe { schedule_managed_coroutine(descriptor) }
    }
}

#[no_mangle]
pub extern "C" fn ffi_coroutine_cancel(handle: FfiCoroutineHandle) {
    if registry::is_initialized() {
        (registry::get().coroutine_cancel)(handle);
    } else {
        cancel_managed_coroutine(handle);
    }
}

/// Standalone/local scheduler entry point used by ABI validation tools.
#[no_mangle]
pub extern "C" fn ffi_coroutine_tick(delta_seconds: f32) {
    tick_managed_coroutines(delta_seconds);
}

#[no_mangle]
pub extern "C" fn ffi_coroutine_active_count() -> u32 {
    active_managed_coroutine_count()
        .try_into()
        .unwrap_or(u32::MAX)
}

#[no_mangle]
pub extern "C" fn ffi_coroutine_clear() {
    clear_managed_coroutines();
}

/// Check whether an async handle has completed (used by managed diagnostics).
#[no_mangle]
pub extern "C" fn ffi_async_is_complete(handle: FfiAsyncHandle) -> bool {
    if registry::is_initialized() {
        (registry::get().async_is_complete)(handle)
    } else {
        crate::r#async::host_async_is_complete(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static REENTER_TICK: AtomicBool = AtomicBool::new(false);
    static OUTER_RELEASES: AtomicUsize = AtomicUsize::new(0);
    static NESTED_RELEASES: AtomicUsize = AtomicUsize::new(0);
    static RELEASE_SPAWN_HANDLE: AtomicU64 = AtomicU64::new(u64::MAX);

    struct TestContext {
        moves: Mutex<VecDeque<(u32, FfiYieldInstruction)>>,
        ready: AtomicU32,
        self_handle: AtomicU64,
        cancel_self: bool,
        move_count: Arc<AtomicUsize>,
        release_count: Arc<AtomicUsize>,
    }

    extern "C" fn move_next(context: *mut std::ffi::c_void, out: *mut FfiYieldInstruction) -> u32 {
        // SAFETY: Test descriptors keep this boxed context alive until release.
        let context = unsafe { &*(context as *const TestContext) };
        context.move_count.fetch_add(1, Ordering::SeqCst);
        // This lock acquisition proves scheduler callbacks run without its lock.
        let _ = active_managed_coroutine_count();
        if REENTER_TICK.swap(false, Ordering::SeqCst) {
            tick_managed_coroutines(0.001);
        }
        if context.cancel_self {
            cancel_managed_coroutine(FfiCoroutineHandle(
                context.self_handle.load(Ordering::SeqCst),
            ));
        }
        let (status, instruction) = context.moves.lock().unwrap().pop_front().unwrap_or((
            FFI_COROUTINE_MOVE_COMPLETED,
            FfiYieldInstruction::next_frame(),
        ));
        if !out.is_null() {
            // SAFETY: The scheduler provides a writable instruction pointer.
            unsafe { *out = instruction };
        }
        status
    }

    extern "C" fn readiness(
        context: *mut std::ffi::c_void,
        _token: u64,
        _delta_seconds: f32,
    ) -> u32 {
        // SAFETY: Test descriptors keep this boxed context alive until release.
        unsafe { &*(context as *const TestContext) }
            .ready
            .load(Ordering::SeqCst)
    }

    extern "C" fn release(context: *mut std::ffi::c_void) {
        // SAFETY: Native ownership transfers exactly once on successful start.
        let context = unsafe { Box::from_raw(context as *mut TestContext) };
        context.release_count.fetch_add(1, Ordering::SeqCst);
        let _ = active_managed_coroutine_count();
    }

    extern "C" fn complete_immediately(
        _context: *mut std::ffi::c_void,
        _out: *mut FfiYieldInstruction,
    ) -> u32 {
        FFI_COROUTINE_MOVE_COMPLETED
    }

    extern "C" fn clear_then_complete(
        _context: *mut std::ffi::c_void,
        _out: *mut FfiYieldInstruction,
    ) -> u32 {
        clear_managed_coroutines();
        FFI_COROUTINE_MOVE_COMPLETED
    }

    extern "C" fn always_ready(
        _context: *mut std::ffi::c_void,
        _token: u64,
        _delta_seconds: f32,
    ) -> u32 {
        FFI_COROUTINE_READY
    }

    extern "C" fn release_nested(context: *mut std::ffi::c_void) {
        // SAFETY: The matching test descriptor allocates one boxed byte.
        drop(unsafe { Box::from_raw(context.cast::<u8>()) });
        NESTED_RELEASES.fetch_add(1, Ordering::SeqCst);
    }

    extern "C" fn release_and_attempt_restart(context: *mut std::ffi::c_void) {
        // SAFETY: The matching test descriptor allocates one boxed byte.
        drop(unsafe { Box::from_raw(context.cast::<u8>()) });
        OUTER_RELEASES.fetch_add(1, Ordering::SeqCst);

        let nested_context = Box::into_raw(Box::new(2u8));
        let nested = FfiManagedCoroutineDescriptor {
            abi_version: FFI_MANAGED_COROUTINE_ABI_VERSION,
            struct_size: std::mem::size_of::<FfiManagedCoroutineDescriptor>() as u32,
            context: nested_context.cast(),
            move_next: Some(complete_immediately),
            readiness: Some(always_ready),
            release: Some(release_nested),
        };
        // SAFETY: `nested` is readable for this call.
        let handle = unsafe { schedule_managed_coroutine(&nested) };
        RELEASE_SPAWN_HANDLE.store(handle.0, Ordering::SeqCst);
        if handle == FfiCoroutineHandle::INVALID {
            // Rejected starts retain caller ownership.
            release_nested(nested_context.cast());
        }
    }

    fn restart_on_release_descriptor(
        move_next: FfiCoroutineMoveNextFn,
    ) -> FfiManagedCoroutineDescriptor {
        FfiManagedCoroutineDescriptor {
            abi_version: FFI_MANAGED_COROUTINE_ABI_VERSION,
            struct_size: std::mem::size_of::<FfiManagedCoroutineDescriptor>() as u32,
            context: Box::into_raw(Box::new(1u8)).cast(),
            move_next: Some(move_next),
            readiness: Some(always_ready),
            release: Some(release_and_attempt_restart),
        }
    }

    fn reset_restart_counters() {
        OUTER_RELEASES.store(0, Ordering::SeqCst);
        NESTED_RELEASES.store(0, Ordering::SeqCst);
        RELEASE_SPAWN_HANDLE.store(u64::MAX, Ordering::SeqCst);
    }

    fn descriptor(
        moves: Vec<(u32, FfiYieldInstruction)>,
        ready: u32,
        cancel_self: bool,
    ) -> (
        FfiManagedCoroutineDescriptor,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        *mut TestContext,
    ) {
        let move_count = Arc::new(AtomicUsize::new(0));
        let release_count = Arc::new(AtomicUsize::new(0));
        let context = Box::into_raw(Box::new(TestContext {
            moves: Mutex::new(moves.into()),
            ready: AtomicU32::new(ready),
            self_handle: AtomicU64::new(0),
            cancel_self,
            move_count: Arc::clone(&move_count),
            release_count: Arc::clone(&release_count),
        }));
        (
            FfiManagedCoroutineDescriptor {
                abi_version: FFI_MANAGED_COROUTINE_ABI_VERSION,
                struct_size: std::mem::size_of::<FfiManagedCoroutineDescriptor>() as u32,
                context: context.cast(),
                move_next: Some(move_next),
                readiness: Some(readiness),
                release: Some(release),
            },
            move_count,
            release_count,
            context,
        )
    }

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_managed_coroutines();
        guard
    }

    #[test]
    fn natural_completion_releases_exactly_once() {
        let _guard = test_guard();
        let (descriptor, moves, releases, _) = descriptor(
            vec![
                (
                    FFI_COROUTINE_MOVE_YIELDED,
                    FfiYieldInstruction::next_frame(),
                ),
                (
                    FFI_COROUTINE_MOVE_COMPLETED,
                    FfiYieldInstruction::next_frame(),
                ),
            ],
            FFI_COROUTINE_READY,
            false,
        );
        // SAFETY: `descriptor` is readable for this call.
        let handle = unsafe { schedule_managed_coroutine(&descriptor) };
        assert_ne!(handle, FfiCoroutineHandle::INVALID);
        tick_managed_coroutines(0.016);
        assert_eq!(moves.load(Ordering::SeqCst), 1);
        tick_managed_coroutines(0.016);
        assert_eq!(active_managed_coroutine_count(), 0);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
        cancel_managed_coroutine(handle);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_clear_failure_and_reentrant_cancel_release_once() {
        let _guard = test_guard();
        for action in 0..4 {
            let status = if action == 2 {
                FFI_COROUTINE_MOVE_FAILED
            } else {
                FFI_COROUTINE_MOVE_YIELDED
            };
            let (descriptor, _, releases, context) = descriptor(
                vec![(status, FfiYieldInstruction::next_frame())],
                FFI_COROUTINE_READY,
                action == 3,
            );
            // SAFETY: `descriptor` is readable for this call.
            let handle = unsafe { schedule_managed_coroutine(&descriptor) };
            // SAFETY: The scheduler owns the live test context at this point.
            unsafe { &*context }
                .self_handle
                .store(handle.0, Ordering::SeqCst);
            match action {
                0 => cancel_managed_coroutine(handle),
                1 => clear_managed_coroutines(),
                _ => tick_managed_coroutines(0.016),
            }
            assert_eq!(releases.load(Ordering::SeqCst), 1);
            cancel_managed_coroutine(handle);
            clear_managed_coroutines();
            assert_eq!(releases.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn rejected_start_does_not_take_context_ownership() {
        let _guard = test_guard();
        let (mut descriptor, _, releases, context) = descriptor(vec![], FFI_COROUTINE_READY, false);
        descriptor.abi_version += 1;
        assert_eq!(
            // SAFETY: `descriptor` is readable for this call.
            unsafe { schedule_managed_coroutine(&descriptor) },
            FfiCoroutineHandle::INVALID
        );
        assert_eq!(releases.load(Ordering::SeqCst), 0);
        release(context.cast());
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn seconds_and_managed_readiness_gate_move_next() {
        let _guard = test_guard();
        let (descriptor, moves, releases, context) = descriptor(
            vec![
                (
                    FFI_COROUTINE_MOVE_YIELDED,
                    FfiYieldInstruction::wait_for_seconds(0.5),
                ),
                (
                    FFI_COROUTINE_MOVE_YIELDED,
                    FfiYieldInstruction::wait_until(7),
                ),
                (
                    FFI_COROUTINE_MOVE_COMPLETED,
                    FfiYieldInstruction::next_frame(),
                ),
            ],
            FFI_COROUTINE_READY_WAITING,
            false,
        );
        // SAFETY: `descriptor` is readable for this call.
        unsafe { schedule_managed_coroutine(&descriptor) };
        tick_managed_coroutines(0.1);
        tick_managed_coroutines(0.2);
        assert_eq!(moves.load(Ordering::SeqCst), 1);
        tick_managed_coroutines(0.3);
        assert_eq!(moves.load(Ordering::SeqCst), 2);
        tick_managed_coroutines(0.1);
        assert_eq!(moves.load(Ordering::SeqCst), 2);
        // SAFETY: Context is still owned by the active scheduler entry.
        unsafe { &*context }
            .ready
            .store(FFI_COROUTINE_READY, Ordering::SeqCst);
        tick_managed_coroutines(0.1);
        assert_eq!(moves.load(Ordering::SeqCst), 3);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn wait_for_async_resumes_after_main_thread_dispatch() {
        let _guard = test_guard();
        extern "C" fn callback(
            _handle: FfiAsyncHandle,
            _data: *mut u8,
            _len: u32,
            _user_data: u64,
        ) {
        }
        let handle = FfiAsyncHandle(0xCAFE);
        let (descriptor, moves, releases, _) = descriptor(
            vec![
                (
                    FFI_COROUTINE_MOVE_YIELDED,
                    FfiYieldInstruction::wait_for_async(handle),
                ),
                (
                    FFI_COROUTINE_MOVE_COMPLETED,
                    FfiYieldInstruction::next_frame(),
                ),
            ],
            FFI_COROUTINE_READY,
            false,
        );
        // SAFETY: `descriptor` is readable for this call.
        unsafe { schedule_managed_coroutine(&descriptor) };
        tick_managed_coroutines(0.016);
        tick_managed_coroutines(0.016);
        assert_eq!(moves.load(Ordering::SeqCst), 1);
        crate::r#async::queue_main_thread_callback(handle, callback, Vec::new(), 0);
        crate::r#async::dispatch_main_thread_callbacks();
        tick_managed_coroutines(0.016);
        assert_eq!(moves.load(Ordering::SeqCst), 2);
        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clear_rejects_coroutines_started_reentrantly_from_release() {
        let _guard = test_guard();
        reset_restart_counters();
        let descriptor = restart_on_release_descriptor(complete_immediately);
        // SAFETY: `descriptor` is readable for this call.
        assert_ne!(
            unsafe { schedule_managed_coroutine(&descriptor) },
            FfiCoroutineHandle::INVALID
        );

        clear_managed_coroutines();

        assert_eq!(OUTER_RELEASES.load(Ordering::SeqCst), 1);
        assert_eq!(NESTED_RELEASES.load(Ordering::SeqCst), 1);
        assert_eq!(RELEASE_SPAWN_HANDLE.load(Ordering::SeqCst), 0);
        assert_eq!(active_managed_coroutine_count(), 0);
    }

    #[test]
    fn in_flight_clear_epoch_prevents_release_from_resurrecting_work() {
        let _guard = test_guard();
        reset_restart_counters();
        let descriptor = restart_on_release_descriptor(clear_then_complete);
        // SAFETY: `descriptor` is readable for this call.
        assert_ne!(
            unsafe { schedule_managed_coroutine(&descriptor) },
            FfiCoroutineHandle::INVALID
        );

        tick_managed_coroutines(0.016);

        assert_eq!(OUTER_RELEASES.load(Ordering::SeqCst), 1);
        assert_eq!(NESTED_RELEASES.load(Ordering::SeqCst), 1);
        assert_eq!(RELEASE_SPAWN_HANDLE.load(Ordering::SeqCst), 0);
        assert_eq!(active_managed_coroutine_count(), 0);
    }

    #[test]
    fn reentrant_tick_does_not_advance_any_coroutine_twice() {
        let _guard = test_guard();
        let (first, first_moves, first_releases, _) = descriptor(
            vec![(
                FFI_COROUTINE_MOVE_YIELDED,
                FfiYieldInstruction::next_frame(),
            )],
            FFI_COROUTINE_READY,
            false,
        );
        let (second, second_moves, second_releases, _) = descriptor(
            vec![(
                FFI_COROUTINE_MOVE_YIELDED,
                FfiYieldInstruction::next_frame(),
            )],
            FFI_COROUTINE_READY,
            false,
        );
        // SAFETY: Both descriptors are readable for these calls.
        unsafe {
            schedule_managed_coroutine(&first);
            schedule_managed_coroutine(&second);
        }
        REENTER_TICK.store(true, Ordering::SeqCst);

        tick_managed_coroutines(0.016);

        assert_eq!(first_moves.load(Ordering::SeqCst), 1);
        assert_eq!(second_moves.load(Ordering::SeqCst), 1);
        clear_managed_coroutines();
        assert_eq!(first_releases.load(Ordering::SeqCst), 1);
        assert_eq!(second_releases.load(Ordering::SeqCst), 1);
    }
}
