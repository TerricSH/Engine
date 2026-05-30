//! Maps character controller state to animation parameters.
//!
//! The controller outputs movement state which the animation system consumes
//! to drive the locomotion state machine (idle/walk/run/jump/fall/land).
//! This module defines the parameter names and helper to extract them.

use glam::Vec3;

use crate::controller::CharacterController;
use crate::CharacterState;

/// Names of animation parameters produced by the character controller.
///
/// These match the parameter names expected by the locomotion
/// [`AnimStateMachine`](engine_animation::AnimStateMachine) definition.
pub mod anim_params {
    /// Horizontal movement speed in m/s (f32).
    pub const SPEED: &str = "speed";
    /// Whether the character is on the ground (bool).
    pub const GROUNDED: &str = "grounded";
    /// Vertical velocity in m/s (f32) — negative = falling.
    pub const VERTICAL_VELOCITY: &str = "vertical_velocity";
    /// Whether the character is moving (bool, speed > threshold).
    pub const IS_MOVING: &str = "is_moving";
    /// Current movement mode as float (0=idle, 1=walk, 2=run, 3=jump, 4=fall, 5=land).
    pub const MOVE_STATE: &str = "move_state";
}

/// Movement state classification for animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimMoveState {
    Idle,
    Walk,
    Run,
    Jump,
    Fall,
    Land,
}

impl std::fmt::Display for AnimMoveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Walk => write!(f, "walk"),
            Self::Run => write!(f, "run"),
            Self::Jump => write!(f, "jump"),
            Self::Fall => write!(f, "fall"),
            Self::Land => write!(f, "land"),
        }
    }
}

/// Parameters extracted from controller state for animation consumption.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimParams {
    /// Horizontal movement speed in m/s.
    pub speed: f32,
    /// Whether on the ground.
    pub grounded: bool,
    /// Vertical velocity (positive = upward, negative = downward).
    pub vertical_velocity: f32,
    /// Whether the character is moving significantly.
    pub is_moving: bool,
    /// High-level movement state classification.
    pub move_state: AnimMoveState,
}

impl AnimParams {
    /// Extract animation parameters from the controller state.
    pub fn from_controller(controller: &CharacterController) -> Self {
        let vel = controller.velocity();
        let h_speed = Vec3::new(vel.x, 0.0, vel.z).length();
        let grounded = controller.is_grounded();
        let is_moving = h_speed > 0.1;
        let state = controller.state();

        let move_state = match state {
            CharacterState::Jumping => AnimMoveState::Jump,
            CharacterState::Falling if !grounded => AnimMoveState::Fall,
            CharacterState::Grounded if is_moving => {
                if h_speed > 3.0 {
                    AnimMoveState::Run
                } else {
                    AnimMoveState::Walk
                }
            }
            CharacterState::Grounded => AnimMoveState::Idle,
            _ => AnimMoveState::Idle,
        };

        Self {
            speed: h_speed,
            grounded,
            vertical_velocity: vel.y,
            is_moving,
            move_state,
        }
    }

    /// Apply these parameters to an engine_animation state machine instance.
    pub fn apply_to_state_machine(&self, sm: &mut engine_animation::AnimStateMachineInstance) {
        sm.set_param(anim_params::SPEED, engine_animation::AnimParamValue::Float(self.speed));
        sm.set_param(
            anim_params::GROUNDED,
            engine_animation::AnimParamValue::Bool(self.grounded),
        );
        sm.set_param(
            anim_params::VERTICAL_VELOCITY,
            engine_animation::AnimParamValue::Float(self.vertical_velocity),
        );
        sm.set_param(
            anim_params::IS_MOVING,
            engine_animation::AnimParamValue::Bool(self.is_moving),
        );
        sm.set_param(
            anim_params::MOVE_STATE,
            engine_animation::AnimParamValue::Float(match self.move_state {
                AnimMoveState::Idle => 0.0,
                AnimMoveState::Walk => 1.0,
                AnimMoveState::Run => 2.0,
                AnimMoveState::Jump => 3.0,
                AnimMoveState::Fall => 4.0,
                AnimMoveState::Land => 5.0,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctrl(_grounded: bool, vy: f32, h_speed: f32, state: CharacterState) -> CharacterController {
        let mut ctrl = CharacterController::new();
        ctrl.state = state;
        ctrl.velocity = Vec3::new(h_speed, vy, 0.0);
        ctrl
    }

    #[test]
    fn anim_params_idle() {
        let ctrl = make_ctrl(true, 0.0, 0.0, CharacterState::Grounded);
        let p = AnimParams::from_controller(&ctrl);
        assert_eq!(p.speed, 0.0);
        assert!(p.grounded);
        assert!(!p.is_moving);
        assert_eq!(p.move_state, AnimMoveState::Idle);
    }

    #[test]
    fn anim_params_walk() {
        let ctrl = make_ctrl(true, 0.0, 2.0, CharacterState::Grounded);
        let p = AnimParams::from_controller(&ctrl);
        assert!((p.speed - 2.0).abs() < 1e-6);
        assert!(p.grounded);
        assert!(p.is_moving);
        assert_eq!(p.move_state, AnimMoveState::Walk);
    }

    #[test]
    fn anim_params_run() {
        let ctrl = make_ctrl(true, 0.0, 5.0, CharacterState::Grounded);
        let p = AnimParams::from_controller(&ctrl);
        assert_eq!(p.move_state, AnimMoveState::Run);
    }

    #[test]
    fn anim_params_jump() {
        let ctrl = make_ctrl(false, 5.0, 0.0, CharacterState::Jumping);
        let p = AnimParams::from_controller(&ctrl);
        assert_eq!(p.move_state, AnimMoveState::Jump);
        assert!(!p.grounded);
        assert!((p.vertical_velocity - 5.0).abs() < 1e-6);
    }

    #[test]
    fn anim_params_fall() {
        let ctrl = make_ctrl(false, -3.0, 0.0, CharacterState::Falling);
        let p = AnimParams::from_controller(&ctrl);
        assert_eq!(p.move_state, AnimMoveState::Fall);
        assert!(!p.grounded);
        assert!(p.vertical_velocity < 0.0);
    }

    #[test]
    fn anim_params_display() {
        assert_eq!(AnimMoveState::Idle.to_string(), "idle");
        assert_eq!(AnimMoveState::Walk.to_string(), "walk");
        assert_eq!(AnimMoveState::Run.to_string(), "run");
        assert_eq!(AnimMoveState::Jump.to_string(), "jump");
        assert_eq!(AnimMoveState::Fall.to_string(), "fall");
        assert_eq!(AnimMoveState::Land.to_string(), "land");
    }

    #[test]
    fn anim_params_apply_to_state_machine() {
        let sm_def = engine_animation::AnimStateMachine::new("idle".to_string());
        let mut sm = engine_animation::AnimStateMachineInstance::new(sm_def);

        let ctrl = make_ctrl(true, 0.0, 2.0, CharacterState::Grounded);
        let p = AnimParams::from_controller(&ctrl);
        p.apply_to_state_machine(&mut sm);

        // Verify parameters were set
        let speed = sm.get_param("speed");
        assert!(matches!(speed, Some(engine_animation::AnimParamValue::Float(v)) if (*v - 2.0).abs() < 1e-6));
        let grounded = sm.get_param("grounded");
        assert!(matches!(grounded, Some(engine_animation::AnimParamValue::Bool(true))));
        let is_moving = sm.get_param("is_moving");
        assert!(matches!(is_moving, Some(engine_animation::AnimParamValue::Bool(true))));
    }
}
