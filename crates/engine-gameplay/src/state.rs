//! # Game State Manager (G18-F01)
//!
//! Finite state machine for the game lifecycle.  Supports:
//!
//! * **Defined states** — `Boot`, `Menu`, `Loading`, `Playing`, `Paused`, `GameOver`.
//! * **Transition rules** — only explicitly added transitions are permitted
//!   via `request_transition`; invalid transitions return `Err`.
//! * **Forced transitions** — `force_transition` bypasses rule validation
//!   (useful for loading screens and emergency resets).
//! * **Previous-state stack** — tracks prior states so the manager can
//!   support resume-like patterns (e.g. `Paused → Playing`).
//! * **C#-compatible callbacks** — register `Box<dyn FnMut>` functions
//!   that fire on every state change.

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// GameState
// ---------------------------------------------------------------------------

/// The discrete states in the game lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameState {
    /// Engine is initialising — no user interaction yet.
    Boot,
    /// Main menu / title screen.
    Menu,
    /// Loading assets or a new scene.
    Loading,
    /// Active gameplay.
    Playing,
    /// Game is paused (menus / settings overlay).
    Paused,
    /// Game-over screen.
    GameOver,
}

impl GameState {
    /// All known states, in declaration order.
    pub const ALL: &'static [GameState] = &[
        Self::Boot,
        Self::Menu,
        Self::Loading,
        Self::Playing,
        Self::Paused,
        Self::GameOver,
    ];

    /// Convert to a `u32` for FFI.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Boot => 0,
            Self::Menu => 1,
            Self::Loading => 2,
            Self::Playing => 3,
            Self::Paused => 4,
            Self::GameOver => 5,
        }
    }

    /// Convert from a `u32` (returns `None` for out-of-range values).
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Boot),
            1 => Some(Self::Menu),
            2 => Some(Self::Loading),
            3 => Some(Self::Playing),
            4 => Some(Self::Paused),
            5 => Some(Self::GameOver),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// StateTransitionRule
// ---------------------------------------------------------------------------

/// Describes one allowed transition.
///
/// A transition from any state in `from` to `to` is allowed only when
/// `condition()` returns `true`.
pub struct StateTransitionRule {
    /// Which source states this rule applies to.
    pub from: HashSet<GameState>,
    /// The destination state.
    pub to: GameState,
    /// An optional predicate that must return `true` for the transition
    /// to succeed.  When `None` the transition is always allowed.
    pub condition: Option<Box<dyn FnMut() -> bool + Send>>,
}

