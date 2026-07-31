// =========================================================================
// New Gate 10 asset tests
// =========================================================================

// ── Helper: 2-bone skeleton for testing ────────────────────────────────

fn test_skeleton() -> Skeleton {
    Skeleton {
        joints: vec![
            Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            },
            Joint {
                name: "child".into(),
                parent_index: Some(0),
                local_transform: JointTransform {
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            },
        ],
        inverse_bind_matrices: vec![IDENTITY_MAT4_4X4; 2],
    }
}

const IDENTITY_MAT4_4X4: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

// ── Keyframe lerp tests ────────────────────────────────────────────────

#[test]
fn lerp_translation_identity() {
    let a = [0.0, 0.0, 0.0];
    let b = [10.0, 20.0, 30.0];
    assert_eq!(AnimationEvaluator::lerp_translation(&a, &b, 0.0), a);
    assert_eq!(AnimationEvaluator::lerp_translation(&a, &b, 1.0), b);
}

#[test]
fn lerp_translation_midpoint() {
    let a = [0.0, 0.0, 0.0];
    let b = [10.0, 20.0, 30.0];
    let mid = AnimationEvaluator::lerp_translation(&a, &b, 0.5);
    assert_eq!(mid, [5.0, 10.0, 15.0]);
}

#[test]
fn lerp_rotation_identity() {
    let a = [0.0, 0.0, 0.0, 1.0]; // identity quat
    let b = [0.0, 0.0, 0.0, 1.0];
    let r = AnimationEvaluator::lerp_rotation(&a, &b, 0.5);
    assert!((r[3] - 1.0).abs() < 1e-5);
}

#[test]
fn lerp_rotation_ninety_degrees() {
    // Rotate 90° around X: q = (sin(45°), 0, 0, cos(45°)) for 90° total
    // Halfway should be 45° around X
    let a = [0.0, 0.0, 0.0, 1.0]; // identity
    let b = [FRAC_1_SQRT_2, 0.0, 0.0, FRAC_1_SQRT_2]; // 90° around X
    let mid = AnimationEvaluator::lerp_rotation(&a, &b, 0.5);
    // At 45° around X: (sin(22.5°), 0, 0, cos(22.5°))
    let expected_w = (22.5f32).to_radians().cos();
    let expected_x = (22.5f32).to_radians().sin();
    assert!(
        (mid[0] - expected_x).abs() < 1e-5,
        "x={} expected={}",
        mid[0],
        expected_x
    );
    assert!(
        (mid[3] - expected_w).abs() < 1e-5,
        "w={} expected={}",
        mid[3],
        expected_w
    );
}

#[test]
fn lerp_scale_midpoint() {
    let a = [1.0, 1.0, 1.0];
    let b = [2.0, 3.0, 4.0];
    let mid = AnimationEvaluator::lerp_scale(&a, &b, 0.5);
    assert_eq!(mid, [1.5, 2.0, 2.5]);
}

// ── Evaluator tests ────────────────────────────────────────────────────

#[test]
fn evaluate_empty_clip_returns_identity() {
    let skeleton = test_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let local = AnimationEvaluator::evaluate(&clip, 0.0, &skeleton);
    assert_eq!(local.len(), 2);
    assert_eq!(local[0], JointTransform::IDENTITY);
    assert_eq!(local[1], JointTransform::IDENTITY);
}

#[test]
fn evaluate_single_channel_overrides_joint() {
    let skeleton = test_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 1,
            translations: vec![Keyframe {
                time: 0.0,
                value: [5.0, 10.0, 0.0],
            }],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![1],
    };
    let local = AnimationEvaluator::evaluate(&clip, 0.0, &skeleton);
    assert_eq!(local[1].translation, [5.0, 10.0, 0.0]);
    // Non-animated joints stay identity
    assert_eq!(local[0], JointTransform::IDENTITY);
}

#[test]
fn evaluate_pose_preserves_rest_components_missing_from_channel() {
    let skeleton = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "rotate_child".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 1,
            translations: vec![],
            rotations: vec![Keyframe {
                time: 0.0,
                value: Quat::from_rotation_y(1.0).to_array(),
            }],
            scales: vec![],
        }],
        joint_indices: vec![1],
    };

    let pose = AnimationEvaluator::evaluate_pose(&clip, 0.0, &skeleton);

    assert_eq!(pose.local_transforms()[1].translation, Vec3::Y);
    assert_eq!(pose.local_transforms()[1].scale, Vec3::ONE);
    assert!(pose.local_transforms()[1].rotation.is_finite());
}

