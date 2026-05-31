// ═════════════════════════════════════════════════════════════════════════
// Tests for engine-animation
// ═════════════════════════════════════════════════════════════════════════

use engine_renderer::{DebugDrawProvider, RenderExtensionProducer};
use engine_scene::Component;
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::FRAC_1_SQRT_2;

use super::*;

// =========================================================================
// Old runtime tests (preserved backward compat)
// =========================================================================

fn old_test_skeleton() -> crate::skeleton::Skeleton {
    let mut skel = crate::skeleton::Skeleton::new("test".to_string());
    let root = skel.add_bone(None, "root".into(), BoneTransform::IDENTITY);
    let hip = skel.add_bone(
        Some(root),
        "hip".into(),
        BoneTransform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    let knee = skel.add_bone(
        Some(hip),
        "knee".into(),
        BoneTransform {
            translation: Vec3::new(0.0, -0.5, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    let _foot = skel.add_bone(
        Some(knee),
        "foot".into(),
        BoneTransform {
            translation: Vec3::new(0.0, -0.5, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    skel
}

/// 2-bone runtime skeleton matching the structure of `test_skeleton()`.
fn test_runtime_skeleton() -> crate::skeleton::Skeleton {
    let mut skel = crate::skeleton::Skeleton::new("test".to_string());
    skel.add_bone(None, "root".into(), BoneTransform::IDENTITY);
    skel.add_bone(
        Some(BoneIndex(0)),
        "child".into(),
        BoneTransform {
            translation: Vec3::new(0.0, 1.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    skel
}

#[test]
fn old_skeleton_bone_count() {
    let skel = old_test_skeleton();
    assert_eq!(skel.bone_count(), 4);
}

#[test]
fn old_skeleton_bone_name() {
    let skel = old_test_skeleton();
    assert_eq!(skel.bone_name(BoneIndex(0)), Some("root"));
    assert_eq!(skel.bone_name(BoneIndex(99)), None);
}

#[test]
fn old_skeleton_parent_child_relationships() {
    let skel = old_test_skeleton();
    assert_eq!(skel.parent_of(BoneIndex(0)), None);
    assert_eq!(skel.parent_of(BoneIndex(1)), Some(BoneIndex(0)));
    assert_eq!(skel.parent_of(BoneIndex(2)), Some(BoneIndex(1)));
    assert_eq!(skel.children_of(BoneIndex(0)), &[BoneIndex(1)]);
    assert_eq!(skel.children_of(BoneIndex(1)), &[BoneIndex(2)]);
    assert_eq!(skel.children_of(BoneIndex(3)), &[] as &[BoneIndex]);
}

#[test]
fn old_rest_pose_is_identity() {
    let skel = old_test_skeleton();
    let pose = skel.rest_pose();
    assert_eq!(pose.local.len(), 4);
    assert_eq!(pose.local[0].translation, Vec3::ZERO);
    assert_eq!(pose.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn old_global_transforms_walk_hierarchy() {
    let skel = old_test_skeleton();
    let pose = skel.rest_pose();
    let global = pose.global_transforms(&skel);
    assert_eq!(global.len(), 4);
    assert_eq!(global[0].translation, Vec3::ZERO);
    assert_eq!(global[1].translation, Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(global[2].translation, Vec3::new(0.0, 0.5, 0.0));
    assert_eq!(global[3].translation, Vec3::new(0.0, 0.0, 0.0));
}

#[test]
fn old_skin_matrices_identity_at_rest() {
    let skel = old_test_skeleton();
    let pose = skel.rest_pose();
    let matrices = pose.skin_matrices(&skel);
    assert_eq!(matrices.len(), 4);
    for (i, m) in matrices.iter().enumerate() {
        let identity = Mat4::IDENTITY;
        let elements = m.to_cols_array();
        let identity_elements = identity.to_cols_array();
        let diff_max = elements
            .iter()
            .zip(identity_elements.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            diff_max < 1e-5,
            "skin matrix {i} should be near identity at rest, max diff {diff_max}"
        );
    }
}

#[test]
fn old_clip_sample_at_zero() {
    let skel = old_test_skeleton();
    let mut clip = RuntimeAnimationClip::new("walk".into(), 2.0);
    clip.add_channel(
        BoneIndex(0),
        vec![RuntimeKeyframe {
            time: 0.0,
            transform: BoneTransform {
                translation: Vec3::new(1.0, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
        }],
    );
    let pose = clip.sample(0.0, &skel);
    assert_eq!(pose.local[0].translation, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(pose.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn old_pose_blend() {
    let skel = old_test_skeleton();
    let a = Pose::new(&skel);
    let mut b = Pose::new(&skel);
    b.local[0].translation = Vec3::new(2.0, 0.0, 0.0);

    let blended = Pose::blend(&a, &b, 0.5);
    assert_eq!(blended.local[0].translation, Vec3::new(1.0, 0.0, 0.0));
}

#[test]
fn old_bone_transform_mul() {
    let a = BoneTransform {
        translation: Vec3::new(1.0, 0.0, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };
    let b = BoneTransform {
        translation: Vec3::new(0.0, 2.0, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };
    let c = a * b;
    assert_eq!(c.translation, Vec3::new(1.0, 2.0, 0.0));
}

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
    });

    let mut input = engine_renderer::RenderFrameInput::empty(42);
    producer.produce(&mut input, 42);

    assert_eq!(input.skinned_items.len(), 1);
    assert_eq!(input.skinned_items[0].mesh.id, "mesh-char");
    assert_eq!(input.skinned_items[0].bone_palette.len(), 2);
}

// ── Debug draw tests ───────────────────────────────────────────────────

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

    register_animation_extensions(
        &mut component_reg,
        &mut asset_type_reg,
        &mut render_ext_reg,
        &mut debug_draw_reg,
    );

    // Components
    assert!(component_reg.is_registered("engine.animation_player"));
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
