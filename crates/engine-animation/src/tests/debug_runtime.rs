#[test]
fn skeleton_debug_draw_empty_no_crash() {
    let drawer = SkeletonDebugDraw::new();
    let mut buf = engine_renderer::DebugDrawBuffer::new();
    let view = Mat4::IDENTITY;
    let proj = Mat4::IDENTITY;
    drawer.populate(&mut buf, &view, &proj);
    assert!(buf.is_empty());
}

#[test]
fn skeleton_debug_draw_pushed_info_appears() {
    let drawer = SkeletonDebugDraw::new();
    drawer.push(SkeletonDebugInfo {
        world_positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
        parents: vec![None, Some(0)],
        joint_names: vec!["root".into(), "child".into()],
    });

    let mut buf = engine_renderer::DebugDrawBuffer::new();
    let view = Mat4::IDENTITY;
    let proj = Mat4::IDENTITY;
    drawer.populate(&mut buf, &view, &proj);

    // Should have 2 spheres + 1 arrow
    assert_eq!(buf.shapes.len(), 3);
}

// ── Registration tests ─────────────────────────────────────────────────

#[test]
fn register_animation_extensions_registers_components() {
    let mut component_reg = engine_scene::registry::ComponentRegistry::new();
    let mut asset_type_reg = engine_scene::registry::AssetTypeRegistry::new();
    let mut render_ext_reg = engine_renderer::RenderExtensionRegistry::new();
    let mut debug_draw_reg = engine_renderer::DebugDrawRegistry::new();

    let handles = register_animation_extensions(
        &mut component_reg,
        &mut asset_type_reg,
        &mut render_ext_reg,
        &mut debug_draw_reg,
    );

    // Components
    assert!(component_reg.is_registered("engine.animation_player"));
    assert!(component_reg.is_registered("engine.ragdoll"));
    assert!(component_reg.is_registered("engine.ragdoll_part"));
    assert!(component_reg.is_registered("engine.skeleton"));
    assert!(component_reg.is_registered("engine.ik_target"));

    // Asset types
    assert!(asset_type_reg.get("skeleton").is_some());
    assert!(asset_type_reg.get("animation_clip").is_some());
    assert!(asset_type_reg.cooker_for("skel").is_some());
    assert!(asset_type_reg.cooker_for("anim").is_some());

    // Render extension
    assert_eq!(render_ext_reg.producer_count(), 1);

    // Debug draw — SkeletonDebugDraw + IkDebugDraw
    assert_eq!(debug_draw_reg.provider_count(), 2);

    handles.skinned_extract.push(PendingSkinnedItem {
        entity: Some("entity-1".into()),
        mesh: "mesh".into(),
        material: "material".into(),
        skeleton: "skeleton".into(),
        bone_palette: vec![IDENTITY_MAT4_4X4],
        world_transform: IDENTITY_MAT4_4X4,
        bounds_min: [-0.5; 3],
        bounds_max: [0.5; 3],
        render_layer: "default".into(),
        cast_shadows: true,
        morph_target_set: None,
        morph_weights: Vec::new(),
    });
    let mut frame = engine_renderer::RenderFrameInput::empty(1);
    render_ext_reg.produce_all(&mut frame, 1);
    assert_eq!(frame.skinned_items.len(), 1);
    assert_eq!(handles.skinned_extract.pending_count(), 0);
}

// ── Advanced evaluator tests ───────────────────────────────────────────

#[test]
fn update_animation_no_clip_returns_empty() {
    let skel = test_runtime_skeleton();
    let mut player = AnimationPlayer {
        playing: true,
        ..Default::default()
    };
    let palette = update_animation(&mut player, None, Some(&skel), 1.0);
    assert!(palette.is_empty());
}

#[test]
fn external_pose_override_owns_the_final_skinning_pose() {
    let skel = old_test_skeleton();
    let mut local = skel.rest_pose().local_transforms().to_vec();
    local[0].translation = Vec3::new(2.0, 3.0, 4.0);
    let mut player = AnimationPlayer {
        external_pose_override: Some(ExternalPoseOverride {
            local_transforms: local,
            weight: 1.0,
        }),
        ..Default::default()
    };
    let mut state_machine = None;

    let palette = update_animation_pipeline(&mut player, &mut state_machine, &[], &skel, None, 0.0);

    assert_eq!(palette.len(), skel.bone_count());
    assert_eq!(
        player.cached_bone_transforms[0].translation,
        Vec3::new(2.0, 3.0, 4.0)
    );
    assert_eq!(player.cached_bone_positions[0], [2.0, 3.0, 4.0]);
}

#[test]
fn update_animation_no_skeleton_returns_empty() {
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
    let palette = update_animation(&mut player, Some(&clip), None, 1.0);
    assert!(palette.is_empty());
}

#[test]
fn evaluate_clip_with_interpolation() {
    let skeleton = test_skeleton();
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
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![1],
    };

    let at_start = AnimationEvaluator::evaluate(&clip, 0.0, &skeleton);
    assert_eq!(at_start[1].translation, [0.0, 0.0, 0.0]);

    let at_mid = AnimationEvaluator::evaluate(&clip, 0.5, &skeleton);
    assert_eq!(at_mid[1].translation, [5.0, 0.0, 0.0]);

    let at_end = AnimationEvaluator::evaluate(&clip, 1.0, &skeleton);
    assert_eq!(at_end[1].translation, [10.0, 0.0, 0.0]);

    let past_end = AnimationEvaluator::evaluate(&clip, 2.0, &skeleton);
    assert_eq!(past_end[1].translation, [10.0, 0.0, 0.0]);
}
