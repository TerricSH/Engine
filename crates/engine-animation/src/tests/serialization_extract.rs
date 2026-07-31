#[test]
fn animation_player_roundtrip() {
    let p = AnimationPlayer {
        clip_asset: Some("walk.anim".into()),
        playing: true,
        looping: false,
        speed: 1.5,
        current_time: 2.0,
        layer: 1,
        state_machine: None,
        layers: vec![AnimLayer::new("base")],
        cached_bone_positions: Vec::new(),
        cached_bone_transforms: Vec::new(),
        external_pose_override: None,
    };
    let bytes = bincode::serialize(&p).unwrap();
    let restored: AnimationPlayer = bincode::deserialize(&bytes).unwrap();
    assert_eq!(restored.clip_asset, Some("walk.anim".into()));
    assert!(restored.playing);
    assert!(!restored.looping);
    assert!((restored.speed - 1.5).abs() < 1e-5);
    assert!((restored.current_time - 2.0).abs() < 1e-5);
    assert_eq!(restored.layer, 1);
}

#[test]
fn skeleton_component_roundtrip() {
    let sc = SkeletonComponent {
        skeleton_asset: Some("human.skel".into()),
        bind_shape: [1.0, 2.0, 3.0],
        morph_target_set: Some("human.morphs".into()),
        morph_weights: vec![0.5],
    };
    let bytes = bincode::serialize(&sc).unwrap();
    let restored: SkeletonComponent = bincode::deserialize(&bytes).unwrap();
    assert_eq!(restored.skeleton_asset, Some("human.skel".into()));
    assert_eq!(restored.bind_shape, [1.0, 2.0, 3.0]);
}

#[test]
fn animation_player_component_trait_type_id() {
    assert_eq!(AnimationPlayer::TYPE_ID, "engine.animation_player");
}

#[test]
fn skeleton_component_trait_type_id() {
    assert_eq!(SkeletonComponent::TYPE_ID, "engine.skeleton");
}

// ── Loader roundtrip tests ─────────────────────────────────────────────

#[test]
fn load_skeleton_roundtrip() {
    let skel = test_skeleton();
    let bytes = bincode::serialize(&skel).unwrap();
    let loaded = load_skeleton(&bytes).unwrap();
    assert_eq!(loaded.joint_count(), 2);
    assert_eq!(loaded.joints[0].name, "root");
    assert_eq!(loaded.joints[1].name, "child");
}

#[test]
fn load_animation_clip_roundtrip() {
    let clip = AnimationClip {
        name: "walk".into(),
        duration: 2.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![Keyframe {
                time: 0.0,
                value: [0.0, 0.0, 0.0],
            }],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![0],
    };
    let bytes = bincode::serialize(&clip).unwrap();
    let loaded = load_animation_clip(&bytes).unwrap();
    assert_eq!(loaded.name, "walk");
    assert!((loaded.duration - 2.0).abs() < 1e-5);
    assert_eq!(loaded.channels.len(), 1);
}

#[test]
fn load_skeleton_invalid_data_returns_error() {
    let result = load_skeleton(&[0xFF, 0xFF, 0xFF]);
    assert!(result.is_err());
}

#[test]
fn load_animation_clip_invalid_data_returns_error() {
    let result = load_animation_clip(&[]);
    assert!(result.is_err());
}

#[test]
fn load_skeleton_rejects_forward_parent_reference() {
    let mut skeleton = test_skeleton();
    skeleton.joints[0].parent_index = Some(1);
    let bytes = bincode::serialize(&skeleton).unwrap();

    let error = load_skeleton(&bytes).unwrap_err();
    assert!(error.contains("parents must precede children"));
}

#[test]
fn load_skeleton_rejects_bind_matrix_count_mismatch() {
    let mut skeleton = test_skeleton();
    skeleton.inverse_bind_matrices.pop();
    let bytes = bincode::serialize(&skeleton).unwrap();

    let error = load_skeleton(&bytes).unwrap_err();
    assert!(error.contains("inverse bind matrices"));
}

#[test]
fn load_animation_clip_rejects_unsorted_keyframes() {
    let clip = AnimationClip {
        name: "bad".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 0,
            translations: vec![
                Keyframe {
                    time: 1.0,
                    value: [0.0; 3],
                },
                Keyframe {
                    time: 0.0,
                    value: [1.0; 3],
                },
            ],
            rotations: vec![],
            scales: vec![],
        }],
        joint_indices: vec![0],
    };
    let bytes = bincode::serialize(&clip).unwrap();

    let error = load_animation_clip(&bytes).unwrap_err();
    assert!(error.contains("not sorted"));
}

