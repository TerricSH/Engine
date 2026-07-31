// ═════════════════════════════════════════════════════════════════════════
// Tests for engine-animation
// ═════════════════════════════════════════════════════════════════════════

use engine_renderer::{DebugDrawProvider, RenderExtensionProducer};
use engine_scene::Component;
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::FRAC_1_SQRT_2;

use super::*;

#[test]
fn canonical_asset_and_runtime_names_keep_legacy_type_identity() {
    assert_eq!(
        std::any::TypeId::of::<AnimationClip>(),
        std::any::TypeId::of::<AnimationClipAsset>()
    );
    assert_eq!(
        std::any::TypeId::of::<Keyframe<[f32; 3]>>(),
        std::any::TypeId::of::<AssetKeyframe<[f32; 3]>>()
    );
    assert_eq!(
        std::any::TypeId::of::<Skeleton>(),
        std::any::TypeId::of::<SkeletonAsset>()
    );
    assert_eq!(
        std::any::TypeId::of::<crate::skeleton::Skeleton>(),
        std::any::TypeId::of::<RuntimeSkeleton>()
    );
}

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

include!("asset_runtime.rs");
include!("serialization_extract.rs");
include!("far_origin.rs");
include!("debug_runtime.rs");
include!("pipeline_core.rs");
include!("pipeline_layers.rs");
