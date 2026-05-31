use crate::assets::*;
use crate::blend_space::{BlendSpace1D, BlendSpaceSample};
use crate::state_machine::ConditionOp::*;
use crate::state_machine::*;
use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// Helper: sine-wave translation keyframes
// ---------------------------------------------------------------------------

/// Sine-wave keyframes for the Y component only.
fn sine_y(period: f32, amplitude: f32, num: usize) -> Vec<Keyframe<[f32; 3]>> {
    (0..num)
        .map(|i| {
            let t = (i as f32 / (num - 1) as f32) * period;
            let v = (t / period * 2.0 * PI).sin() * amplitude;
            Keyframe {
                time: t,
                value: [0.0, v, 0.0],
            }
        })
        .collect()
}

/// Sine-wave keyframes for Y and Z components simultaneously.
fn sine_yz(period: f32, y_amp: f32, z_amp: f32, num: usize) -> Vec<Keyframe<[f32; 3]>> {
    (0..num)
        .map(|i| {
            let t = (i as f32 / (num - 1) as f32) * period;
            let s = (t / period * 2.0 * PI).sin();
            Keyframe {
                time: t,
                value: [0.0, s * y_amp, s * z_amp],
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helper: rotation keyframes around the X axis (quaternion [x, y, z, w])
// ---------------------------------------------------------------------------

/// Sine-wave rotation keyframes around the X axis over `period` seconds
/// with the given peak `amplitude` (radians) and `num` keyframes.
fn rot_x_sine(period: f32, amplitude: f32, num: usize) -> Vec<Keyframe<[f32; 4]>> {
    (0..num)
        .map(|i| {
            let t = (i as f32 / (num - 1) as f32) * period;
            let angle = amplitude * (std::f32::consts::TAU * t / period).sin();
            let half = angle * 0.5;
            Keyframe {
                time: t,
                value: [half.sin(), 0.0, 0.0, half.cos()],
            }
        })
        .collect()
}

fn rot_x_keys(pairs: &[(f32, f32)]) -> Vec<Keyframe<[f32; 4]>> {
    pairs
        .iter()
        .map(|&(t, angle)| {
            let half = angle * 0.5;
            Keyframe {
                time: t,
                value: [half.sin(), 0.0, 0.0, half.cos()],
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helper: single-value identity keyframes
// ---------------------------------------------------------------------------

fn identity_trans() -> Vec<Keyframe<[f32; 3]>> {
    vec![Keyframe {
        time: 0.0,
        value: [0.0, 0.0, 0.0],
    }]
}

fn identity_rot() -> Vec<Keyframe<[f32; 4]>> {
    vec![Keyframe {
        time: 0.0,
        value: [0.0, 0.0, 0.0, 1.0],
    }]
}

fn identity_scale() -> Vec<Keyframe<[f32; 3]>> {
    vec![Keyframe {
        time: 0.0,
        value: [1.0, 1.0, 1.0],
    }]
}

// ---------------------------------------------------------------------------
// Helper: translation keyframes from (time, value) pairs
// ---------------------------------------------------------------------------

fn trans_keys(pairs: &[(f32, [f32; 3])]) -> Vec<Keyframe<[f32; 3]>> {
    pairs
        .iter()
        .map(|&(t, v)| Keyframe { time: t, value: v })
        .collect()
}

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Returns 6 procedural locomotion `AnimationClip` assets for testing:
///
/// | Clip  | Duration | Looping | Description                              |
/// |-------|----------|---------|------------------------------------------|
/// | idle  | 2.0 s    | yes     | Gentle Y-axis breathing                  |
/// | walk  | 1.0 s    | yes     | Y bounce + Z forward sine               |
/// | run   | 0.6 s    | yes     | Stronger Y bounce + Z forward sine       |
/// | jump  | 0.5 s    | no      | Y parabola (0 → 0.5 → 0)                |
/// | fall  | 1.0 s    | yes     | Slow Y descent with arms-out rotation    |
/// | land  | 0.3 s    | no      | Y squat-and-recover with slight bend     |
///
/// Each clip animates at least joint 0 (root). Idle uses only the root;
/// walk, run, jump, fall, and land also animate joint 1 (upper body).
pub fn locomotion_clips() -> Vec<(&'static str, AnimationClip)> {
    vec![
        ("idle", idle_clip()),
        ("walk", walk_clip()),
        ("run", run_clip()),
        ("jump", jump_clip()),
        ("fall", fall_clip()),
        ("land", land_clip()),
    ]
}

// ---------------------------------------------------------------------------
// Clip builders (internal)
// ---------------------------------------------------------------------------

fn idle_clip() -> AnimationClip {
    AnimationClip {
        name: "idle".into(),
        duration: 2.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: sine_y(2.0, 0.01, 4),
            rotations: identity_rot(),
            scales: identity_scale(),
        }],
        joint_indices: vec![0],
    }
}

fn walk_clip() -> AnimationClip {
    AnimationClip {
        name: "walk".into(),
        duration: 1.0,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                // Y bounce 0.03 + Z forward 0.05 over a 1 s cycle
                translations: sine_yz(1.0, 0.03, 0.05, 4),
                rotations: identity_rot(),
                scales: identity_scale(),
            },
            AnimationChannel {
                joint_index: 1,
                translations: identity_trans(),
                // Upper body leans forward/backward through the stride (sine oscillation)
                rotations: rot_x_sine(1.0, 0.08, 4),
                scales: identity_scale(),
            },
        ],
        joint_indices: vec![0, 1],
    }
}

fn run_clip() -> AnimationClip {
    AnimationClip {
        name: "run".into(),
        duration: 0.6,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                // Stronger Y bounce + faster Z forward over the shorter cycle
                translations: sine_yz(0.6, 0.05, 0.10, 4),
                rotations: identity_rot(),
                scales: identity_scale(),
            },
            AnimationChannel {
                joint_index: 1,
                translations: identity_trans(),
                // More aggressive lean than walk (sine oscillation)
                rotations: rot_x_sine(0.6, 0.15, 4),
                scales: identity_scale(),
            },
        ],
        joint_indices: vec![0, 1],
    }
}

fn jump_clip() -> AnimationClip {
    AnimationClip {
        name: "jump".into(),
        duration: 0.5,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                // Vertical parabola: crouch → leap → land
                translations: trans_keys(&[
                    (0.0, [0.0, 0.0, 0.0]),
                    (0.25, [0.0, 0.5, 0.0]),
                    (0.5, [0.0, 0.0, 0.0]),
                ]),
                rotations: identity_rot(),
                scales: identity_scale(),
            },
            AnimationChannel {
                joint_index: 1,
                translations: identity_trans(),
                // Brief forward rotation at the peak of the jump
                rotations: rot_x_keys(&[(0.0, 0.0), (0.25, 0.3), (0.5, 0.0)]),
                scales: identity_scale(),
            },
        ],
        joint_indices: vec![0, 1],
    }
}

