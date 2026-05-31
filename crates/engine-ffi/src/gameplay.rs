//! Game state and event bus FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points for
//! the gameplay state machine and event bus.
//!
//! # Safety policy
//!
//! Every function that accepts a raw pointer documents its safety
//! contract with a `// SAFETY:` comment.  Null pointers are handled
//! gracefully (no-op / false / 0 return).
//!
//! Interning rules for GameState:
//!
//! | Value | State      |
//! |-------|------------|
//! | 0     | Boot       |
//! | 1     | Menu       |
//! | 2     | Loading    |
//! | 3     | Playing    |
//! | 4     | Paused     |
//! | 5     | GameOver   |
//!
//! Event type codes:
//!
//! | Value | Event            | Value field |
//! |-------|------------------|-------------|
//! | 0     | ScoreChanged     | f32 → i32  |
//! | 1     | LivesChanged     | f32 → i32  |
//! | 2     | HealthChanged    | f32        |
//! | 3     | AmmoChanged      | f32 → u32  |
//! | 4     | GameStateChanged | f32 → u32  |
//! | 5     | DialogueTriggered | str_val    |
//! | 6     | ObjectiveUpdated  | str_val    |
//! | 7     | QuestCompleted    | str_val    |
//! | 8     | Custom            | str_val    |

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use engine_gameplay::event_bus::{EventBus, GameplayEvent};
use engine_gameplay::state::GameState;
use engine_gameplay::GameStateManager;

// ---------------------------------------------------------------------------
// Game State Manager FFI
// ---------------------------------------------------------------------------

/// Request a state transition on the given manager.
///
/// `state` is the integer code for the target state (0–5).
/// Returns `true` if the transition was accepted, `false` otherwise (invalid
/// transition or null pointer).
///
/// # Safety
/// `manager` must be a valid pointer to a `GameStateManager`, or null.
#[no_mangle]
pub unsafe extern "C" fn gameplay_request_state(
    manager: *mut std::ffi::c_void,
    state: i32,
) -> bool {
    if manager.is_null() {
        return false;
    }
    // SAFETY: Caller guarantees a valid GameStateManager or null.
    let mgr = &mut *(manager as *mut GameStateManager);

    let target = match state {
        0 => GameState::Boot,
        1 => GameState::Menu,
        2 => GameState::Loading,
        3 => GameState::Playing,
        4 => GameState::Paused,
        5 => GameState::GameOver,
        _ => return false,
    };

    mgr.request_transition(target).is_ok()
}

/// Returns the integer code for the current state of the given manager.
///
/// Returns 0 (Boot) for a null pointer.
///
/// # Safety
/// `manager` must be a valid pointer to a `GameStateManager`, or null.
#[no_mangle]
pub unsafe extern "C" fn gameplay_current_state(manager: *const std::ffi::c_void) -> i32 {
    if manager.is_null() {
        return 0;
    }
    // SAFETY: Caller guarantees a valid GameStateManager or null.
    let mgr = &*(manager as *const GameStateManager);
    mgr.current().to_u32() as i32
}

// ---------------------------------------------------------------------------
// Event Bus FFI
// ---------------------------------------------------------------------------

/// Publish a gameplay event to the given bus.
///
/// * `event_type` — integer code (0–8, see table at module level).
/// * `value` — f32 payload; interpretation depends on event_type.
/// * `str_val` — optional string payload (for dialogue, quests, custom).
///
/// # Safety
/// `bus` must be a valid pointer to an `EventBus`, or null.
/// `str_val` must be a valid null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn gameplay_publish_event(
    bus: *mut std::ffi::c_void,
    event_type: i32,
    value: f32,
    str_val: *const c_char,
) {
    if bus.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid EventBus or null.
    let bus = &mut *(bus as *mut EventBus);

    let str_payload = if str_val.is_null() {
        String::new()
    } else {
        // SAFETY: Caller guarantees a valid null-terminated string.
        CStr::from_ptr(str_val).to_string_lossy().into_owned()
    };

    let event = match event_type {
        0 => GameplayEvent::ScoreChanged(value as i32),
        1 => GameplayEvent::LivesChanged(value as i32),
        2 => GameplayEvent::HealthChanged(value),
        3 => GameplayEvent::AmmoChanged(value as u32),
        4 => {
            let state = GameState::from_u32(value as u32).unwrap_or(GameState::Boot);
            GameplayEvent::GameStateChanged(state)
        }
        5 => GameplayEvent::DialogueTriggered(str_payload),
        6 => GameplayEvent::ObjectiveUpdated(str_payload),
        7 => GameplayEvent::QuestCompleted(str_payload),
        8 => GameplayEvent::Custom(str_payload, String::new()),
        _ => return, // unknown event type
    };

    bus.publish(event);
}

/// Subscribe to an event type.
///
/// Returns a `u64` subscription ID, or 0 on failure.
///
/// # Safety
/// `bus` must be a valid pointer to an `EventBus`, or null.
/// `event_type_str` must be a valid null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn gameplay_subscribe(
    bus: *mut std::ffi::c_void,
    event_type_str: *const c_char,
    callback: Option<
        extern "C" fn(event_type: i32, value: f32, str_val: *const c_char, user_data: u64),
    >,
    user_data: u64,
) -> u64 {
    if bus.is_null() || callback.is_none() {
        return 0;
    }
    // SAFETY: Null-checked above; caller guarantees valid pointers or null.
    let bus = &mut *(bus as *mut EventBus);
    let et = if event_type_str.is_null() {
        String::new()
    } else {
        CStr::from_ptr(event_type_str)
            .to_string_lossy()
            .into_owned()
    };

    let cb = callback.unwrap();

    let id = bus.subscribe(
        if et.is_empty() { "*" } else { &et },
        Box::new(move |ev: &GameplayEvent| {
            let (code, fval, sval) = event_to_ffi(ev);
            // SAFETY: The C# side registered this callback; we trust it to
            // be valid for the duration of the subscription.
            // Using as_ptr() instead of into_raw() — the CString is dropped
            // after the callback, freeing the allocation.  C# must copy
            // the string synchronously; the pointer is invalid after cb() returns.
            let c_string = CString::new(sval).unwrap_or_default();
            cb(code, fval, c_string.as_ptr(), user_data);
        }),
    );

    id.to_u64()
}

/// Translate a `GameplayEvent` back into FFI-friendly components.
fn event_to_ffi(ev: &GameplayEvent) -> (i32, f32, String) {
    match ev {
        GameplayEvent::ScoreChanged(v) => (0, *v as f32, String::new()),
        GameplayEvent::LivesChanged(v) => (1, *v as f32, String::new()),
        GameplayEvent::HealthChanged(v) => (2, *v, String::new()),
        GameplayEvent::AmmoChanged(v) => (3, *v as f32, String::new()),
        GameplayEvent::GameStateChanged(s) => (4, s.to_u32() as f32, String::new()),
        GameplayEvent::DialogueTriggered(s) => (5, 0.0, s.clone()),
        GameplayEvent::ObjectiveUpdated(s) => (6, 0.0, s.clone()),
        GameplayEvent::QuestCompleted(s) => (7, 0.0, s.clone()),
        GameplayEvent::Custom(k, v) => (8, 0.0, format!("{}:{}", k, v)),
    }
}
