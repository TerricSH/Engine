#![forbid(unsafe_code)]

// ── Internal runtime modules (kept for backward compat) ────────────────
mod skeleton;
pub(crate) mod pose;
mod clip;
// Old player module replaced by the new Gate 10 player below.

// ── Gate 10 modules ────────────────────────────────────────────────────
pub mod assets;
pub mod components;
pub mod loader;
pub mod player;
pub mod extract;
pub mod debug;

// ── Re-exports: old public API (from skeleton, clip, pose) ────────────
pub use skeleton::{AnimationError, BoneIndex, BoneTransform};
pub use pose::Pose;
pub use clip::{AnimationClip as RuntimeAnimationClip, Keyframe as RuntimeKeyframe};
// Old player types are no longer re-exported — use the Gate 10 player instead.

// ── Re-exports: new Gate 10 public API ─────────────────────────────────
pub use assets::{AnimationClip, AnimationChannel, Joint, JointTransform, Keyframe, Skeleton};
pub use components::{AnimationPlayer, SkeletonComponent};
pub use loader::{load_animation_clip, load_skeleton, register_asset_types};
pub use player::{update_animation, AnimationEvaluator};
pub use extract::{PendingSkinnedItem, SkinnedExtractProducer};
pub use debug::{SkeletonDebugDraw, SkeletonDebugInfo};

// ── Registration (Gate 9 extension surfaces) ──────────────────────────

/// Register all animation extensions with the Gate 9 registries.
///
/// This function should be called once at engine startup to register:
/// - `AnimationPlayer` and `SkeletonComponent` ECS component types
/// - `skeleton` and `animation_clip` asset types
/// - `SkinnedExtractProducer` render extension
/// - `SkeletonDebugDraw` debug draw provider
pub fn register_animation_extensions(
    component_reg: &mut engine_scene::registry::ComponentRegistry,
    asset_type_reg: &mut engine_scene::registry::AssetTypeRegistry,
    render_ext_reg: &mut engine_renderer::RenderExtensionRegistry,
    debug_draw_reg: &mut engine_renderer::DebugDrawRegistry,
) {
    // 1) Register ECS components via ComponentRegistry::register_core() path.
    use engine_scene::{Component, ComponentStorageDyn, SparseSet};
    use engine_scene::{ComponentExtension, ComponentMeta};

    fn anim_player_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<AnimationPlayer>::new())
    }
    fn skeleton_comp_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<SkeletonComponent>::new())
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

    // 2) Register asset types.
    loader::register_asset_types(asset_type_reg);

    // 3) Register render extension producer.
    let skinned_producer: Box<dyn engine_renderer::RenderExtensionProducer> =
        Box::new(SkinnedExtractProducer::new());
    render_ext_reg.register(skinned_producer);

    // 4) Register debug draw provider.
    let debug_draw: Box<dyn engine_renderer::DebugDrawProvider> =
        Box::new(SkeletonDebugDraw::new());
    debug_draw_reg.register(debug_draw);
}

#[cfg(test)]
mod tests;
