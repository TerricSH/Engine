use engine_renderer::{UiBatch, UiVertex, Vec2};
use engine_serialize::AssetId;

use crate::types::{UiElementKind, UiRect};
use crate::Color;

// ---------------------------------------------------------------------------
// Batch-building helpers
// ---------------------------------------------------------------------------

/// Returns the texture [`AssetId`] for element kinds that use one, or `None`.
pub(crate) fn element_kind_texture(kind: &UiElementKind) -> Option<AssetId> {
    match kind {
        UiElementKind::Image { texture_id, .. } => Some(AssetId::new(texture_id.clone())),
        _ => None,
    }
}

/// Convert [`Color`] into the `[u8; 4]` format used by [`UiVertex`].
pub(crate) fn color_to_array(color: Color) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
}

/// Append a single quad (4 vertices, 6 indices) to a batch.
pub(crate) fn add_quad(
    batch: &mut UiBatch,
    rect: &UiRect,
    uv_min: &Vec2,
    uv_max: &Vec2,
    color: &[u8; 4],
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let base = batch.vertices.len() as u32;
    let left = rect.x;
    let right = rect.x + rect.width;
    let top = rect.y;
    let bottom = rect.y + rect.height;

    batch.vertices.push(UiVertex {
        position: [left, top],
        uv: [uv_min[0], uv_min[1]],
        color: *color,
    });
    batch.vertices.push(UiVertex {
        position: [right, top],
        uv: [uv_max[0], uv_min[1]],
        color: *color,
    });
    batch.vertices.push(UiVertex {
        position: [right, bottom],
        uv: [uv_max[0], uv_max[1]],
        color: *color,
    });
    batch.vertices.push(UiVertex {
        position: [left, bottom],
        uv: [uv_min[0], uv_max[1]],
        color: *color,
    });

    batch.indices.push(base);
    batch.indices.push(base + 1);
    batch.indices.push(base + 2);
    batch.indices.push(base);
    batch.indices.push(base + 2);
    batch.indices.push(base + 3);
}
