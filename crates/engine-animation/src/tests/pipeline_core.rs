// =========================================================================
// Unified pipeline integration tests (New Gate 10+)
// =========================================================================

#[test]
fn unified_skeleton_conversion() {
    let skel = test_skeleton();
    let (runtime, joint_map) = skeleton_asset_to_runtime(&skel);

    assert_eq!(runtime.bone_count(), 2);
    assert_eq!(runtime.bone_name(BoneIndex(0)), Some("root"));
    assert_eq!(runtime.bone_name(BoneIndex(1)), Some("child"));
    assert_eq!(runtime.parent_of(BoneIndex(1)), Some(BoneIndex(0)));

    assert_eq!(joint_map.len(), 2);
    assert_eq!(joint_map[0], BoneIndex(0));
    assert_eq!(joint_map[1], BoneIndex(1));
}

#[test]
fn imported_inverse_bind_matrix_is_used_by_runtime_skinning() {
    let inverse_bind = Mat4::from_translation(Vec3::new(-2.0, 0.0, 0.0)).to_cols_array_2d();
    let asset = Skeleton {
        joints: vec![Joint {
            name: "root".into(),
            parent_index: None,
            local_transform: JointTransform::IDENTITY,
        }],
        inverse_bind_matrices: vec![inverse_bind],
    };
    let (runtime, _) = skeleton_asset_to_runtime(&asset);
    let matrices = runtime.rest_pose().skin_matrices(&runtime);

    assert_eq!(matrices.len(), 1);
    assert!((matrices[0].w_axis.x + 2.0).abs() < 1e-5);
}

#[test]
fn unified_clip_conversion() {
    let skel = test_skeleton();
    let (runtime_skel, joint_map) = skeleton_asset_to_runtime(&skel);

    // Asset clip: animate joint 1 translation [0,0,0] → [10,0,0] over 1 s.
    let clip = AnimationClip {
        name: "test".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 1,
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
        joint_indices: vec![1],
    };

    let runtime_clip = clip_asset_to_runtime(&clip, &joint_map);

    // Sample at t = 0.5 → interpolated translation should be [5, 0, 0].
    let pose_half = runtime_clip.sample(0.5, &runtime_skel);
    assert!(
        (pose_half.local[1].translation.x - 5.0).abs() < 1e-5,
        "mid translation.x expected ≈5.0 got {}",
        pose_half.local[1].translation.x
    );
    assert!(
        (pose_half.local[1].translation.y).abs() < 1e-5,
        "mid translation.y expected ≈0.0 got {}",
        pose_half.local[1].translation.y
    );
    assert!(
        (pose_half.local[1].translation.z).abs() < 1e-5,
        "mid translation.z expected ≈0.0 got {}",
        pose_half.local[1].translation.z
    );

    // Sample at t = 0.0 → identity translation.
    let pose_zero = runtime_clip.sample(0.0, &runtime_skel);
    assert_eq!(
        pose_zero.local[1].translation,
        Vec3::ZERO,
        "start translation should be zero"
    );
}

