use std::sync::{Arc, Mutex};

use engine_renderer::{
    AxisAlignedBox, BonePaletteLayout, RenderExtensionProducer, RenderFrameInput, SkinnedItem,
};
use engine_scene::{components::Renderable, components::Transform, Entity, World};
use glam::Vec3;

/// Helper: convert a column-major `[[f32;4];4]` to flat `[f32;16]`.
#[inline]
fn mat4x4_to_flat(m: [[f32; 4]; 4]) -> [f32; 16] {
    [
        m[0][0], m[1][0], m[2][0], m[3][0], //
        m[0][1], m[1][1], m[2][1], m[3][1], //
        m[0][2], m[1][2], m[2][2], m[3][2], //
        m[0][3], m[1][3], m[2][3], m[3][3],
    ]
}

/// A pending skinned item waiting to be injected into the render frame input.
///
/// The animation system populates these during the update phase; the
/// [`SkinnedExtractProducer`] drains them during the render extension phase.
pub struct PendingSkinnedItem {
    /// Optional entity identifier (PersistentId).
    pub entity: Option<String>,
    /// Asset ID of the mesh.
    pub mesh: String,
    /// Asset ID of the material.
    pub material: String,
    /// Asset ID of the skeleton.
    pub skeleton: String,
    /// Bone palette matrices in column-major `[[f32;4];4]` form.
    pub bone_palette: Vec<[[f32; 4]; 4]>,
    /// World transform in column-major `[[f32;4];4]` form.
    pub world_transform: [[f32; 4]; 4],
    /// AABB minimum corner.
    pub bounds_min: [f32; 3],
    /// AABB maximum corner.
    pub bounds_max: [f32; 3],
    /// Render layer string.
    pub render_layer: String,
    /// Whether the item casts shadows.
    pub cast_shadows: bool,
    pub morph_target_set: Option<String>,
    pub morph_weights: Vec<f32>,
}

/// Render extension producer that injects skinned items into the frame input
/// each frame.
///
/// The animation system pushes [`PendingSkinnedItem`]s into the shared queue,
/// and [`produce`](Self::produce) drains them into
/// [`RenderFrameInput::skinned_items`].
#[derive(Clone)]
pub struct SkinnedExtractProducer {
    items: Arc<Mutex<Vec<PendingSkinnedItem>>>,
}

impl SkinnedExtractProducer {
    /// Create a new empty producer.
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Push a pending skinned item into the queue.
    ///
    /// Called by the animation system during the update phase.
    pub fn push(&self, item: PendingSkinnedItem) {
        if let Ok(mut guard) = self.items.lock() {
            guard.push(item);
        }
    }

    /// Drain all pending items and return them.
    pub fn drain(&self) -> Vec<PendingSkinnedItem> {
        if let Ok(mut guard) = self.items.lock() {
            std::mem::take(&mut *guard)
        } else {
            Vec::new()
        }
    }

    /// Number of pending items (for diagnostics).
    pub fn pending_count(&self) -> usize {
        self.items.lock().map(|g| g.len()).unwrap_or(0)
    }
}

impl Default for SkinnedExtractProducer {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderExtensionProducer for SkinnedExtractProducer {
    fn name(&self) -> &str {
        "animation_skinned"
    }

