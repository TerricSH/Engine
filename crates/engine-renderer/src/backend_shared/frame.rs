use thiserror::Error;

use crate::{ClearFlags, PassGraphOutputMode, Rect, RenderFrameInput};

use super::ui::PixelRect;

#[derive(Clone, Copy, Debug)]
pub struct BackendFrameCapabilities {
    pub allowed_output_modes: &'static [PassGraphOutputMode],
    pub allowed_msaa_samples: &'static [u8],
    pub require_view: bool,
    pub require_matching_view_msaa: bool,
    pub require_matching_viewports: bool,
    pub allowed_clear_flags: &'static [ClearFlags],
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum FrameContractViolation {
    #[error("the requested pass-graph output mode is unsupported")]
    UnsupportedOutputMode,
    #[error("at least one render view is required")]
    MissingView,
    #[error("the requested MSAA configuration is unsupported")]
    UnsupportedMsaa,
    #[error("a render view has an invalid or mismatched normalized viewport")]
    InvalidViewport,
    #[error("a render view uses an unsupported clear mode")]
    UnsupportedClearMode,
}

pub fn validate_backend_frame_contract(
    input: &RenderFrameInput,
    capabilities: BackendFrameCapabilities,
) -> Result<(), FrameContractViolation> {
    if !capabilities
        .allowed_output_modes
        .contains(&input.render_options.pass_graph_config.output_mode)
    {
        return Err(FrameContractViolation::UnsupportedOutputMode);
    }
    if capabilities.require_view && input.views.is_empty() {
        return Err(FrameContractViolation::MissingView);
    }
    if !capabilities
        .allowed_msaa_samples
        .contains(&input.render_options.msaa_samples)
        || (capabilities.require_matching_view_msaa
            && input
                .views
                .iter()
                .any(|view| view.msaa_samples != input.render_options.msaa_samples))
    {
        return Err(FrameContractViolation::UnsupportedMsaa);
    }
    if input.views.iter().any(|view| {
        !view.viewport.is_valid_normalized()
            || !view.viewport_rect_normalized.is_valid_normalized()
            || (capabilities.require_matching_viewports
                && view.viewport != view.viewport_rect_normalized)
    }) {
        return Err(FrameContractViolation::InvalidViewport);
    }
    if input
        .views
        .iter()
        .any(|view| !capabilities.allowed_clear_flags.contains(&view.clear_flags))
    {
        return Err(FrameContractViolation::UnsupportedClearMode);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub scissor: PixelRect,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ViewportPlanError {
    #[error("viewport must be finite, positive, and contained in [0, 1]")]
    InvalidNormalizedRect,
    #[error("viewport surface dimensions must be positive")]
    ZeroSurface,
    #[error("viewport surface dimensions exceed the signed offset range")]
    SurfaceTooLarge,
}

pub fn prepare_normalized_viewport(
    rect: Rect,
    surface_width: u32,
    surface_height: u32,
) -> Result<PreparedViewport, ViewportPlanError> {
    if !rect.is_valid_normalized() {
        return Err(ViewportPlanError::InvalidNormalizedRect);
    }
    if surface_width == 0 || surface_height == 0 {
        return Err(ViewportPlanError::ZeroSurface);
    }
    if surface_width > i32::MAX as u32 || surface_height > i32::MAX as u32 {
        return Err(ViewportPlanError::SurfaceTooLarge);
    }

    let surface_width_f = surface_width as f32;
    let surface_height_f = surface_height as f32;
    let x = rect.min[0] * surface_width_f;
    let y = rect.min[1] * surface_height_f;
    let right = rect.max[0] * surface_width_f;
    let bottom = rect.max[1] * surface_height_f;
    let scissor_left = x.floor().clamp(0.0, surface_width_f) as u32;
    let scissor_top = y.floor().clamp(0.0, surface_height_f) as u32;
    let scissor_right = right.ceil().clamp(0.0, surface_width_f) as u32;
    let scissor_bottom = bottom.ceil().clamp(0.0, surface_height_f) as u32;

    Ok(PreparedViewport {
        x,
        y,
        width: right - x,
        height: bottom - y,
        scissor: PixelRect {
            x: scissor_left as i32,
            y: scissor_top as i32,
            width: scissor_right.saturating_sub(scissor_left),
            height: scissor_bottom.saturating_sub(scissor_top),
        },
    })
}