impl std::fmt::Debug for StateTransitionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateTransitionRule")
            .field("from", &self.from)
            .field("to", &self.to)
            .field("condition", &self.condition.as_ref().map(|_| "…"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// OnStateChange callback
// ---------------------------------------------------------------------------

/// Unique identifier for a registered state-change callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CallbackId(u64);

impl CallbackId {
    fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------
// StateChangeCallback type alias
// ---------------------------------------------------------------------------

type StateChangeCallback = (
    CallbackId,
    Box<dyn FnMut(&GameState, &GameState) + Send>,
);

// ---------------------------------------------------------------------------
// GameStateManager
// ---------------------------------------------------------------------------

/// The game state machine.
///
/// Managed states: `Boot`, `Menu`, `Loading`, `Playing`, `Paused`, `GameOver`.
/// Transitions are validated against registered `StateTransitionRule`s;
/// `force_transition` bypasses validation.  Callbacks fire on every state
/// change and are designed to be used from C# via `engine-ffi`.
///
/// # Example
///
/// ```
/// use engine_gameplay::state::{GameState, GameStateManager};
///
/// let mut mgr = GameStateManager::new(GameState::Boot);
/// mgr.add_transition(GameState::Boot, GameState::Menu);
///
/// assert!(mgr.request_transition(GameState::Menu).is_ok());
/// assert!(mgr.request_transition(GameState::Paused).is_err()); // no rule Menu→Paused
/// ```
pub struct GameStateManager {
    current: GameState,
    previous: Vec<GameState>,
    transitions: Vec<StateTransitionRule>,
    callbacks: Vec<StateChangeCallback>,
}

impl GameStateManager {
    /// Create a new manager starting in the given state.
    pub fn new(initial: GameState) -> Self {
        Self {
            current: initial,
            previous: Vec::new(),
            transitions: Vec::new(),
            callbacks: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // State queries
    // -----------------------------------------------------------------------

    /// The current state.
    pub fn current(&self) -> GameState {
        self.current
    }

    /// The stack of previous states (most recent first).
    pub fn previous_states(&self) -> &[GameState] {
        &self.previous
    }

    /// The list of registered transition rules.
    pub fn transitions(&self) -> &[StateTransitionRule] {
        &self.transitions
    }

    // -----------------------------------------------------------------------
    // Transitions
    // -----------------------------------------------------------------------

    /// Register a transition rule from `from` to `to` with no condition
    /// (always allowed).
    pub fn add_transition(&mut self, from: GameState, to: GameState) {
        let mut from_set = HashSet::new();
        from_set.insert(from);
        self.transitions.push(StateTransitionRule {
            from: from_set,
            to,
            condition: None,
        });
    }

    /// Register a transition rule from a set of `from` states to `to`
    /// with an optional condition predicate.
    pub fn add_transition_with_condition(
        &mut self,
        from: HashSet<GameState>,
        to: GameState,
        condition: Option<Box<dyn FnMut() -> bool + Send>>,
    ) {
        self.transitions.push(StateTransitionRule {
            from,
            to,
            condition,
        });
    }

    /// Attempt a validated transition to `target`.
    ///
    /// Returns `Ok(())` if a matching rule exists and its condition
    /// passes, or `Err(reason)` otherwise.
    pub fn request_transition(&mut self, target: GameState) -> Result<(), String> {
        if target == self.current {
            return Err("already in requested state".into());
        }

        for rule in &mut self.transitions {
            if rule.to == target && rule.from.contains(&self.current) {
                if let Some(ref mut cond) = rule.condition {
                    if !cond() {
                        return Err("transition condition not met".into());
                    }
                }
                self.previous.push(self.current);
                self.current = target;
                self._fire_callbacks();
                return Ok(());
            }
        }

        Err(format!(
            "no transition rule from {:?} to {:?}",
            self.current, target
        ))
    }

    /// Force a transition without validation.
    pub fn force_transition(&mut self, target: GameState) {
        if target == self.current {
            return;
        }
        self.previous.push(self.current);
        self.current = target;
        self._fire_callbacks();
    }

    /// Attempt to transition back to the most recent previous state.
    pub fn pop_state(&mut self) -> Result<(), String> {
        let prev = self
            .previous
            .last()
            .copied()
            .ok_or_else(|| "no previous state".to_string())?;
        self.request_transition(prev)
    }

    // -----------------------------------------------------------------------
    // Callbacks
    // -----------------------------------------------------------------------

    /// Register a callback that fires on every state change.
    pub fn on_state_change<F>(&mut self, callback: F) -> CallbackId
    where
        F: FnMut(&GameState, &GameState) + Send + 'static,
    {
        let id = CallbackId::next();
        self.callbacks.push((id, Box::new(callback)));
        id
    }

    /// Remove a previously registered callback.
    pub fn unregister_callback(&mut self, id: CallbackId) -> bool {
        let len_before = self.callbacks.len();
        self.callbacks.retain(|(cid, _)| *cid != id);
        self.callbacks.len() < len_before
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn _fire_callbacks(&mut self) {
        let prev = self
            .previous
            .last()
            .expect("previous must be set before firing");
        let new = self.current;
        for (_, cb) in &mut self.callbacks {
            cb(prev, &new);
        }
    }
}

// ---------------------------------------------------------------------------
// Default transition rules (convenience)
// ---------------------------------------------------------------------------

impl GameStateManager {
    /// Register a sensible set of default transitions for a typical game:
    ///
    /// | From       | To        |
    /// |------------|-----------|
    /// | Boot       | Menu      |
    /// | Menu       | Loading   |
    /// | Loading    | Playing   |
    /// | Playing    | Paused    |
    /// | Paused     | Playing   |
    /// | Playing    | GameOver  |
    /// | GameOver   | Menu      |
    pub fn with_default_transitions(initial: GameState) -> Self {
        let mut mgr = Self::new(initial);
        mgr.add_default_transitions();
        mgr
    }

    /// Add default transitions to an existing manager.
    pub fn add_default_transitions(&mut self) {
        // Boot → Menu
        self.add_transition(GameState::Boot, GameState::Menu);
        // Menu → Loading
        self.add_transition(GameState::Menu, GameState::Loading);
        // Loading → Playing
        self.add_transition(GameState::Loading, GameState::Playing);
        // Playing ↔ Paused
        self.add_transition(GameState::Playing, GameState::Paused);
        self.add_transition(GameState::Paused, GameState::Playing);
        // Playing → GameOver
        self.add_transition(GameState::Playing, GameState::GameOver);
        // GameOver → Menu
        self.add_transition(GameState::GameOver, GameState::Menu);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    // -- Valid transitions --------------------------------------------------

    #[test]
    fn boot_to_menu() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        assert_eq!(mgr.current(), GameState::Boot);
        assert!(mgr.request_transition(GameState::Menu).is_ok());
        assert_eq!(mgr.current(), GameState::Menu);
    }

    #[test]
    fn full_lifecycle() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        assert!(mgr.request_transition(GameState::Menu).is_ok());
        assert!(mgr.request_transition(GameState::Loading).is_ok());
        assert!(mgr.request_transition(GameState::Playing).is_ok());
        assert!(mgr.request_transition(GameState::Paused).is_ok());
        assert!(mgr.request_transition(GameState::Playing).is_ok());
        assert!(mgr.request_transition(GameState::GameOver).is_ok());
        assert!(mgr.request_transition(GameState::Menu).is_ok());
    }

    // -- Invalid transitions ------------------------------------------------

    #[test]
    fn invalid_transition_rejected() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        // No rule Boot → Playing
        assert!(mgr.request_transition(GameState::Playing).is_err());
        assert_eq!(mgr.current(), GameState::Boot);
    }

    #[test]
    fn no_rule_returns_err() {
        // Start with no transition rules at all.
        let mut mgr = GameStateManager::new(GameState::Boot);
        assert!(mgr.request_transition(GameState::Menu).is_err());
    }

    #[test]
    fn same_state_rejected() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        // Even with a rule Boot → Menu, requesting Boot → Boot should fail.
        assert!(mgr.request_transition(GameState::Boot).is_err());
    }

    #[test]
    fn condition_blocks_transition() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        let allowed = Arc::new(AtomicBool::new(false));
        let cond = Arc::clone(&allowed);

        mgr.add_transition_with_condition(
            HashSet::from([GameState::Boot]),
            GameState::Menu,
            Some(Box::new(move || cond.load(Ordering::SeqCst))),
        );

        // Condition is false → transition is blocked.
        assert!(mgr.request_transition(GameState::Menu).is_err());
        assert_eq!(mgr.current(), GameState::Boot);

        // Flip condition → transition succeeds.
        allowed.store(true, Ordering::SeqCst);
        assert!(mgr.request_transition(GameState::Menu).is_ok());
        assert_eq!(mgr.current(), GameState::Menu);
    }

    // -- Force transition ---------------------------------------------------

    #[test]
    fn force_transition_bypasses_validation() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        // No rule Boot → Playing, but force should still work.
        mgr.force_transition(GameState::Playing);
        assert_eq!(mgr.current(), GameState::Playing);
    }

    #[test]
    fn force_transition_to_same_is_noop() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        mgr.force_transition(GameState::Boot);
        assert_eq!(mgr.current(), GameState::Boot);
        assert!(mgr.previous_states().is_empty());
    }

    // -- Previous state tracking --------------------------------------------

    #[test]
    fn previous_state_tracked() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        mgr.request_transition(GameState::Menu).unwrap();
        assert_eq!(mgr.previous_states(), &[GameState::Boot]);
    }

    #[test]
    fn multiple_previous_states() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        mgr.request_transition(GameState::Menu).unwrap();
        mgr.request_transition(GameState::Loading).unwrap();
        mgr.request_transition(GameState::Playing).unwrap();
        assert_eq!(
            mgr.previous_states(),
            &[GameState::Boot, GameState::Menu, GameState::Loading]
        );
    }

    #[test]
    fn pop_state_goes_back() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        mgr.add_transition(GameState::Boot, GameState::Menu);
        mgr.add_transition(GameState::Menu, GameState::Boot); // return rule
        mgr.request_transition(GameState::Menu).unwrap();
        assert!(mgr.pop_state().is_ok());
        assert_eq!(mgr.current(), GameState::Boot);
    }

    #[test]
    fn pop_state_empty_stack() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        assert!(mgr.pop_state().is_err());
    }

    // -- Callbacks ----------------------------------------------------------

    #[test]
    fn callback_fires_on_transition() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        let transitions = Arc::new(Mutex::new(Vec::new()));
        let t = Arc::clone(&transitions);

        mgr.on_state_change(move |old, new| {
            t.lock().unwrap().push((*old, *new));
        });

        mgr.request_transition(GameState::Menu).unwrap();
        assert_eq!(transitions.lock().unwrap().len(), 1);
        assert_eq!(
            transitions.lock().unwrap()[0],
            (GameState::Boot, GameState::Menu)
        );

        mgr.request_transition(GameState::Loading).unwrap();
        assert_eq!(transitions.lock().unwrap().len(), 2);
        assert_eq!(
            transitions.lock().unwrap()[1],
            (GameState::Menu, GameState::Loading)
        );
    }

    #[test]
    fn callback_fires_on_force_transition() {
        let mut mgr = GameStateManager::new(GameState::Boot);
        let fired = Arc::new(Mutex::new(false));
        let f = Arc::clone(&fired);

        mgr.on_state_change(move |_, _| {
            *f.lock().unwrap() = true;
        });

        mgr.force_transition(GameState::Playing);
        assert!(*fired.lock().unwrap());
    }

    #[test]
    fn unregister_callback() {
        let mut mgr = GameStateManager::with_default_transitions(GameState::Boot);
        let count = Arc::new(Mutex::new(0u32));
        let c = Arc::clone(&count);

        let id = mgr.on_state_change(move |_, _| {
            *c.lock().unwrap() += 1;
        });

        mgr.request_transition(GameState::Menu).unwrap();
        assert_eq!(*count.lock().unwrap(), 1);

        assert!(mgr.unregister_callback(id));
        mgr.request_transition(GameState::Loading).unwrap();
        // Callback was removed, count stays at 1.
        assert_eq!(*count.lock().unwrap(), 1);
    }

    // -- GameState helpers --------------------------------------------------

    #[test]
    fn state_u32_roundtrip() {
        for state in GameState::ALL {
            let v = state.to_u32();
            assert_eq!(GameState::from_u32(v), Some(*state));
        }
    }

    #[test]
    fn state_from_u32_invalid() {
        assert_eq!(GameState::from_u32(99), None);
    }
}