#[test]
fn pipeline_evaluate_to_skin_matrices() {
    let skel = test_runtime_skeleton();
    let clip = AnimationClip {
        name: "test_clip".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    let player = AnimationPlayer {
        clip_asset: Some("test_clip".into()),
        playing: true,
        ..Default::default()
    };

    let mut player = player;
    let matrices = update_animation_pipeline(
        &mut player,
        &mut None,
        &[("test_clip", clip)],
        &skel,
        None,
        0.0,
    );

    assert_eq!(matrices.len(), skel.bone_count());
    // First matrix should be near identity (no animation, root at origin).
    let m0 = &matrices[0];
    assert!(
        (m0[0][0] - 1.0).abs() < 1e-5
            && (m0[1][1] - 1.0).abs() < 1e-5
            && (m0[2][2] - 1.0).abs() < 1e-5
            && (m0[3][3] - 1.0).abs() < 1e-5,
        "first skin matrix should be near identity, got {m0:?}"
    );
}

#[test]
fn pipeline_ik_via_orchestrator() {
    // 4-bone chain: root(0) → hip(1) → knee(2) → foot(3)
    let skel = old_test_skeleton();

    // IK chain: foot→knee→hip→root (tip→base).
    let chain = IkChain::new(
        "leg",
        vec![BoneIndex(3), BoneIndex(2), BoneIndex(1), BoneIndex(0)],
    )
    .with_solver(IkSolverType::Fabrik)
    .with_iterations(30)
    .with_tolerance(0.01);

    let target = Vec3::new(0.3, -0.3, 0.0);
    let effector = IkEffector::new("foot_target", BoneIndex(3), target);

    let ik = IkTargetComponent {
        effectors: vec![effector],
        chains: vec![chain],
        constraints: IkConstraintSet::new(),
        enabled: true,
        blend_weight: 1.0,
    };

    let mut player = AnimationPlayer {
        playing: false, // uses rest pose
        ..Default::default()
    };

    let matrices = update_animation_pipeline(&mut player, &mut None, &[], &skel, Some(&ik), 0.0);

    assert_eq!(matrices.len(), 4);

    // At rest the foot (bone 3) global position is [0,0,0], so the
    // skin-matrix translation column equals the foot's world position.
    let foot_pos = Vec3::new(matrices[3][3][0], matrices[3][3][1], matrices[3][3][2]);
    let rest_dist = Vec3::ZERO.distance(target);
    let ik_dist = foot_pos.distance(target);

    assert!(
        ik_dist < rest_dist,
        "IK foot ({foot_pos:?}) should be closer to target ({target:?}) \
         than rest ({rest_dist:.4}); ik_dist={ik_dist:.4}"
    );
    assert!(
        ik_dist < 0.1,
        "IK foot too far from target: {ik_dist:.4} (expected < 0.1)"
    );
}

#[test]
fn pipeline_constraint_enforced() {
    // 4-bone chain: root(0) → hip(1) → knee(2) → foot(3)
    let skel = old_test_skeleton();

    let chain = IkChain::new(
        "leg",
        vec![BoneIndex(3), BoneIndex(2), BoneIndex(1), BoneIndex(0)],
    )
    .with_solver(IkSolverType::Fabrik)
    .with_iterations(30)
    .with_tolerance(0.01);

    // Extreme target that would cause hyper-extension without constraints.
    let effector = IkEffector::new("foot_target", BoneIndex(3), Vec3::new(2.0, 0.0, 2.0));

    // Very tight twist/swing limits on the knee (BoneIndex 2) — ±1°.
    let mut constraints = IkConstraintSet::new();
    constraints.add(
        IkConstraint::new(BoneIndex(2))
            .with_twist(-1.0, 1.0)
            .with_swing(-1.0, 1.0),
    );

    let ik = IkTargetComponent {
        effectors: vec![effector],
        chains: vec![chain.clone()],
        constraints,
        enabled: true,
        blend_weight: 1.0,
    };

    let mut player = AnimationPlayer {
        playing: false,
        ..Default::default()
    };

    // ── Smoke test: pipeline runs without panicking ──────────────────────
    let matrices = update_animation_pipeline(&mut player, &mut None, &[], &skel, Some(&ik), 0.0);
    assert_eq!(matrices.len(), 4);

    // ── Direct constraint verification via solve_pose_multi ──────────────
    // The pipeline internally calls solve_pose_multi which applies the
    // constraint to the knee.  We re-solve here to inspect the knee's local
    // rotation directly.
    let effector_direct = IkEffector::new("foot_target", BoneIndex(3), Vec3::new(2.0, 0.0, 2.0));
    let mut constraints_direct = IkConstraintSet::new();
    // Same tight ±1° limit used by the pipeline.
    constraints_direct.add(
        IkConstraint::new(BoneIndex(2))
            .with_twist(-1.0, 1.0)
            .with_swing(-1.0, 1.0),
    );

    let mut pose = skel.rest_pose();
    solve_pose_multi(
        &mut pose,
        &skel,
        &[chain],
        &[effector_direct],
        &constraints_direct,
    );

    // Decompose the knee's local rotation into swing + twist around Z.
    let knee_rot = pose.local[2].rotation;
    let rest_rot = Quat::IDENTITY; // knee's rest rotation is identity.
    let delta = rest_rot.inverse() * knee_rot;

    // Swing-twist decomposition (see solver.rs for the canonical impl).
    let v = Vec3::new(delta.x, delta.y, delta.z);
    let proj = Vec3::Z * v.dot(Vec3::Z);
    let twist = Quat::from_xyzw(proj.x, proj.y, proj.z, delta.w).normalize();
    let (_twist_axis, twist_angle) = twist.to_axis_angle();

    // Constraint limits twist to ±1°.
    let max_allowed_rad = 1.1_f32.to_radians();
    assert!(
        twist_angle.abs() < max_allowed_rad,
        "knee twist {:.4}° exceeds ±1° constraint (max {:.4}°)",
        twist_angle.to_degrees(),
        max_allowed_rad.to_degrees()
    );
}
