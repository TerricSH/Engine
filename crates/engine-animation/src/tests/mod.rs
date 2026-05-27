use super::*;
use glam::{Mat4, Quat, Vec3};
use std::sync::Arc;

fn test_skeleton() -> Skeleton {
    let mut skel = Skeleton::new("test".to_string());
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

#[test]
fn skeleton_bone_count() {
    let skel = test_skeleton();
    assert_eq!(skel.bone_count(), 4);
}

#[test]
fn skeleton_bone_name() {
    let skel = test_skeleton();
    assert_eq!(skel.bone_name(BoneIndex(0)), Some("root"));
    assert_eq!(skel.bone_name(BoneIndex(99)), None);
}

#[test]
fn skeleton_parent_child_relationships() {
    let skel = test_skeleton();
    assert_eq!(skel.parent_of(BoneIndex(0)), None);
    assert_eq!(skel.parent_of(BoneIndex(1)), Some(BoneIndex(0)));
    assert_eq!(skel.parent_of(BoneIndex(2)), Some(BoneIndex(1)));
    assert_eq!(skel.children_of(BoneIndex(0)), &[BoneIndex(1)]);
    assert_eq!(skel.children_of(BoneIndex(1)), &[BoneIndex(2)]);
    assert_eq!(skel.children_of(BoneIndex(3)), &[] as &[BoneIndex]);
}

#[test]
fn rest_pose_is_identity() {
    let skel = test_skeleton();
    let pose = skel.rest_pose();
    assert_eq!(pose.local.len(), 4);
    // The root bone has IDENTITY rest transform.
    assert_eq!(pose.local[0].translation, Vec3::ZERO);
    assert_eq!(pose.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn global_transforms_walk_hierarchy() {
    let skel = test_skeleton();
    let pose = skel.rest_pose();
    let global = pose.global_transforms(&skel);
    assert_eq!(global.len(), 4);
    // Root: identity
    assert_eq!(global[0].translation, Vec3::ZERO);
    // Hip: root * hip_local = (0,0,0) + rot * (1 * (0,1,0)) = (0,1,0)
    assert_eq!(global[1].translation, Vec3::new(0.0, 1.0, 0.0));
    // Knee: hip_global * knee_local = (0,1,0) + (0,-0.5,0) = (0,0.5,0)
    assert_eq!(global[2].translation, Vec3::new(0.0, 0.5, 0.0));
    // Foot: knee_global * foot_local = (0,0.5,0) + (0,-0.5,0) = (0,0,0)
    assert_eq!(global[3].translation, Vec3::new(0.0, 0.0, 0.0));
}

#[test]
fn skin_matrices_identity_at_rest() {
    let skel = test_skeleton();
    let pose = skel.rest_pose();
    let matrices = pose.skin_matrices(&skel);
    // At rest pose, each skin matrix should be identity because
    // current_global * inverse(rest_global) = I.
    assert_eq!(matrices.len(), 4);
    for (i, m) in matrices.iter().enumerate() {
        // At rest, skin matrix should equal the identity matrix.
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
fn clip_sample_at_zero() {
    let skel = test_skeleton();
    let mut clip = AnimationClip::new("walk".into(), 2.0);
    // Animate root translation at time 0.
    clip.add_channel(
        BoneIndex(0),
        vec![Keyframe {
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
    // Non-animated bones stay at rest.
    assert_eq!(pose.local[1].translation, Vec3::new(0.0, 1.0, 0.0));
}

#[test]
fn player_plays_and_stops() {
    let skel = test_skeleton();
    let clip = Arc::new(AnimationClip::new("idle".into(), 1.0));

    let mut player = AnimationPlayer::new();
    assert!(!player.is_playing());

    player.play(clip, 0.0);
    assert!(player.is_playing());
    assert_eq!(player.current_clip_name(), Some("idle"));

    let _pose = player.update(0.5, &skel);
    assert!(player.is_playing());
}

#[test]
fn animator_basic_lifecycle() {
    let skel = test_skeleton();
    let clip = Arc::new(AnimationClip::new("run".into(), 1.5));

    let mut anim = Animator::new();
    anim.play(clip, 0.2);
    let _pose = anim.update(1.0, &skel);
}

#[test]
fn pose_blend() {
    let skel = test_skeleton();
    let a = Pose::new(&skel);
    let mut b = Pose::new(&skel);
    b.local[0].translation = Vec3::new(2.0, 0.0, 0.0);

    let blended = Pose::blend(&a, &b, 0.5);
    assert_eq!(blended.local[0].translation, Vec3::new(1.0, 0.0, 0.0));
}

#[test]
fn bone_transform_mul() {
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
