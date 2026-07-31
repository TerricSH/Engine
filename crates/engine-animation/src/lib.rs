#![forbid(unsafe_code)]

mod clip;
mod component_serde;
mod convert;
pub(crate) mod pose;
pub mod skeleton;

pub mod assets;
pub mod blend_space;
pub mod components;
pub mod debug;
pub mod events;
pub mod extract;
pub mod foot_ik;
pub mod gltf_import;
pub mod ik;
pub mod layers;
pub mod loader;
pub mod locomotion_clips;
pub mod player;
pub mod ragdoll;
pub mod root_motion;
pub mod state_machine;

pub use clip::{RuntimeAnimationClip, RuntimeKeyframe};
pub use pose::Pose;
pub use skeleton::{AnimationError, BoneIndex, BoneTransform, RuntimeSkeleton};

pub use assets::{
    AnimationChannel, AnimationClip, AnimationClipAsset, AssetKeyframe, Joint, JointTransform,
    Keyframe, Skeleton, SkeletonAsset,
};
pub use blend_space::*;
pub use locomotion_clips::*;

pub use components::{AnimationPlayer, IkTargetComponent, SkeletonComponent};
pub use convert::*;
pub use debug::{SkeletonDebugDraw, SkeletonDebugInfo};
pub use events::{check_event_trigger, AnimEvent, AnimEventCollector, AnimEventDef};
pub use extract::{bridge_skinned_items, PendingSkinnedItem, SkinnedExtractProducer};
pub use foot_ik::*;
pub use gltf_import::{import_gltf_animation_assets, skeleton_from_gltf_skin, ImportedGltfSkin};
pub use ik::{
    solve_pose, solve_pose_multi, IkChain, IkConstraint, IkConstraintSet, IkDebugDraw, IkDebugInfo,
    IkEffector, IkEffectorSpace, IkSolverType,
};
pub use loader::{load_animation_clip, load_skeleton, register_asset_types};
pub use player::{update_animation, AnimationEvaluator};
pub use ragdoll::{
    ExternalPoseOverride, RagdollBody, RagdollComponent, RagdollConstraint, RagdollJointType,
    RagdollMode, RagdollPart, RagdollPartRole, RagdollShape, MAX_RAGDOLL_BODIES,
    MAX_RAGDOLL_CONSTRAINTS,
};
pub use root_motion::{extract_root_motion, RootMotionApplyTo, RootMotionConfig, RootMotionDelta};

pub use layers::{AnimLayer, LayerBlendMode};
pub use player::update_animation_pipeline;
pub use player::update_animation_sm;
pub use state_machine::{
    AnimParamValue, AnimParameter, AnimStateMachine, AnimStateMachineInstance, AnimationState,
    ConditionOp, StateTransition, TransitionCondition,
};

/// Return the canonical scene field map for a newly authored IK target.
pub fn serialize_ik_target_fields(
    target: &IkTargetComponent,
) -> std::collections::BTreeMap<String, engine_serialize::Value> {
    component_serde::serialize_ik_target(target)
}

/// Shared handles for animation extensions registered with the renderer.
///
/// The update loop writes animation and debug data through these handles;
/// the registries own clones that consume the same queues during extraction.
#[derive(Clone)]
pub struct AnimationExtensionHandles {
    pub skinned_extract: SkinnedExtractProducer,
    pub skeleton_debug: SkeletonDebugDraw,
    pub ik_debug: IkDebugDraw,
}

pub fn register_animation_extensions(
    component_reg: &mut engine_scene::registry::ComponentRegistry,
    asset_type_reg: &mut engine_scene::registry::AssetTypeRegistry,
    render_ext_reg: &mut engine_renderer::RenderExtensionRegistry,
    debug_draw_reg: &mut engine_renderer::DebugDrawRegistry,
) -> AnimationExtensionHandles {
    use engine_scene::{Component, ComponentStorageDyn, SparseSet};
    use engine_scene::{ComponentExtension, ComponentMeta};

    fn anim_player_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<AnimationPlayer>::new())
    }
    fn skeleton_comp_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<SkeletonComponent>::new())
    }
    fn ik_target_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<IkTargetComponent>::new())
    }
    fn ragdoll_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<RagdollComponent>::new())
    }
    fn ragdoll_part_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<RagdollPart>::new())
    }

    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: AnimationPlayer::TYPE_ID,
            display_name: "Animation Player",
            schema_version: (0, 1, 0),
            has_editor: true,
            script_access: engine_scene::ScriptAccess::None,
        },
        storage_factory: anim_player_storage,
        serialize: Some(component_serde::serialize_animation_player),
        deserialize: Some(component_serde::deserialize_animation_player),
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: RagdollPart::TYPE_ID,
            display_name: "Ragdoll Part",
            schema_version: (0, 1, 0),
            has_editor: false,
            script_access: engine_scene::ScriptAccess::None,
        },
        storage_factory: ragdoll_part_storage,
        serialize: Some(ragdoll::serialize_ragdoll_part),
        deserialize: Some(ragdoll::deserialize_ragdoll_part),
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: RagdollComponent::TYPE_ID,
            display_name: "Ragdoll",
            schema_version: (0, 1, 0),
            has_editor: true,
            script_access: engine_scene::ScriptAccess::DedicatedApi,
        },
        storage_factory: ragdoll_storage,
        serialize: Some(ragdoll::serialize_ragdoll),
        deserialize: Some(ragdoll::deserialize_ragdoll),
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: SkeletonComponent::TYPE_ID,
            display_name: "Skeleton",
            schema_version: (0, 1, 0),
            has_editor: false,
            script_access: engine_scene::ScriptAccess::None,
        },
        storage_factory: skeleton_comp_storage,
        serialize: Some(component_serde::serialize_skeleton_component),
        deserialize: Some(component_serde::deserialize_skeleton_component),
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: IkTargetComponent::TYPE_ID,
            display_name: "IK Target",
            schema_version: (0, 1, 0),
            has_editor: true,
            script_access: engine_scene::ScriptAccess::None,
        },
        storage_factory: ik_target_storage,
        serialize: Some(component_serde::serialize_ik_target),
        deserialize: Some(component_serde::deserialize_ik_target),
    });

    loader::register_asset_types(asset_type_reg);
    let skinned_extract = SkinnedExtractProducer::new();
    render_ext_reg.register(Box::new(skinned_extract.clone()));
    let skeleton_debug = SkeletonDebugDraw::new();
    debug_draw_reg.register(Box::new(skeleton_debug.clone()));
    let ik_debug = IkDebugDraw::new();
    debug_draw_reg.register(Box::new(ik_debug.clone()));

    AnimationExtensionHandles {
        skinned_extract,
        skeleton_debug,
        ik_debug,
    }
}

#[cfg(test)]
mod tests;
