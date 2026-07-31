use super::*;

/// Physical render-surface dimensions plus the normalized surface region
/// available to the scene. Camera-authored viewport rectangles are composed
/// inside `output_rect`; this keeps editor embedding on the same extraction
/// path as a full-screen game while producing projection matrices with the
/// actual pixel aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderViewportContext {
    surface_size: [u32; 2],
    output_rect: Rect,
}

/// Read-only description of the renderer's base camera for gameplay tools.
///
/// It uses the same camera ordering, hierarchy resolution, viewport
/// composition and projection functions as renderer extraction, preventing
/// picking code from drifting away from what the player sees.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveCameraView {
    pub entity_id: Option<PersistentId>,
    pub perspective: bool,
    pub position: glam::Vec3,
    pub forward: glam::Vec3,
    pub right: glam::Vec3,
    pub up: glam::Vec3,
    pub viewport_pixels: [f32; 4],
    pub view_projection: glam::Mat4,
    pub inverse_view_projection: glam::Mat4,
}

impl ActiveCameraView {
    /// Build a world-space ray for a top-left-origin surface pixel.
    pub fn screen_ray(&self, point: [f32; 2]) -> Option<(glam::Vec3, glam::Vec3)> {
        let [x, y, width, height] = self.viewport_pixels;
        if width <= 0.0
            || height <= 0.0
            || point[0] < x
            || point[1] < y
            || point[0] > x + width
            || point[1] > y + height
        {
            return None;
        }
        let ndc_x = ((point[0] - x) / width) * 2.0 - 1.0;
        let ndc_y = 1.0 - ((point[1] - y) / height) * 2.0;
        let near = self
            .inverse_view_projection
            .project_point3(glam::Vec3::new(ndc_x, ndc_y, 0.0));
        let far = self
            .inverse_view_projection
            .project_point3(glam::Vec3::new(ndc_x, ndc_y, 1.0));
        if !near.is_finite() || !far.is_finite() {
            return None;
        }
        let origin = if self.perspective {
            self.position
        } else {
            near
        };
        let direction = (far - origin).normalize_or_zero();
        (direction.length_squared() > 0.0).then_some((origin, direction))
    }
}

impl RenderViewportContext {
    pub fn new(surface_width: u32, surface_height: u32, output_rect: Rect) -> Option<Self> {
        (surface_width > 0 && surface_height > 0 && output_rect.is_valid_normalized()).then_some(
            Self {
                surface_size: [surface_width, surface_height],
                output_rect,
            },
        )
    }

    pub const fn surface_size(self) -> [u32; 2] {
        self.surface_size
    }

    pub const fn output_rect(self) -> Rect {
        self.output_rect
    }

    pub(super) fn compose(self, camera_rect: Rect) -> Rect {
        let width = self.output_rect.width();
        let height = self.output_rect.height();
        let compose_x = |value: f32| {
            (self.output_rect.min[0] + value * width)
                .clamp(self.output_rect.min[0], self.output_rect.max[0])
        };
        let compose_y = |value: f32| {
            (self.output_rect.min[1] + value * height)
                .clamp(self.output_rect.min[1], self.output_rect.max[1])
        };
        Rect {
            min: [compose_x(camera_rect.min[0]), compose_y(camera_rect.min[1])],
            max: [compose_x(camera_rect.max[0]), compose_y(camera_rect.max[1])],
        }
    }

    pub(super) fn aspect_ratio(self, viewport: Rect) -> f32 {
        viewport.width() * self.surface_size[0] as f32
            / (viewport.height() * self.surface_size[1] as f32)
    }
}

impl Default for RenderViewportContext {
    fn default() -> Self {
        // Surface-independent callers retain the historical 16:9 behaviour.
        Self {
            surface_size: [16, 9],
            output_rect: Rect::FULL,
        }
    }
}