fn fall_clip() -> AnimationClip {
    AnimationClip {
        name: "fall".into(),
        duration: 1.0,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                // Slow downward drift
                translations: trans_keys(&[(0.0, [0.0, 0.0, 0.0]), (1.0, [0.0, -0.02, 0.0])]),
                rotations: identity_rot(),
                scales: identity_scale(),
            },
            AnimationChannel {
                joint_index: 1,
                translations: identity_trans(),
                // Arms-out rotation (protective pose, held constant)
                rotations: rot_x_keys(&[(0.0, 0.4), (1.0, 0.4)]),
                scales: identity_scale(),
            },
        ],
        joint_indices: vec![0, 1],
    }
}

fn land_clip() -> AnimationClip {
    AnimationClip {
        name: "land".into(),
        duration: 0.3,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                // Squat at impact, then recover to standing
                translations: trans_keys(&[
                    (0.0, [0.0, 0.0, 0.0]),
                    (0.15, [0.0, -0.05, 0.0]),
                    (0.3, [0.0, 0.0, 0.0]),
                ]),
                rotations: identity_rot(),
                scales: identity_scale(),
            },
            AnimationChannel {
                joint_index: 1,
                translations: identity_trans(),
                // Slight bend on impact
                rotations: rot_x_keys(&[(0.0, 0.0), (0.15, 0.15), (0.3, 0.0)]),
                scales: identity_scale(),
            },
        ],
        joint_indices: vec![0, 1],
    }
}

// ---------------------------------------------------------------------------
// Locomotion state machine builder
// ---------------------------------------------------------------------------

