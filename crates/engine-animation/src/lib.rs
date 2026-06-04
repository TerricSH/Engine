#![forbid(unsafe_code)]

mod clip;
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
pub mod ik;
pub mod layers;
pub mod loader;
pub mod locomotion_clips;
pub mod player;
pub mod root_motion;
pub mod state_machine;

pub use clip::{AnimationClip as RuntimeAnimationClip, Keyframe as RuntimeKeyframe};
pub use pose::Pose;
pub use skeleton::{AnimationError, BoneIndex, BoneTransform};

pub use assets::{AnimationChannel, AnimationClip, Joint, JointTransform, Keyframe, Skeleton};
pub use blend_space::*;
pub use locomotion_clips::*;

pub use components::{AnimationPlayer, IkTargetComponent, SkeletonComponent};
pub use convert::*;
pub use debug::{SkeletonDebugDraw, SkeletonDebugInfo};
pub use events::{check_event_trigger, AnimEvent, AnimEventCollector, AnimEventDef};
pub use extract::{bridge_skinned_items, PendingSkinnedItem, SkinnedExtractProducer};
pub use foot_ik::*;
pub use ik::{
    solve_pose, solve_pose_multi, IkChain, IkConstraint, IkConstraintSet, IkDebugDraw, IkDebugInfo,
    IkEffector, IkEffectorSpace, IkSolverType,
};
pub use loader::{load_animation_clip, load_skeleton, register_asset_types};
pub use player::{update_animation, AnimationEvaluator};
pub use root_motion::{extract_root_motion, RootMotionApplyTo, RootMotionConfig, RootMotionDelta};

pub use layers::{AnimLayer, LayerBlendMode};
pub use player::update_animation_pipeline;
pub use player::update_animation_sm;
pub use state_machine::{
    AnimParamValue, AnimParameter, AnimStateMachine, AnimStateMachineInstance, AnimationState,
    ConditionOp, StateTransition, TransitionCondition,
};

pub fn register_animation_extensions(
    component_reg: &mut engine_scene::registry::ComponentRegistry,
    asset_type_reg: &mut engine_scene::registry::AssetTypeRegistry,
    render_ext_reg: &mut engine_renderer::RenderExtensionRegistry,
    debug_draw_reg: &mut engine_renderer::DebugDrawRegistry,
) {
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

    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: AnimationPlayer::TYPE_ID,
            display_name: "Animation Player",
            schema_version: (0, 1, 0),
            has_editor: false,
            has_script_binding: false,
        },
        storage_factory: anim_player_storage,
        serialize: None,
        deserialize: None,
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: SkeletonComponent::TYPE_ID,
            display_name: "Skeleton",
            schema_version: (0, 1, 0),
            has_editor: false,
            has_script_binding: false,
        },
        storage_factory: skeleton_comp_storage,
        serialize: None,
        deserialize: None,
    });
    let _ = component_reg.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: IkTargetComponent::TYPE_ID,
            display_name: "IK Target",
            schema_version: (0, 1, 0),
            has_editor: true,
            has_script_binding: false,
        },
        storage_factory: ik_target_storage,
        serialize: None,
        deserialize: None,
    });

    loader::register_asset_types(asset_type_reg);
    let skinned_producer: Box<dyn engine_renderer::RenderExtensionProducer> =
        Box::new(SkinnedExtractProducer::new());
    render_ext_reg.register(skinned_producer);
    let skeleton_draw: Box<dyn engine_renderer::DebugDrawProvider> =
        Box::new(SkeletonDebugDraw::new());
    debug_draw_reg.register(skeleton_draw);
    let ik_draw: Box<dyn engine_renderer::DebugDrawProvider> = Box::new(IkDebugDraw::new());
    debug_draw_reg.register(ik_draw);
}

#[cfg(test)]
mod tests;
