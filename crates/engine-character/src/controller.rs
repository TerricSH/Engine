//! Character controller definition and state machine.
//!
//! Contains the [`CharacterController`] struct, the [`CharacterState`] enum
//! implementing the **State** design pattern, and the [`CharacterError`] type.

use engine_physics::PhysicsWorld;
use engine_scene::Component;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::CharacterMovement;

// ── Error type ───────────────────────────────────────────────────────────────

/// Typed errors returned by character controller operations.
#[derive(Error, Debug)]
pub enum CharacterError {
    /// The provided input contained invalid or out-of-range values.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// A physics-world reference is required but was not provided.
    #[error("physics world is not available for collision queries")]
    PhysicsWorldMissing,
}

// ── State machine ────────────────────────────────────────────────────────────

/// Describes the character's vertical movement state.
///
/// The controller transitions between these states based on ground detection,
/// jump input, and vertical velocity.
///
/// # State diagram (State pattern)
///
/// ```text
///                         ┌──────────┐
///                     ┌──>│ Grounded │<──────┐
///                     │   └─────┬────┘       │
///                     │         │            │
///                     │    jump │        land │
///                     │         │      timer │
///                     │   ┌─────▼────┐   ┌───┴────┐
///                     │   │ Jumping  │   │ Landing│
///                     │   └─────┬────┘   └───┬────┘
///                     │         │            │
///                     │    apex │       land │
///                     │         │            │
///                     │   ┌─────▼────┐       │
///                     └───│ Falling  │───────┘
///                     │   └──────────┘
///                     │
///                     │   ┌──────────┐
///                     └──>│   Free   │ (manual entry/exit only)
///                         └──────────┘
/// ```
///
/// # Transitions
///
/// | From        | To         | Trigger               |
/// |-------------|------------|-----------------------|
/// | Grounded    | Grounded   | Stay on ground        |
/// | Grounded    | Jumping    | Jump input            |
/// | Grounded    | Falling    | Walk off edge         |
/// | Jumping     | Falling    | Reach apex (v.y ≤ 0)  |
/// | Jumping     | Landing    | Land on surface       |
/// | Falling     | Landing    | Land on surface       |
/// | Landing     | Grounded   | Recovery timer expires (200 ms) |
/// | *           | Free       | Set explicitly        |
/// | Free        | *          | Set explicitly        |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharacterState {
    /// On the ground; can jump.
    Grounded,
    /// Ascending after a jump.
    Jumping,
    /// Descending (after jump peak or walking off an edge).
    Falling,
    /// Landing recovery — on the ground but not yet ready to jump.
    ///
    /// Entered automatically when ground is detected after falling or
    /// jumping. After a brief recovery period (200 ms) the controller
    /// transitions to [`Grounded`](CharacterState::Grounded).
    Landing,
    /// No constraints (e.g., in air from external launch).
    ///
    /// The controller will not automatically transition out of `Free`;
    /// it must be set explicitly by gameplay code.
    Free,
}

impl CharacterState {
    /// Returns `true` if a transition from `self` to `other` is valid
    /// according to the state machine rules.
    ///
    /// This implements the **State** design pattern: each state knows which
    /// successor states are reachable. The controller uses this guard in
    /// [`CharacterController::transition_state`] to reject illegal
    /// transitions.
    ///
    /// # Examples
    ///
    /// ```
    /// # use engine_character::CharacterState;
    /// assert!(CharacterState::Grounded.can_transition_to(CharacterState::Jumping));
    /// assert!(CharacterState::Jumping.can_transition_to(CharacterState::Falling));
    /// assert!(CharacterState::Jumping.can_transition_to(CharacterState::Landing));
    /// assert!(CharacterState::Falling.can_transition_to(CharacterState::Landing));
    /// assert!(CharacterState::Landing.can_transition_to(CharacterState::Grounded));
    /// assert!(CharacterState::Free.can_transition_to(CharacterState::Grounded));
    /// assert!(!CharacterState::Jumping.can_transition_to(CharacterState::Grounded)); // must land first
    /// assert!(!CharacterState::Falling.can_transition_to(CharacterState::Grounded)); // must go through Landing
    /// ```
    pub fn can_transition_to(self, other: CharacterState) -> bool {
        if self == other {
            return true; // identity transition is always valid
        }
        match (self, other) {
            // Grounded can jump or fall off an edge.
            (CharacterState::Grounded, CharacterState::Jumping)
            | (CharacterState::Grounded, CharacterState::Falling) => true,

            // Jumping transitions to falling at the apex.
            (CharacterState::Jumping, CharacterState::Falling) => true,

            // Jumping or Falling can land (entering Landing recovery).
            (CharacterState::Jumping, CharacterState::Landing)
            | (CharacterState::Falling, CharacterState::Landing) => true,

            // Landing recovery complete → Grounded.
            (CharacterState::Landing, CharacterState::Grounded) => true,

            // Free state can go anywhere and anywhere can go to Free.
            (CharacterState::Free, _) | (_, CharacterState::Free) => true,

            // All other transitions are illegal.
            _ => false,
        }
    }
}