/// Builds the locomotion [`AnimStateMachine`] for idle/walk/run/jump/fall/land.
///
/// States map to the procedural clips returned by [`locomotion_clips()`].
/// Transitions are driven by animation parameters set from the character
/// controller (see `engine_character::animation_params`):
///
/// | Parameter          | Type  | Purpose                           |
/// |--------------------|-------|-----------------------------------|
/// | `"speed"`          | Float | Horizontal speed in m/s           |
/// | `"grounded"`       | Bool  | Whether on the ground             |
/// | `"vertical_velocity"` | Float | Vertical speed (+up / −down)    |
///
/// Transitions with higher `priority` are evaluated first.
pub fn locomotion_state_machine() -> AnimStateMachine {
    let mut sm = AnimStateMachine::new("idle".into());

    // ---- states ----
    sm.add_state(AnimationState {
        name: "idle".into(),
        clip_asset: "idle".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "walk".into(),
        clip_asset: "walk".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "run".into(),
        clip_asset: "run".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "jump".into(),
        clip_asset: "jump".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: false,
    });
    sm.add_state(AnimationState {
        name: "fall".into(),
        clip_asset: "fall".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "land".into(),
        clip_asset: "land".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: false,
    });

    // ---- transitions from idle ----
    sm.add_transition(StateTransition {
        from_state: "idle".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 0.0, // grounded == false
        }],
        priority: 1,
        blend_duration: 0.15,
    });
    sm.add_transition(StateTransition {
        from_state: "idle".into(),
        to_state: "walk".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Greater,
            threshold: 0.1,
        }],
        priority: 0,
        blend_duration: 0.2,
    });

    // ---- transitions from walk ----
    sm.add_transition(StateTransition {
        from_state: "walk".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 0.0,
        }],
        priority: 1,
        blend_duration: 0.15,
    });
    sm.add_transition(StateTransition {
        from_state: "walk".into(),
        to_state: "run".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Greater,
            threshold: 3.0,
        }],
        priority: 0,
        blend_duration: 0.3,
    });
    sm.add_transition(StateTransition {
        from_state: "walk".into(),
        to_state: "idle".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Less,
            threshold: 0.1,
        }],
        priority: 0,
        blend_duration: 0.2,
    });

    // ---- transitions from run ----
    sm.add_transition(StateTransition {
        from_state: "run".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 0.0,
        }],
        priority: 1,
        blend_duration: 0.15,
    });
    sm.add_transition(StateTransition {
        from_state: "run".into(),
        to_state: "walk".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Less,
            threshold: 3.0,
        }],
        priority: 0,
        blend_duration: 0.3,
    });
    sm.add_transition(StateTransition {
        from_state: "run".into(),
        to_state: "idle".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Less,
            threshold: 0.1,
        }],
        priority: 0,
        blend_duration: 0.2,
    });

    // ---- transitions from jump ----
    sm.add_transition(StateTransition {
        from_state: "jump".into(),
        to_state: "land".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 1.0, // grounded == true
        }],
        priority: 1,
        blend_duration: 0.1,
    });
    sm.add_transition(StateTransition {
        from_state: "jump".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "vertical_velocity".into(),
            op: Less,
            threshold: 0.001, // effectively <= 0 (apex or descending)
        }],
        priority: 0,
        blend_duration: 0.1,
    });

    // ---- transitions from fall ----
    sm.add_transition(StateTransition {
        from_state: "fall".into(),
        to_state: "land".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 1.0,
        }],
        priority: 0,
        blend_duration: 0.1,
    });

    // ---- transitions from land ----
    // land → idle  (grounded && speed < 0.1)
    sm.add_transition(StateTransition {
        from_state: "land".into(),
        to_state: "idle".into(),
        conditions: vec![
            TransitionCondition {
                parameter_name: "grounded".into(),
                op: Equal,
                threshold: 1.0,
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Less,
                threshold: 0.1,
            },
        ],
        priority: 0,
        blend_duration: 0.15,
    });
    // land → walk  (grounded && speed >= 0.1 && speed <= 3.0)
    sm.add_transition(StateTransition {
        from_state: "land".into(),
        to_state: "walk".into(),
        conditions: vec![
            TransitionCondition {
                parameter_name: "grounded".into(),
                op: Equal,
                threshold: 1.0,
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Greater,
                threshold: 0.09, // effectively >= 0.1
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Less,
                threshold: 3.001, // effectively <= 3.0
            },
        ],
        priority: 0,
        blend_duration: 0.15,
    });
    // land → run  (grounded && speed > 3.0)
    sm.add_transition(StateTransition {
        from_state: "land".into(),
        to_state: "run".into(),
        conditions: vec![
            TransitionCondition {
                parameter_name: "grounded".into(),
                op: Equal,
                threshold: 1.0,
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Greater,
                threshold: 3.0,
            },
        ],
        priority: 0,
        blend_duration: 0.2,
    });

    sm
}

