use std::sync::Mutex;

use engine_renderer::{
    AxisAlignedBox, BonePaletteLayout, RenderExtensionProducer, RenderFrameInput, SkinnedItem,
};

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
}

/// Render extension producer that injects skinned items into the frame input
/// each frame.
///
/// The animation system pushes [`PendingSkinnedItem`]s into the shared queue,
/// and [`produce`](Self::produce) drains them into
/// [`RenderFrameInput::skinned_items`].
pub struct SkinnedExtractProducer {
    items: Mutex<Vec<PendingSkinnedItem>>,
}

impl SkinnedExtractProducer {
    /// Create a new empty producer.
    pub fn new() -> Self {
        Self {
            items: Mutex::new(Vec::new()),
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
        for item in pending {
            let bone_count = item.bone_palette.len() as u32;

            input.skinned_items.push(SkinnedItem {
                entity: item.entity,
                mesh: engine_serialize::AssetId::new(&item.mesh),
                material: engine_serialize::AssetId::new(&item.material),
                skeleton: engine_serialize::AssetId::new(&item.skeleton),
                bone_palette: item.bone_palette.into_iter().map(mat4x4_to_flat).collect(),
                bone_palette_layout: BonePaletteLayout::Full4x4 { count: bone_count },
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
