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
        UiElementKind::Image { texture, .. } => Some(texture.clone()),
        UiElementKind::NineSlice { texture, .. } => Some(texture.clone()),
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

/// Append a rectangular border (4 quads) to a batch.
pub(crate) fn add_border(batch: &mut UiBatch, rect: &UiRect, thickness: f32, color: [u8; 4]) {
    if thickness <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let t = thickness.min(rect.width / 2.0).min(rect.height / 2.0);

    // Top edge
    add_quad(
        batch,
        &UiRect::new(rect.x, rect.y, rect.width, t),
        &[0.0, 0.0],
        &[1.0, 1.0],
        &color,
    );
    // Bottom edge
    add_quad(
        batch,
        &UiRect::new(rect.x, rect.y + rect.height - t, rect.width, t),
        &[0.0, 0.0],
        &[1.0, 1.0],
        &color,
    );
    // Left edge (excluding corners already drawn)
    add_quad(
        batch,
        &UiRect::new(rect.x, rect.y + t, t, rect.height - 2.0 * t),
        &[0.0, 0.0],
        &[1.0, 1.0],
        &color,
    );
    // Right edge
    add_quad(
        batch,
        &UiRect::new(
            rect.x + rect.width - t,
            rect.y + t,
            t,
            rect.height - 2.0 * t,
        ),
        &[0.0, 0.0],
        &[1.0, 1.0],
        &color,
    );
}

/// Append a nine-slice quad set (9 quads) to a batch.
///
/// The `border` UiRect represents [left, top, right, bottom] border sizes in
/// both destination pixels and source UV fractions (see [`UiElementKind::NineSlice`]).
pub(crate) fn add_nine_slice(batch: &mut UiBatch, rect: &UiRect, border: &UiRect, color: [u8; 4]) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }

    let l = border.x;
    let t = border.y;
    let r = border.width;
    let b = border.height;

    let inner_w = rect.width - l - r;
    let inner_h = rect.height - t - b;
    let inner_x = rect.x + l;
    let inner_y = rect.y + t;

    let uv_zero = [0.0_f32, 0.0];

    // Corner UVs
    let uv_tl_max = [l, t];
    let uv_tr_min = [1.0 - r, 0.0];
    let uv_tr_max = [1.0, t];
    let uv_bl_min = [0.0, 1.0 - b];
    let uv_bl_max = [l, 1.0];
    let uv_br_min = [1.0 - r, 1.0 - b];
    let uv_br_max = [1.0, 1.0];

    // Edge UVs
    let uv_tm_min = [l, 0.0];
    let uv_tm_max = [1.0 - r, t];
    let uv_bm_min = [l, 1.0 - b];
    let uv_bm_max = [1.0 - r, 1.0];
    let uv_lm_min = [0.0, t];
    let uv_lm_max = [l, 1.0 - b];
    let uv_rm_min = [1.0 - r, t];
    let uv_rm_max = [1.0, 1.0 - b];

    // Center UV
    let uv_c_min = [l, t];
    let uv_c_max = [1.0 - r, 1.0 - b];

    macro_rules! maybe_add {
        ($x:expr, $y:expr, $w:expr, $h:expr, $umin:expr, $umax:expr) => {
            if $w > 0.0 && $h > 0.0 {
                add_quad(batch, &UiRect::new($x, $y, $w, $h), &$umin, &$umax, &color);
            }
        };
    }

    // Corners
    maybe_add!(rect.x, rect.y, l, t, uv_zero, uv_tl_max);
    maybe_add!(rect.x + rect.width - r, rect.y, r, t, uv_tr_min, uv_tr_max);
    maybe_add!(rect.x, rect.y + rect.height - b, l, b, uv_bl_min, uv_bl_max);
    maybe_add!(
        rect.x + rect.width - r,
        rect.y + rect.height - b,
        r,
        b,
        uv_br_min,
        uv_br_max
    );

    // Edges
    maybe_add!(inner_x, rect.y, inner_w, t, uv_tm_min, uv_tm_max);
    maybe_add!(
        inner_x,
        rect.y + rect.height - b,
        inner_w,
        b,
        uv_bm_min,
        uv_bm_max
    );
    maybe_add!(rect.x, inner_y, l, inner_h, uv_lm_min, uv_lm_max);
    maybe_add!(
        rect.x + rect.width - r,
        inner_y,
        r,
        inner_h,
        uv_rm_min,
        uv_rm_max
    );

    // Center
    maybe_add!(inner_x, inner_y, inner_w, inner_h, uv_c_min, uv_c_max);
}
