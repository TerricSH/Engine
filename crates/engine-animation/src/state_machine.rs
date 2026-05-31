use serde::{Deserialize, Serialize};

use crate::blend_space::BlendSpace1D;

// ---------------------------------------------------------------------------
// Animation parameter types
// ---------------------------------------------------------------------------

/// Types of animation parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AnimParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
}

/// A named parameter used by transition conditions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimParameter {
    pub name: String,
    pub value: AnimParamValue,
}

/// Comparison operator for transition conditions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionOp {
    Greater,
    Less,
    Equal,
    NotEqual,
}

/// A single transition condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionCondition {
    pub parameter_name: String,
    pub op: ConditionOp,
    pub threshold: f32,
}

// ---------------------------------------------------------------------------
// State machine asset types
// ---------------------------------------------------------------------------

/// A state in the animation state machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationState {
    pub name: String,
    pub clip_asset: String,
    /// Optional 1D blend space. When present the state evaluates the blend
    /// space instead of the single `clip_asset`.
    #[serde(default)]
    pub blend_space_1d: Option<BlendSpace1D>,
    pub speed: f32,
    pub looping: bool,
}

/// A transition between two states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateTransition {
    pub from_state: String,
    pub to_state: String,
    pub conditions: Vec<TransitionCondition>,
    #[serde(default)]
    pub priority: u8,
    pub blend_duration: f32,
}

/// An animation state machine asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimStateMachine {
    pub states: Vec<AnimationState>,
    pub transitions: Vec<StateTransition>,
    pub default_state: String,
}

impl AnimStateMachine {
    pub fn new(default_state: String) -> Self {
        Self {
            states: Vec::new(),
            transitions: Vec::new(),
            default_state,
        }
    }

    pub fn add_state(&mut self, state: AnimationState) {
        if !self.states.iter().any(|s| s.name == state.name) {
            self.states.push(state);
        }
    }

    pub fn add_transition(&mut self, transition: StateTransition) {
        self.transitions.push(transition);
    }

    pub fn find_state(&self, name: &str) -> Option<&AnimationState> {
        self.states.iter().find(|s| s.name == name)
    }
}

// ---------------------------------------------------------------------------
// Runtime state machine instance
// ---------------------------------------------------------------------------

/// Runtime state for a state machine instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimStateMachineInstance {
    pub state_machine: AnimStateMachine,
    pub current_state: String,
    pub current_time: f32,
    /// Whether a crossfade transition is in progress.
    pub transitioning: bool,
    /// The state being transitioned *from*.
    pub transition_from: String,
    /// Progress within the transition blend (0.0 = fully `from`, 1.0 = fully `to`).
    pub transition_progress: f32,
    /// Total duration of the current transition in seconds.
    pub transition_duration: f32,
    /// Runtime parameter values.
    pub parameters: Vec<AnimParameter>,
}

impl AnimStateMachineInstance {
    /// Create a new instance starting at the machine's default state.
    pub fn new(state_machine: AnimStateMachine) -> Self {
        let default = state_machine.default_state.clone();
        Self {
            state_machine,
            current_state: default,
            current_time: 0.0,
            transitioning: false,
            transition_from: String::new(),
            transition_progress: 0.0,
            transition_duration: 0.0,
            parameters: Vec::new(),
        }
    }

    /// Set a parameter value (creates if not exists).
    pub fn set_param(&mut self, name: &str, value: AnimParamValue) {
        if let Some(p) = self.parameters.iter_mut().find(|p| p.name == name) {
            p.value = value;
        } else {
            self.parameters.push(AnimParameter {
                name: name.to_string(),
                value,
            });
        }
    }

    /// Get a parameter value by name.
    pub fn get_param(&self, name: &str) -> Option<&AnimParamValue> {
        self.parameters
            .iter()
            .find(|p| p.name == name)
            .map(|p| &p.value)
    }

    /// Force-transition to a named state immediately, bypassing conditions.
    /// This is useful for script-driven animation control (cutscenes, etc.).
    pub fn force_transition_to(&mut self, state_name: &str) -> bool {
        if self.state_machine.states.iter().any(|s| s.name == state_name) {
            self.current_state = state_name.to_string();
            self.current_time = 0.0;
            self.transitioning = false;
            true
        } else {
            false
        }
    }

    /// Evaluate all given conditions — returns `true` only if every condition passes.
    fn evaluate_conditions(&self, conditions: &[TransitionCondition]) -> bool {
        conditions.iter().all(|cond| {
            let val = match self.get_param(&cond.parameter_name) {
                Some(AnimParamValue::Float(f)) => *f,
                Some(AnimParamValue::Int(i)) => *i as f32,
                Some(AnimParamValue::Bool(b)) => {
                    return match cond.op {
                        ConditionOp::Equal => *b == (cond.threshold != 0.0),
                        ConditionOp::NotEqual => *b != (cond.threshold != 0.0),
                        _ => false,
                    };
                }
                None => return false,
            };
            match cond.op {
                ConditionOp::Greater => val > cond.threshold,
                ConditionOp::Less => val < cond.threshold,
                ConditionOp::Equal => (val - cond.threshold).abs() < 0.001,
                ConditionOp::NotEqual => (val - cond.threshold).abs() >= 0.001,
            }
        })
    }

    /// Find the highest-priority matching transition whose origin matches
    /// `current_state` and whose conditions are all satisfied. If multiple
    /// transitions have the same priority, the first one is returned (stable).
    fn find_active_transition(&self) -> Option<&StateTransition> {
        self.state_machine
            .transitions
            .iter()
            .filter(|t| t.from_state == self.current_state && self.evaluate_conditions(&t.conditions))
            .fold(None, |best, t| match best {
                None => Some(t),
                Some(b) if t.priority > b.priority => Some(t),
                _ => best,
            })
    }

    /// Advance the state machine by `dt` seconds.
    ///
    /// Returns a `(state_name, blend_weight)` pair where `blend_weight` is
    /// `0..1` during a crossfade transition (0 = from-state, 1 = to-state) and
    /// `1.0` when fully on the current state.
    pub fn update(&mut self, dt: f32) -> (&str, f32) {
        if self.transitioning {
            // Advance the crossfade blend
            if self.transition_duration > 0.0 {
                self.transition_progress =
                    (self.transition_progress + dt / self.transition_duration).min(1.0);
            } else {
                self.transition_progress = 1.0;
            }

            if self.transition_progress >= 1.0 {
                // Transition complete — snap to the new state
                self.transitioning = false;
                self.current_time = 0.0;
                return (&self.current_state, 1.0);
            }

            return (&self.current_state, self.transition_progress);
        }

        // Advance current clip time
        if let Some(state) = self.state_machine.find_state(&self.current_state) {
            self.current_time += dt * state.speed;
        }

        // Check for an active transition (clone to avoid borrow conflict)
        if let Some(transition) = self.find_active_transition().cloned() {
            self.transitioning = true;
            self.transition_from = self.current_state.clone();
            self.current_state = transition.to_state;
            self.transition_progress = 0.0;
            self.transition_duration = transition.blend_duration;
            return (&self.current_state, 0.0);
        }

        (&self.current_state, 1.0)
    }
}