// ── Movement command ─────────────────────────────────────────────────────────

/// A movement command issued by input, AI, or C# code.
///
/// Commands are pushed into [`CharacterController::pending_commands`] and
/// consumed each frame by [`CharacterController::update`].  The controller
/// owns movement resolution — callers express intent, not transforms.
#[derive(Debug, Clone, Copy)]
pub struct CharacterCommand {
    /// Desired horizontal movement direction (normalised).
    pub direction: Vec3,
    /// Desired speed in m/s.  `0` or negative uses the controller's
    /// built-in [`move_speed`](CharacterController::move_speed).
    pub desired_speed: f32,
    /// Whether the character should attempt a jump.
    pub jump_requested: bool,
}

impl CharacterCommand {
    /// Create a movement-only command.
    pub fn move_towards(direction: Vec3) -> Self {
        Self {
            direction,
            desired_speed: 0.0,
            jump_requested: false,
        }
    }
    /// Create a jump command.
    pub fn jump() -> Self {
        Self {
            direction: Vec3::ZERO,
            desired_speed: 0.0,
            jump_requested: true,
        }
    }
}

// ── Character controller ─────────────────────────────────────────────────────

/// A kinematic character controller.
///
/// Moves a capsule-shaped character through the world using configurable
/// movement parameters and ray-based collision detection. The character is
/// **not** a physics rigid body — it moves by setting position directly and
/// uses the physics world only for collision queries.
///
/// # Defaults
///
/// | Parameter        | Value | Unit       |
/// |------------------|-------|------------|
/// | `height`         | 1.8   | metres     |
/// | `radius`         | 0.3   | metres     |
/// | `move_speed`     | 5.0   | m/s        |
/// | `acceleration`   | 20.0  | m/s²       |
/// | `deceleration`   | 15.0  | m/s²       |
/// | `air_acceleration` | 5.0 | m/s²       |
/// | `air_deceleration` | 2.0 | m/s²       |
/// | `jump_velocity`  | 5.0   | m/s        |
/// | `gravity_scale`  | 1.0   | multiplier |
/// | `max_fall_speed` | 20.0  | m/s        |
/// | `step_height`    | 0.3   | metres     |
/// | `slope_limit`    | 45.0  | degrees    |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterController {
    // ── Capsule shape ───────────────────────────────────────────────────
    /// Total capsule height in metres (default: 1.8).
    pub height: f32,

    /// Capsule radius in metres (default: 0.3).
    pub radius: f32,

    // ── Movement parameters ─────────────────────────────────────────────
    /// Maximum horizontal speed in m/s (default: 5.0).
    pub move_speed: f32,

    /// Ground acceleration in m/s² (default: 20.0).
    pub acceleration: f32,

    /// Ground deceleration / friction in m/s² (default: 15.0).
    pub deceleration: f32,

    /// Air acceleration in m/s² (default: 5.0).
    pub air_acceleration: f32,

    /// Air deceleration in m/s² (default: 2.0).
    pub air_deceleration: f32,

    // ── Jump & gravity ──────────────────────────────────────────────────
    /// Upward velocity applied on jump in m/s (default: 5.0).
    pub jump_velocity: f32,

    /// Gravity multiplier (default: 1.0).
    ///
    /// A value of 0.0 disables gravity; 2.0 doubles gravity.
    pub gravity_scale: f32,

    /// Terminal fall speed in m/s (default: 20.0).
    ///
    /// The character's downward velocity will never exceed this magnitude.
    pub max_fall_speed: f32,

    // ── Collision ───────────────────────────────────────────────────────
    /// Maximum walkable step height in metres (default: 0.3).
    ///
    /// Ground with a vertical step ≤ this value is traversable.
    pub step_height: f32,

    /// Maximum walkable slope angle in degrees (default: 45.0).
    ///
    /// Surfaces steeper than this are treated as walls for ground-detection
    /// purposes.
    pub slope_limit: f32,

    // ── Command queue ────────────────────────────────────────────────────
    /// Pending movement commands (queued by input/AI/C#).
    /// Flushed each frame by [`update`](Self::update).
    #[serde(skip)]
    pub pending_commands: Vec<CharacterCommand>,

    // ── Internal state (crate-visible for FFI) ──────────────────────────
    pub(crate) state: CharacterState,
    pub(crate) position: Vec3,
    pub(crate) velocity: Vec3,

    /// Timer for Landing recovery state (seconds).
    ///
    /// Accumulated each frame while in [`CharacterState::Landing`]; when
    /// it exceeds 200 ms the controller transitions to [`Grounded`](CharacterState::Grounded).
    #[serde(skip)]
    pub landing_timer: f32,
}

impl Component for CharacterController {
    const TYPE_ID: &'static str = "engine.character_controller";
}