#[test]
fn clip_conversion_skips_channels_without_a_matching_joint() {
    let clip = AnimationClip {
        name: "orphan".into(),
        duration: 1.0,
        channels: vec![AnimationChannel {
            joint_index: 99,
            translations: vec![Keyframe {
                time: 0.0,
                value: [1.0, 0.0, 0.0],
            }],
            rotations: vec![Keyframe {
                time: 0.0,
                value: [0.0, 0.0, 0.0, 1.0],
            }],
            scales: vec![Keyframe {
                time: 0.0,
                value: [1.0; 3],
            }],
        }],
        joint_indices: vec![99],
    };

    let runtime = clip_asset_to_runtime(&clip, &[]);
    let empty_skeleton = crate::skeleton::Skeleton::new("empty".into());
    assert!(runtime
        .sample(0.0, &empty_skeleton)
        .local_transforms()
        .is_empty());
}

// ── Extractor tests ────────────────────────────────────────────────────

#[test]
fn skinned_extract_producer_push_and_drain() {
    let producer = SkinnedExtractProducer::new();
    assert_eq!(producer.pending_count(), 0);

    producer.push(PendingSkinnedItem {
        entity: Some("ent-1".into()),
        mesh: "mesh-char".into(),
        material: "mat-skin".into(),
        skeleton: "skel-human".into(),
        bone_palette: vec![IDENTITY_MAT4_4X4; 3],
        world_transform: IDENTITY_MAT4_4X4,
        bounds_min: [-1.0, -1.0, -1.0],
        bounds_max: [1.0, 1.0, 1.0],
        render_layer: "default".into(),
        cast_shadows: true,
        morph_target_set: None,
        morph_weights: Vec::new(),
    });

    assert_eq!(producer.pending_count(), 1);
    let drained = producer.drain();
    assert_eq!(drained.len(), 1);
    assert_eq!(producer.pending_count(), 0);
}

#[test]
fn skinned_extract_producer_produce_injects_into_input() {
    let producer = SkinnedExtractProducer::new();
    producer.push(PendingSkinnedItem {
        entity: None,
        mesh: "mesh-char".into(),
        material: "mat-skin".into(),
        skeleton: "skel-human".into(),
        bone_palette: vec![IDENTITY_MAT4_4X4; 2],
        world_transform: IDENTITY_MAT4_4X4,
        bounds_min: [-1.0, -1.0, -1.0],
        bounds_max: [1.0, 1.0, 1.0],
        render_layer: "default".into(),
        cast_shadows: true,
        morph_target_set: Some("face.morphs".into()),
        morph_weights: vec![0.25, 0.75],
    });

    let mut input = engine_renderer::RenderFrameInput::empty(42);
    producer.produce(&mut input, 42);

    assert_eq!(input.skinned_items.len(), 1);
    assert_eq!(input.skinned_items[0].mesh.id, "mesh-char");
    assert_eq!(input.skinned_items[0].bone_palette.len(), 2);
}

#[test]
fn skinned_extract_replaces_the_matching_static_drawable() {
    let producer = SkinnedExtractProducer::new();
    producer.push(PendingSkinnedItem {
        entity: Some("animated".into()),
        mesh: "mesh-char".into(),
        material: "mat-skin".into(),
        skeleton: "skel-human".into(),
        bone_palette: vec![IDENTITY_MAT4_4X4],
        world_transform: IDENTITY_MAT4_4X4,
        bounds_min: [-1.0; 3],
        bounds_max: [1.0; 3],
        render_layer: "default".into(),
        cast_shadows: true,
        morph_target_set: None,
        morph_weights: Vec::new(),
    });

    let drawable = |entity: &str| engine_renderer::RenderableItem {
        entity: Some(entity.into()),
        mesh: engine_serialize::AssetId::new(format!("mesh-{entity}")),
        material: engine_serialize::AssetId::new("mat-default"),
        world_transform: Mat4::IDENTITY.to_cols_array(),
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "default".into(),
        cast_shadows: true,
        radial_vertex_morph: None,
        triplanar_material_mapping: None,
        sort_key: 0,
    };
    let mut input = engine_renderer::RenderFrameInput::empty(42);
    input.drawables = vec![drawable("animated"), drawable("static")];

    producer.produce(&mut input, 42);

    assert_eq!(input.skinned_items.len(), 1);
    assert_eq!(input.skinned_items[0].entity.as_deref(), Some("animated"));
    assert_eq!(input.drawables.len(), 1);
    assert_eq!(input.drawables[0].entity.as_deref(), Some("static"));
}

// ── Far-from-origin precision (ENG-01 Phase 0, skinned-path twin) ────────
//