#[test]
fn evaluate_interpolates_between_keyframes() {
    let skeleton = test_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 2.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 2.0,
                    value: [10.0, 0.0, 0.0],
                },
            ],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![0],
    };
    let local = AnimationEvaluator::evaluate(&clip, 1.0, &skeleton);
    assert_eq!(local[0].translation, [5.0, 0.0, 0.0]);
}

// ── Hierarchy solve tests (via Pose::global_transforms) ────────────────

#[test]
fn hierarchy_solve_identity_via_pose() {
    let skel = test_runtime_skeleton();
    // Override pose with identity transforms for both bones.
    let mut pose = Pose::new(&skel);
    pose.local[0] = BoneTransform::IDENTITY;
    pose.local[1] = BoneTransform::IDENTITY;

    let global = pose.global_transforms(&skel);
    assert_eq!(global.len(), 2);
    for bt in &global {
        assert_eq!(bt.translation, Vec3::ZERO);
        assert_eq!(bt.rotation, Quat::IDENTITY);
        assert_eq!(bt.scale, Vec3::ONE);
    }
}

#[test]
fn hierarchy_solve_composes_parent_child_via_pose() {
    let skel = test_runtime_skeleton();
    // Create a pose with root at [1,0,0] and child at [0,2,0].
    let mut pose = Pose::new(&skel);
    pose.local[0].translation = Vec3::new(1.0, 0.0, 0.0);
    pose.local[1].translation = Vec3::new(0.0, 2.0, 0.0);

    let global = pose.global_transforms(&skel);
    assert_eq!(global[0].translation, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(global[1].translation, Vec3::new(1.0, 2.0, 0.0));
}

// ── AnimationPlayer component time advancement ─────────────────────────

#[test]
fn player_default_is_stopped() {
    let p = AnimationPlayer::new();
    assert!(!p.playing);
    assert_eq!(p.current_time, 0.0);
    assert_eq!(p.speed, 1.0);
    assert!(p.looping);
}

#[test]
fn player_advances_time_with_speed() {
    let mut player = AnimationPlayer {
        playing: true,
        speed: 2.0,
        ..Default::default()
    };
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 10.0,
        channels: vec![],
        joint_indices: vec![],
    };

    let _palette = update_animation(&mut player, Some(&clip), Some(&skel), 1.0);
    assert!((player.current_time - 2.0).abs() < 1e-5);
}

#[test]
fn player_looping_wraps_time() {
    let mut player = AnimationPlayer {
        playing: true,
        looping: true,
        current_time: 9.0,
        ..Default::default()
    };
    let clip = AnimationClip {
        name: "test".into(),
        duration: 10.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let skel = test_runtime_skeleton();
    let _palette = update_animation(&mut player, Some(&clip), Some(&skel), 2.0);
    // 9 + 2 = 11, rem_euclid(10) = 1
    assert!((player.current_time - 1.0).abs() < 1e-5);
}

#[test]
fn player_non_looping_clamps_and_stops() {
    let mut player = AnimationPlayer {
        playing: true,
        looping: false,
        current_time: 8.0,
        ..Default::default()
    };
    let clip = AnimationClip {
        name: "test".into(),
        duration: 10.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let skel = test_runtime_skeleton();
    let _palette = update_animation(&mut player, Some(&clip), Some(&skel), 5.0);
    assert!((player.current_time - 10.0).abs() < 1e-5);
    assert!(!player.playing);
}

#[test]
fn player_paused_does_not_advance() {
    let mut player = AnimationPlayer {
        playing: false,
        current_time: 3.0,
        ..Default::default()
    };
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 10.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let _palette = update_animation(&mut player, Some(&clip), Some(&skel), 5.0);
    assert!((player.current_time - 3.0).abs() < 1e-5);
}

#[test]
fn player_update_returns_bone_palette() {
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "test".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let mut player = AnimationPlayer {
        playing: true,
        ..Default::default()
    };
    let palette = update_animation(&mut player, Some(&clip), Some(&skel), 0.0);
    // Should return 2 identity matrices (one per joint)
    assert_eq!(palette.len(), 2);
    assert_eq!(palette[0], IDENTITY_MAT4_4X4);
    assert_eq!(palette[1], IDENTITY_MAT4_4X4);
}

// ── Component Serialize/Deserialize ────────────────────────────────────
