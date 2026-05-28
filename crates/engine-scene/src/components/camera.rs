use crate::Component;
use serde::{Deserialize, Serialize};

/// Projection type for a camera.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CameraProjection {
    Perspective,
    Orthographic,
}

/// Camera component per FD-034.
///
/// Provides full camera description including exposure parameters for
/// physically-based rendering.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub projection: CameraProjection,
    pub near: f32,
    pub far: f32,
    /// Vertical field of view in radians (only for Perspective).
    pub fov_y: f32,
    /// Half-height of the orthographic view volume.
    pub ortho_half_height: f32,
    /// Normalized viewport rectangle `[x, y, w, h]`. `None` means full viewport.
    pub viewport_rect: Option<[f32; 4]>,
    /// Bitmask of render layers this camera renders.
    pub render_layer_mask: u32,
    /// Bitmask: 1 = color, 2 = depth.
    pub clear_flags: u8,
    /// Clear colour (RGBA).
    pub clear_color: [f32; 4],
    /// Render priority (higher = later).
    pub priority: i32,
    /// MSAA sample count.
    pub msaa_samples: u8,
    /// Whether to use HDR output.
    pub hdr_output: bool,
    /// Aperture (f-stop) for depth-of-field and exposure.
    pub aperture: f32,
    /// Shutter speed in seconds.
    pub shutter_speed: f32,
    /// ISO sensitivity.
    pub iso: f32,
    /// Exposure value compensation.
    pub ev_compensation: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: CameraProjection::Perspective,
            near: 0.1,
            far: 1000.0,
            fov_y: std::f32::consts::FRAC_PI_4,
            ortho_half_height: 5.0,
            viewport_rect: None,
            render_layer_mask: u32::MAX,
            clear_flags: 3,
            clear_color: [0.02, 0.02, 0.06, 1.0],
            priority: 0,
            msaa_samples: 1,
            hdr_output: false,
            aperture: 16.0,
            shutter_speed: 1.0 / 60.0,
            iso: 100.0,
            ev_compensation: 0.0,
        }
    }
}

impl Component for Camera {
    const TYPE_ID: &'static str = "engine.camera";
}
