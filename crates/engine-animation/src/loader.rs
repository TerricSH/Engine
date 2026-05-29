use std::any::Any;

use engine_scene::{AssetTypeExtension, AssetTypeMeta};

use crate::assets::{AnimationClip, Skeleton};

// ---------------------------------------------------------------------------
// Standalone loaders
// ---------------------------------------------------------------------------

/// Deserialize a [`Skeleton`] from bincode-encoded bytes.
pub fn load_skeleton(data: &[u8]) -> Result<Skeleton, String> {
    bincode::deserialize::<Skeleton>(data).map_err(|e| format!("bincode deserialize skeleton: {e}"))
}

/// Deserialize an [`AnimationClip`] from bincode-encoded bytes.
pub fn load_animation_clip(data: &[u8]) -> Result<AnimationClip, String> {
    bincode::deserialize::<AnimationClip>(data)
        .map_err(|e| format!("bincode deserialize animation clip: {e}"))
}

// ---------------------------------------------------------------------------
// Asset type registration (Gate 9 AssetTypeRegistry)
// ---------------------------------------------------------------------------

/// Register "skeleton" and "animation_clip" asset types with the given
/// [`AssetTypeRegistry`], enabling the asset system to cook and load them.
pub fn register_asset_types(asset_type_reg: &mut engine_scene::registry::AssetTypeRegistry) {
    // ── skeleton (.skel) ────────────────────────────────────────────────
    let skeleton_ext = AssetTypeExtension {
        meta: AssetTypeMeta {
            type_id: "skeleton",
            source_extensions: vec!["skel"],
            display_name: "Skeleton",
        },
        cooker: Some(cook_skeleton),
        loader: Some(load_skeleton_typed),
    };
    asset_type_reg.register(skeleton_ext).ok();

    // ── animation_clip (.anim) ──────────────────────────────────────────
    let clip_ext = AssetTypeExtension {
        meta: AssetTypeMeta {
            type_id: "animation_clip",
            source_extensions: vec!["anim"],
            display_name: "Animation Clip",
        },
        cooker: Some(cook_animation_clip),
        loader: Some(load_animation_clip_typed),
    };
    asset_type_reg.register(clip_ext).ok();
}

// ── Cooker fns (passthrough — cooked = source for now) ─────────────────

fn cook_skeleton(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    // Validate by attempting a deserialize, then pass through.
    load_skeleton(source)?;
    output.extend_from_slice(source);
    Ok(())
}

fn cook_animation_clip(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    load_animation_clip(source)?;
    output.extend_from_slice(source);
    Ok(())
}

// ── Typed loader fns ───────────────────────────────────────────────────

fn load_skeleton_typed(cooked: &[u8]) -> Result<Box<dyn Any>, String> {
    let skel = load_skeleton(cooked)?;
    Ok(Box::new(skel))
}

fn load_animation_clip_typed(cooked: &[u8]) -> Result<Box<dyn Any>, String> {
    let clip = load_animation_clip(cooked)?;
    Ok(Box::new(clip))
}
