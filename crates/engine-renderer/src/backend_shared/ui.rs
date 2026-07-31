use thiserror::Error;

use crate::{Rect, UiBatch};

pub const UI_VERTEX_STRIDE: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedUiDraw {
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub texture_id: Option<String>,
    pub scissor: PixelRect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparedUiOverlay {
    pub vertex_bytes: Vec<u8>,
    pub draws: Vec<PreparedUiDraw>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum UiPlanError {
    #[error("UI batch {batch} index count {index_count} is not a triangle-list multiple")]
    IndexCountNotTriangleList { batch: usize, index_count: usize },
    #[error("UI batch {batch} has a non-finite clip rectangle")]
    NonFiniteClipRect { batch: usize },
    #[error("UI batch {batch} has an inverted clip rectangle")]
    InvertedClipRect { batch: usize },
    #[error("UI vertex offset exceeds u32")]
    VertexOffsetOverflow,
    #[error("UI batch {batch} index {index} is outside {vertex_count} vertices")]
    VertexOutOfBounds {
        batch: usize,
        index: u32,
        vertex_count: usize,
    },
    #[error("UI batch {batch} vertex {index} contains non-finite data")]
    NonFiniteVertex { batch: usize, index: u32 },
    #[error("UI batch {batch} has too many indices")]
    TooManyIndices { batch: usize },
}

pub fn first_missing_ui_texture(
    batches: &[UiBatch],
    mut texture_exists: impl FnMut(&str) -> bool,
) -> Option<&str> {
    batches
        .iter()
        .filter_map(|batch| batch.texture.as_ref())
        .map(|asset_id| asset_id.id.as_str())
        .find(|texture_id| !texture_exists(texture_id))
}

fn ui_scissor(
    batch: usize,
    rect: Rect,
    width: u32,
    height: u32,
) -> Result<Option<PixelRect>, UiPlanError> {
    if rect
        .min
        .into_iter()
        .chain(rect.max)
        .any(|value| !value.is_finite())
    {
        return Err(UiPlanError::NonFiniteClipRect { batch });
    }
    if rect.max[0] < rect.min[0] || rect.max[1] < rect.min[1] {
        return Err(UiPlanError::InvertedClipRect { batch });
    }

    let max_x = width.min(i32::MAX as u32) as f32;
    let max_y = height.min(i32::MAX as u32) as f32;
    let x0 = rect.min[0].floor().clamp(0.0, max_x) as i32;
    let y0 = rect.min[1].floor().clamp(0.0, max_y) as i32;
    let x1 = rect.max[0].ceil().clamp(0.0, max_x) as i32;
    let y1 = rect.max[1].ceil().clamp(0.0, max_y) as i32;
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    Ok(Some(PixelRect {
        x: x0,
        y: y0,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    }))
}

/// Expand indexed UI batches into one portable, non-indexed 32-byte stream.
///
/// Draw order is intentionally preserved because texture and clip state are
/// observable parts of the UI contract.
pub fn prepare_ui_overlay(
    batches: &[UiBatch],
    width: u32,
    height: u32,
) -> Result<PreparedUiOverlay, UiPlanError> {
    let mut prepared = PreparedUiOverlay::default();
    for (batch_index, batch) in batches.iter().enumerate() {
        if batch.indices.len() % 3 != 0 {
            return Err(UiPlanError::IndexCountNotTriangleList {
                batch: batch_index,
                index_count: batch.indices.len(),
            });
        }
        let Some(scissor) = ui_scissor(batch_index, batch.clip_rect, width, height)? else {
            continue;
        };
        if batch.indices.is_empty() {
            continue;
        }
        let first_vertex = u32::try_from(prepared.vertex_bytes.len() / UI_VERTEX_STRIDE)
            .map_err(|_| UiPlanError::VertexOffsetOverflow)?;
        for &index in &batch.indices {
            let vertex =
                batch
                    .vertices
                    .get(index as usize)
                    .ok_or(UiPlanError::VertexOutOfBounds {
                        batch: batch_index,
                        index,
                        vertex_count: batch.vertices.len(),
                    })?;
            if vertex
                .position
                .into_iter()
                .chain(vertex.uv)
                .any(|value| !value.is_finite())
            {
                return Err(UiPlanError::NonFiniteVertex {
                    batch: batch_index,
                    index,
                });
            }
            for value in vertex.position.into_iter().chain(vertex.uv) {
                prepared
                    .vertex_bytes
                    .extend_from_slice(&value.to_ne_bytes());
            }
            for channel in vertex.color {
                prepared
                    .vertex_bytes
                    .extend_from_slice(&(f32::from(channel) / 255.0).to_ne_bytes());
            }
        }
        prepared.draws.push(PreparedUiDraw {
            first_vertex,
            vertex_count: batch
                .indices
                .len()
                .try_into()
                .map_err(|_| UiPlanError::TooManyIndices { batch: batch_index })?,
            texture_id: batch.texture.as_ref().map(|texture| texture.id.clone()),
            scissor,
        });
    }
    Ok(prepared)
}