// ---------------------------------------------------------------------------
// Blend-space locomotion state machine
// ---------------------------------------------------------------------------

/// Build a locomotion state machine that uses a 1D blend space for walk→run,
/// replacing the discrete walk/run states with a single "locomotion" state.
///
/// The blend space blends between walk and run clips based on the "speed"
/// parameter, with walk at threshold 1.5 and run at threshold 5.0.
///
/// States: idle, locomotion, jump, fall, land.
/// Transitions follow the same parameter conventions as [`locomotion_state_machine`].
pub fn locomotion_blend_sm() -> AnimStateMachine {
    let mut sm = AnimStateMachine::new("idle".to_string());

    // ---- states ----
    sm.add_state(AnimationState {
        name: "idle".into(),
        clip_asset: "idle".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "locomotion".into(),
        clip_asset: String::new(), // unused when blend_space is set
        blend_space_1d: Some(BlendSpace1D::new("speed", vec![
            BlendSpaceSample { threshold: 1.5, clip_asset: "walk".into() },
            BlendSpaceSample { threshold: 5.0, clip_asset: "run".into() },
        ])),
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "jump".into(),
        clip_asset: "jump".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: false,
    });
    sm.add_state(AnimationState {
        name: "fall".into(),
        clip_asset: "fall".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: true,
    });
    sm.add_state(AnimationState {
        name: "land".into(),
        clip_asset: "land".into(),
        blend_space_1d: None,
        speed: 1.0,
        looping: false,
    });

    // ---- transitions from idle ----
    sm.add_transition(StateTransition {
        from_state: "idle".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 0.0, // grounded == false
        }],
        priority: 1,
        blend_duration: 0.15,
    });
    sm.add_transition(StateTransition {
        from_state: "idle".into(),
        to_state: "locomotion".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Greater,
            threshold: 0.1,
        }],
        priority: 0,
        blend_duration: 0.2,
    });

    // ---- transitions from locomotion ----
    sm.add_transition(StateTransition {
        from_state: "locomotion".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 0.0,
        }],
        priority: 1,
        blend_duration: 0.15,
    });
    sm.add_transition(StateTransition {
        from_state: "locomotion".into(),
        to_state: "idle".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "speed".into(),
            op: Less,
            threshold: 0.1,
        }],
        priority: 0,
        blend_duration: 0.2,
    });

    // ---- transitions from jump ----
    sm.add_transition(StateTransition {
        from_state: "jump".into(),
        to_state: "land".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 1.0, // grounded == true
        }],
        priority: 1,
        blend_duration: 0.1,
    });
    sm.add_transition(StateTransition {
        from_state: "jump".into(),
        to_state: "fall".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "vertical_velocity".into(),
            op: Less,
            threshold: 0.001, // effectively <= 0 (apex or descending)
        }],
        priority: 0,
        blend_duration: 0.1,
    });

    // ---- transitions from fall ----
    sm.add_transition(StateTransition {
        from_state: "fall".into(),
        to_state: "land".into(),
        conditions: vec![TransitionCondition {
            parameter_name: "grounded".into(),
            op: Equal,
            threshold: 1.0,
        }],
        priority: 0,
        blend_duration: 0.1,
    });

    // ---- transitions from land ----
    // land → idle  (grounded && speed < 0.1)
    sm.add_transition(StateTransition {
        from_state: "land".into(),
        to_state: "idle".into(),
        conditions: vec![
            TransitionCondition {
                parameter_name: "grounded".into(),
                op: Equal,
                threshold: 1.0,
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Less,
                threshold: 0.1,
            },
        ],
        priority: 0,
        blend_duration: 0.15,
    });
    // land → locomotion  (grounded && speed >= 0.1)
    sm.add_transition(StateTransition {
        from_state: "land".into(),
        to_state: "locomotion".into(),
        conditions: vec![
            TransitionCondition {
                parameter_name: "grounded".into(),
                op: Equal,
                threshold: 1.0,
            },
            TransitionCondition {
                parameter_name: "speed".into(),
                op: Greater,
                threshold: 0.09, // effectively >= 0.1
            },
        ],
        priority: 0,
        blend_duration: 0.15,
    });

    sm
}