    fn produce(&self, input: &mut RenderFrameInput, _frame_index: u64) {
        let pending = self.drain();
        // Core ECS extraction sees every Renderable as a static drawable. A
        // successfully prepared skinned item replaces that representation;
        // retaining both would draw the same entity twice, once frozen in its
        // bind pose and once with the evaluated bone palette.
        input.drawables.retain(|drawable| {
            !pending.iter().any(|item| {
                item.entity.as_deref().is_some()
                    && item.entity.as_deref() == drawable.entity.as_deref()
            })
        });
        for item in pending {
            let bone_count = item.bone_palette.len() as u32;

            input.skinned_items.push(SkinnedItem {
                entity: item.entity,
                mesh: engine_serialize::AssetId::new(&item.mesh),
                material: engine_serialize::AssetId::new(&item.material),
                skeleton: engine_serialize::AssetId::new(&item.skeleton),
                bone_palette: item.bone_palette.into_iter().map(mat4x4_to_flat).collect(),
                bone_palette_layout: BonePaletteLayout::Full4x4 { count: bone_count },
                morph_target_set: item.morph_target_set.map(engine_serialize::AssetId::new),
                morph_weights: item.morph_weights,
                world_transform: mat4x4_to_flat(item.world_transform),
                bounds: AxisAlignedBox {
                    min: item.bounds_min,
                    max: item.bounds_max,
                },
                render_layer: item.render_layer,
                cast_shadows: item.cast_shadows,
                sort_key: 0,
            });
        }
    }
}

/// Bridge: iterate ECS entities with skinning components and queue
/// [`PendingSkinnedItem`]s into the [`SkinnedExtractProducer`].
///
/// Called once per frame during the update phase, after animations advance.
///
/// # Parameters
/// * `world` — ECS world with `Renderable` + `Transform` + `SkeletonComponent` + `AnimationPlayer`
/// * `asset_skeletons` — map of skeleton asset ID to loaded asset `Skeleton`
/// * `clips` — map of clip asset ID to loaded `AnimationClip`
/// * `producer` — the shared `SkinnedExtractProducer` to push items into
/// * `dt` — delta time in seconds
pub fn bridge_skinned_items(
    world: &mut World,
    asset_skeletons: &std::collections::HashMap<String, crate::assets::Skeleton>,
    clips: &std::collections::HashMap<String, crate::AnimationClip>,
    producer: &SkinnedExtractProducer,
    dt: f32,
) {
    use crate::{AnimationPlayer, IkTargetComponent, SkeletonComponent};

    // ENG-01 camera-relative rendering: skinned items bypass engine-scene's
    // world-transform resolve, so extraction cannot shift them. Apply the
    // same `-origin` translation here or skinned meshes render offset from
    // the camera-relative frame by the origin magnitude.
    let relative_origin = engine_scene::camera_relative_render_origin(world);

    let clip_list: Vec<(&str, crate::AnimationClip)> = clips
        .iter()
        .map(|(id, clip)| (id.as_str(), clip.clone()))
        .collect();

    // Collect entities first to avoid borrow conflicts with get_mut
    let entities: Vec<Entity> = world
        .query::<Renderable>()
        .filter(|(_, r)| r.visible && !r.mesh_asset.is_empty())
        .map(|(e, _)| e)
        .collect();

    for entity in entities {
        // Clone all needed data before mutable borrow on world
        let Some(renderable) = world.get::<Renderable>(entity).cloned() else {
            continue;
        };
        let skeleton_component = world.get::<SkeletonComponent>(entity).cloned();
        let skel_asset_id = skeleton_component
            .as_ref()
            .and_then(|component| component.skeleton_asset.clone());
        let Some(skel_asset_id) = skel_asset_id else {
            continue;
        };
        let Some(asset_skel) = asset_skeletons.get(&skel_asset_id) else {
            continue;
        };
        let transform = world.get::<Transform>(entity).cloned().unwrap_or_default();
        let ik = world.get::<IkTargetComponent>(entity).cloned();

        // Convert to runtime skeleton for animation evaluation
        let runtime_skel = crate::skeleton::Skeleton::from_asset(asset_skel);

        // Advance animation player (mutable borrow) and compute bone palette
        let (bone_palette, bone_positions) =
            if let Some(player) = world.get_mut::<AnimationPlayer>(entity) {
                let mut state_machine = player.state_machine.take();
                let palette = crate::player::update_animation_pipeline(
                    player,
                    &mut state_machine,
                    &clip_list,
                    &runtime_skel,
                    ik.as_ref(),
                    dt,
                );
                player.state_machine = state_machine;
                (palette, player.cached_bone_positions.clone())
            } else {
                let pose = runtime_skel.rest_pose();
                let positions = pose
                    .global_transforms(&runtime_skel)
                    .iter()
                    .map(|transform| transform.translation.to_array())
                    .collect();
                let palette = pose
                    .skin_matrices(&runtime_skel)
                    .iter()
                    .map(|matrix| matrix.to_cols_array_2d())
                    .collect();
                (palette, positions)
            };

        let world_mat = glam::Mat4::from_translation(transform.translation)
            * glam::Mat4::from_quat(transform.rotation)
            * glam::Mat4::from_scale(transform.scale);
        let world_mat = match relative_origin {
            Some(origin) => glam::Mat4::from_translation(-origin) * world_mat,
            None => world_mat,
        };

        // Compute a conservative local-space AABB from animated joint
        // positions. Skin matrices include inverse-bind transforms and are not
        // valid joint positions (at rest they are commonly all identity).
        let (bounds_min, bounds_max) = {
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for position in &bone_positions {
                let position = Vec3::from(*position);
                if position.is_finite() {
                    min = min.min(position);
                    max = max.max(position);
                }
            }
            let half_extents = skeleton_component
                .as_ref()
                .map(|component| Vec3::from(component.bind_shape).abs())
                .filter(|extents| extents.is_finite())
                .unwrap_or(Vec3::splat(0.5))
                .max(Vec3::splat(0.01));
            if min.x == f32::MAX {
                ((-half_extents).to_array(), half_extents.to_array())
            } else {
                (
                    (min - half_extents).to_array(),
                    (max + half_extents).to_array(),
                )
            }
        };

        producer.push(PendingSkinnedItem {
            entity: world.persistent_id(entity).map(|s| s.to_string()),
            mesh: renderable.mesh_asset.clone(),
            material: renderable.material_asset.clone(),
            skeleton: skel_asset_id.clone(),
            bone_palette,
            world_transform: world_mat.to_cols_array_2d(),
            bounds_min,
            bounds_max,
            render_layer: renderable.render_layer.clone(),
            cast_shadows: renderable.cast_shadows,
            morph_target_set: skeleton_component
                .as_ref()
                .and_then(|component| component.morph_target_set.clone()),
            morph_weights: skeleton_component
                .as_ref()
                .map(|component| component.morph_weights.clone())
                .unwrap_or_default(),
        });
    }
}
