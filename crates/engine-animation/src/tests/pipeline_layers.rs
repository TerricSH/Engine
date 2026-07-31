#[test]
fn pipeline_empty_state_machine_no_crash() {
    // State machine with no states.
    let sm = AnimStateMachine::new("".into());
    let sm_instance = AnimStateMachineInstance::new(sm);
    let mut sm_opt = Some(sm_instance);

    let mut player = AnimationPlayer {
        playing: true,
        ..Default::default()
    };

    // 0-bone skeleton so the resulting palette is also empty.
    let skel = crate::skeleton::Skeleton::new("empty".into());

    let matrices = update_animation_pipeline(&mut player, &mut sm_opt, &[], &skel, None, 0.0);

    assert!(
        matrices.is_empty(),
        "expected empty matrices for empty state machine + empty skeleton, got {} matrices",
        matrices.len()
    );
}

#[test]
fn pipeline_clip_advances_time() {
    let skel = test_runtime_skeleton();

    // Clip: animate bone 0 translation [0,0,0] → [10,0,0] over 1 second.
    let clip = AnimationClip {
        name: "test_clip".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [10.0, 0.0, 0.0],
                },
            ],
            rotations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [0.0, 0.0, 0.0, 1.0],
                },
            ],
            scales: vec![
                Keyframe {
                    time: 0.0,
                    value: [1.0, 1.0, 1.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [1.0, 1.0, 1.0],
                },
            ],
        }],
        joint_indices: vec![0],
    };

    let mut player = AnimationPlayer {
        clip_asset: Some("test_clip".into()),
        playing: true,
        speed: 1.0,
        current_time: 0.0,
        ..Default::default()
    };

    let matrices = update_animation_pipeline(
        &mut player,
        &mut None,
        &[("test_clip", clip)],
        &skel,
        None,
        0.5, // dt = 0.5s → effective time = 0.0 + 0.5 * 1.0 = 0.5
    );

    assert_eq!(matrices.len(), 2);
    assert!((player.current_time - 0.5).abs() < 1e-5);

    // At effective time 0.5, bone 0 local translation is [5, 0, 0].
    // rest_global[0] = identity (root at origin), so
    // skin_matrix[0] = current_global[0] = translate([5, 0, 0]).
    assert!(
        (matrices[0][3][0] - 5.0).abs() < 1e-4,
        "expected tx ≈ 5.0 at t=0.5, got {}",
        matrices[0][3][0]
    );
    assert!(
        matrices[0][3][1].abs() < 1e-5,
        "expected ty ≈ 0.0 at t=0.5, got {}",
        matrices[0][3][1]
    );
    assert!(
        matrices[0][3][2].abs() < 1e-5,
        "expected tz ≈ 0.0 at t=0.5, got {}",
        matrices[0][3][2]
    );
}

#[test]
fn pipeline_paused_clip_holds_current_pose() {
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "paused".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [10.0, 0.0, 0.0],
                },
            ],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![0],
    };
    let mut player = AnimationPlayer {
        clip_asset: Some("paused".into()),
        playing: false,
        current_time: 0.5,
        ..Default::default()
    };

    let matrices = update_animation_pipeline(
        &mut player,
        &mut None,
        &[("paused", clip)],
        &skel,
        None,
        0.5,
    );

    assert!((player.current_time - 0.5).abs() < 1e-5);
    assert!((matrices[0][3][0] - 5.0).abs() < 1e-4);
}

#[test]
fn pipeline_non_looping_clip_stops_at_end() {
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "once".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![
                Keyframe {
                    time: 0.0,
                    value: [0.0, 0.0, 0.0],
                },
                Keyframe {
                    time: 1.0,
                    value: [10.0, 0.0, 0.0],
                },
            ],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![0],
    };
    let mut player = AnimationPlayer {
        clip_asset: Some("once".into()),
        playing: true,
        looping: false,
        current_time: 0.75,
        ..Default::default()
    };

    let matrices =
        update_animation_pipeline(&mut player, &mut None, &[("once", clip)], &skel, None, 0.5);

    assert!(!player.playing);
    assert!((player.current_time - 1.0).abs() < 1e-5);
    assert!((matrices[0][3][0] - 10.0).abs() < 1e-4);
}

fn two_bone_layer_clip(name: &str) -> AnimationClip {
    AnimationClip {
        name: name.into(),
        duration: 1.0,
        channels: vec![
            AnimationChannel {
                joint_index: 0,
                translations: vec![
                    Keyframe {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0],
                    },
                    Keyframe {
                        time: 1.0,
                        value: [10.0, 0.0, 0.0],
                    },
                ],
                rotations: vec![],
                scales: vec![],
            },
            AnimationChannel {
                joint_index: 1,
                translations: vec![
                    Keyframe {
                        time: 0.0,
                        value: [0.0, 1.0, 0.0],
                    },
                    Keyframe {
                        time: 1.0,
                        value: [4.0, 1.0, 0.0],
                    },
                ],
                rotations: vec![],
                scales: vec![],
            },
        ],
        joint_indices: vec![0, 1],
    }
}

#[test]
fn pipeline_overwrite_layer_respects_bone_mask() {
    let skel = test_runtime_skeleton();
    let clip = two_bone_layer_clip("upper_body");
    let mut player = AnimationPlayer {
        playing: true,
        layers: vec![
            AnimLayer::new("base"),
            AnimLayer::new("upper")
                .with_clip("upper_body")
                .with_mask(vec![1])
                .with_blend_mode(LayerBlendMode::Overwrite),
        ],
        ..Default::default()
    };

    let matrices = update_animation_pipeline(
        &mut player,
        &mut None,
        &[("upper_body", clip)],
        &skel,
        None,
        0.5,
    );

    assert!(matrices[0][3][0].abs() < 1e-5);
    assert!((matrices[1][3][0] - 2.0).abs() < 1e-4);
    assert!((player.layers[1].current_time - 0.5).abs() < 1e-5);
}

#[test]
fn pipeline_additive_layer_uses_rest_pose_delta() {
    let skel = test_runtime_skeleton();
    let clip = two_bone_layer_clip("additive");
    let mut player = AnimationPlayer {
        playing: true,
        layers: vec![
            AnimLayer::new("base"),
            AnimLayer::new("offset")
                .with_clip("additive")
                .with_weight(0.5)
                .with_mask(vec![1])
                .with_blend_mode(LayerBlendMode::Additive),
        ],
        ..Default::default()
    };

    let matrices = update_animation_pipeline(
        &mut player,
        &mut None,
        &[("additive", clip)],
        &skel,
        None,
        0.5,
    );

    assert!(matrices[0][3][0].abs() < 1e-5);
    assert!((matrices[1][3][0] - 1.0).abs() < 1e-4);
}