impl CharacterController {
    /// Create a new character controller with default parameters at the origin.
    pub fn new() -> Self {
        Self {
            height: 1.8,
            radius: 0.3,
            move_speed: 5.0,
            acceleration: 20.0,
            deceleration: 15.0,
            air_acceleration: 5.0,
            air_deceleration: 2.0,
            jump_velocity: 5.0,
            gravity_scale: 1.0,
            max_fall_speed: 20.0,
            step_height: 0.3,
            slope_limit: 45.0,
            pending_commands: Vec::new(),
            state: CharacterState::Falling,
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            landing_timer: 0.0,
        }
    }

    // ── State accessors ─────────────────────────────────────────────────

    /// Return the current vertical movement state.
    pub fn state(&self) -> CharacterState {
        self.state
    }

    /// Returns `true` if the character is on the ground.
    pub fn is_grounded(&self) -> bool {
        self.state == CharacterState::Grounded
    }

    /// Return the current velocity.
    pub fn velocity(&self) -> Vec3 {
        self.velocity
    }

    /// Return the current world-space position (center of the capsule).
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Set the character's world-space position.
    ///
    /// This teleports the character; no collision checks are performed.
    /// For continuous movement use [`process_movement`] instead.
    pub fn set_position(&mut self, pos: Vec3) {
        self.position = pos;
    }

    // ── State pattern ───────────────────────────────────────────────────

    /// Attempt a state transition, validating it against the state machine
    /// rules defined by [`CharacterState::can_transition_to`].
    ///
    /// Returns `Ok(())` on success or
    /// [`CharacterError::InvalidInput`] if the transition is illegal.
    ///
    /// # State design pattern
    ///
    /// This method encapsulates the state transition logic so that
    /// callers (and the movement system) cannot accidentally put the
    /// controller into an invalid state. The valid transitions are:
    ///
    /// | From        | To         | Trigger               |
    /// |-------------|------------|-----------------------|
    /// | Grounded    | Grounded   | Stay on ground        |
    /// | Grounded    | Jumping    | Jump input            |
    /// | Grounded    | Falling    | Walk off edge         |
    /// | Jumping     | Falling    | Reach apex (v.y ≤ 0)  |
    /// | Jumping     | Landing    | Land on surface       |
    /// | Falling     | Landing    | Land on surface       |
    /// | Landing     | Grounded   | Recovery timer expires |
    /// | *           | Free       | Set explicitly        |
    /// | Free        | *          | Set explicitly        |
    pub fn transition_state(&mut self, new_state: CharacterState) -> Result<(), CharacterError> {
        if self.state.can_transition_to(new_state) {
            self.state = new_state;
            Ok(())
        } else {
            Err(CharacterError::InvalidInput(format!(
                "cannot transition from {:?} to {:?}",
                self.state, new_state
            )))
        }
    }
}

impl Default for CharacterController {
    fn default() -> Self {
        Self::new()
    }
}

// ── Frame update ──────────────────────────────────────────────────────────

impl CharacterController {
    /// Push a movement command into the pending queue.
    ///
    /// Commands are consumed (in FIFO order) the next time
    /// [`update`](Self::update) is called.  Multiple commands per
    /// frame are merged — the last command's direction wins.
    pub fn push_command(&mut self, cmd: CharacterCommand) {
        self.pending_commands.push(cmd);
    }

    /// Run one frame of character movement.
    ///
    /// First drains any pending [`CharacterCommand`]s from the queue,
    /// then applies gravity, horizontal acceleration, collision
    /// resolution, ground detection, and state transitions.
    /// Mutates the controller in place.
    ///
    /// Returns `true` if the character's position changed this frame.
    pub fn update(&mut self, input: &CharacterMovement, physics: Option<&PhysicsWorld>) -> bool {
        // Merge pending commands into the input
        let mut cmd = *input;
        for pending in self.pending_commands.drain(..) {
            if pending.direction.length_squared() > 0.0 {
                cmd.direction = pending.direction;
            }
            if pending.desired_speed > 0.0 {
                self.move_speed = pending.desired_speed;
            }
            if pending.jump_requested {
                cmd.wish_jump = true;
            }
        }

        let output = crate::movement::process_movement(self, &cmd, physics);
        self.position = output.new_position;
        self.velocity = output.new_velocity;

        // ── Landing recovery timer ──────────────────────────────────────
        // We inspect `output.state` (this frame's result) and `self.state`
        // (previous frame's state, still intact) to detect entering Landing.
        if output.state == CharacterState::Landing {
            if self.state != CharacterState::Landing {
                // Just entered Landing — start fresh timer.
                self.landing_timer = 0.0;
            }
            self.landing_timer += cmd.delta_time;
            if self.landing_timer > 0.2 {
                self.state = CharacterState::Grounded;
            } else {
                self.state = CharacterState::Landing;
            }
        } else {
            self.state = output.state;
            // Reset timer whenever we leave the Landing state.
            self.landing_timer = 0.0;
        }

        output.moved
    }
}
