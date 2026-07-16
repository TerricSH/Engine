//! Screen-space rendering for the editor transform gizmo.
//!
//! The runtime renderer already accepts UI triangle batches, while its legacy
//! `DebugDrawBuffer` is not part of the windowed editor submission path.  This
//! module projects the same three-axis gizmo used for hit testing and emits a
//! small untextured UI batch that is drawn after the regular editor canvas.

use engine_renderer::{Rect, UiBatch, UiVertex};
use engine_serialize::AssetId;
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::gizmo::{
    gizmo_axis_direction, gizmo_world_scale, project_world_to_screen, GizmoAxis, GizmoMode,
    GizmoSystem, GIZMO_LENGTH, GIZMO_RING_RADIUS, RING_SEGMENTS,
};

const LINE_WIDTH_PX: f32 = 4.0;
const HANDLE_HALF_PX: f32 = 5.0;

/// Build the visible transform-gizmo overlay for a selected entity.
///
/// `gizmo_position` and `gizmo_rotation` must be world-space values and the
/// matrices must be the same active view used to render the scene.  Returning
/// `None` means the gizmo is outside the visible clip volume or the viewport is
/// invalid.
pub fn build_gizmo_ui_batch(
    system: &GizmoSystem,
    gizmo_position: Vec3,
    gizmo_rotation: Quat,
    view: Mat4,
    projection: Mat4,
    viewport_size: Vec2,
) -> Option<UiBatch> {
    if !gizmo_position.is_finite()
        || !gizmo_rotation.is_finite()
        || !view.is_finite()
        || !projection.is_finite()
        || !viewport_size.is_finite()
        || viewport_size.x <= 0.0
        || viewport_size.y <= 0.0
    {
        return None;
    }
    project_world(gizmo_position, view, projection, viewport_size)?;
    let world_scale = gizmo_world_scale(gizmo_position, &view, &projection, viewport_size)?;

    let mut builder = OverlayBuilder::default();
    let axes = [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z];
    for axis in axes {
        let color = if system.drag_axis == Some(axis) {
            [255, 235, 64, 255]
        } else {
            float_color(axis.color())
        };
        let axis_direction = gizmo_axis_direction(system, gizmo_rotation, axis);

        match system.mode {
            GizmoMode::Translate | GizmoMode::Scale => {
                let from = project_world(gizmo_position, view, projection, viewport_size);
                let to = project_world(
                    gizmo_position + axis_direction * GIZMO_LENGTH * world_scale,
                    view,
                    projection,
                    viewport_size,
                );
                if let (Some(from), Some(to)) = (from, to) {
                    let width = if system.drag_axis == Some(axis) {
                        LINE_WIDTH_PX + 2.0
                    } else {
                        LINE_WIDTH_PX
                    };
                    builder.line(from, to, width, color);
                    builder.square(to, HANDLE_HALF_PX, color);
                }
            }
            GizmoMode::Rotate => {
                let normal = axis_direction.normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                let tangent = if normal.x.abs() > 0.9 {
                    Vec3::Y.cross(normal).normalize_or_zero()
                } else {
                    Vec3::X.cross(normal).normalize_or_zero()
                };
                let bitangent = normal.cross(tangent).normalize_or_zero();
                let step = std::f32::consts::TAU / RING_SEGMENTS as f32;
                let width = if system.drag_axis == Some(axis) {
                    LINE_WIDTH_PX + 2.0
                } else {
                    LINE_WIDTH_PX
                };
                let mut previous = None;
                let mut first = None;
                for segment in 0..RING_SEGMENTS {
                    let angle = segment as f32 * step;
                    let point = gizmo_position
                        + tangent * angle.cos() * GIZMO_RING_RADIUS * world_scale
                        + bitangent * angle.sin() * GIZMO_RING_RADIUS * world_scale;
                    let Some(screen) = project_world(point, view, projection, viewport_size) else {
                        previous = None;
                        continue;
                    };
                    first.get_or_insert(screen);
                    if let Some(previous) = previous {
                        builder.line(previous, screen, width, color);
                    }
                    previous = Some(screen);
                }
                if let (Some(previous), Some(first)) = (previous, first) {
                    builder.line(previous, first, width, color);
                }
            }
        }
    }

    (!builder.indices.is_empty()).then(|| UiBatch {
        canvas_id: "editor-transform-gizmo".to_string(),
        z_order: 10_000,
        clip_rect: Rect {
            min: [0.0, 0.0],
            max: viewport_size.to_array(),
        },
        texture: None,
        vertices: builder.vertices,
        indices: builder.indices,
        material: AssetId::new(engine_ui::DEFAULT_UI_MATERIAL),
    })
}

fn project_world(world: Vec3, view: Mat4, projection: Mat4, viewport: Vec2) -> Option<Vec2> {
    project_world_to_screen(world, &view, &projection, viewport)
}

fn float_color(color: [f32; 4]) -> [u8; 4] {
    color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[derive(Default)]
struct OverlayBuilder {
    vertices: Vec<UiVertex>,
    indices: Vec<u32>,
}

impl OverlayBuilder {
    fn line(&mut self, from: Vec2, to: Vec2, width: f32, color: [u8; 4]) {
        let direction = to - from;
        if !direction.is_finite() || direction.length_squared() <= f32::EPSILON {
            return;
        }
        let normal = Vec2::new(-direction.y, direction.x).normalize() * (width * 0.5);
        self.quad(
            [from - normal, from + normal, to + normal, to - normal],
            color,
        );
    }

    fn square(&mut self, center: Vec2, half: f32, color: [u8; 4]) {
        self.quad(
            [
                center + Vec2::new(-half, -half),
                center + Vec2::new(half, -half),
                center + Vec2::new(half, half),
                center + Vec2::new(-half, half),
            ],
            color,
        );
    }

    fn quad(&mut self, points: [Vec2; 4], color: [u8; 4]) {
        let Ok(base) = u32::try_from(self.vertices.len()) else {
            return;
        };
        self.vertices
            .extend(points.into_iter().map(|point| UiVertex {
                position: point.to_array(),
                uv: [0.0, 0.0],
                color,
            }));
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_projection() -> Mat4 {
        Mat4::perspective_lh(60.0_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0)
    }

    #[test]
    fn translate_overlay_contains_three_colored_axes_and_handles() {
        let system = GizmoSystem::new();
        let batch = build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, 5.0),
            Quat::IDENTITY,
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::new(1280.0, 720.0),
        )
        .expect("visible gizmo batch");

        assert_eq!(batch.canvas_id, "editor-transform-gizmo");
        // The Z axis points directly at this test camera, so its projected
        // line has zero length and only its handle is emitted.
        assert_eq!(batch.vertices.len(), 20);
        assert_eq!(batch.indices.len(), 30);
        assert!(batch
            .vertices
            .iter()
            .any(|vertex| vertex.color == [255, 0, 0, 255]));
        assert!(batch
            .vertices
            .iter()
            .any(|vertex| vertex.color == [0, 255, 0, 255]));
        assert!(batch
            .vertices
            .iter()
            .any(|vertex| vertex.color == [0, 0, 255, 255]));
    }

    #[test]
    fn active_axis_is_highlighted() {
        let mut system = GizmoSystem::new();
        system.drag_axis = Some(GizmoAxis::X);
        let batch = build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, 5.0),
            Quat::IDENTITY,
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::new(800.0, 600.0),
        )
        .unwrap();
        assert!(batch
            .vertices
            .iter()
            .any(|vertex| vertex.color == [255, 235, 64, 255]));
    }

    #[test]
    fn global_scale_overlay_uses_entity_local_axes() {
        let mut system = GizmoSystem::new();
        system.mode = GizmoMode::Scale;
        // Scale is local-component based even while the general space toggle
        // says Global, so a rotated local X handle must also be drawn rotated.
        let batch = build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, 5.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::new(1280.0, 720.0),
        )
        .unwrap();
        let red = batch
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [255, 0, 0, 255])
            .map(|vertex| Vec2::from_array(vertex.position))
            .collect::<Vec<_>>();
        let x_span = red.iter().map(|point| point.x).fold(f32::MIN, f32::max)
            - red.iter().map(|point| point.x).fold(f32::MAX, f32::min);
        let y_span = red.iter().map(|point| point.y).fold(f32::MIN, f32::max)
            - red.iter().map(|point| point.y).fold(f32::MAX, f32::min);
        assert!(y_span > x_span * 4.0, "x_span={x_span}, y_span={y_span}");
    }

    #[test]
    fn translate_overlay_keeps_the_same_pixel_span_at_different_depths() {
        let system = GizmoSystem::new();
        let red_span = |depth| {
            let batch = build_gizmo_ui_batch(
                &system,
                Vec3::new(0.0, 0.0, depth),
                Quat::IDENTITY,
                Mat4::IDENTITY,
                visible_projection(),
                Vec2::new(1280.0, 720.0),
            )
            .unwrap();
            let red = batch
                .vertices
                .iter()
                .filter(|vertex| vertex.color == [255, 0, 0, 255])
                .map(|vertex| vertex.position[0]);
            let (minimum, maximum) = red.fold((f32::MAX, f32::MIN), |(minimum, maximum), x| {
                (minimum.min(x), maximum.max(x))
            });
            maximum - minimum
        };

        let near = red_span(5.0);
        let far = red_span(50.0);
        assert!((near - far).abs() < 0.05, "near={near}, far={far}");
        assert!(near > 80.0 && near < 100.0, "unexpected span={near}");
    }

    #[test]
    fn rotate_overlay_is_a_triangle_list() {
        let mut system = GizmoSystem::new();
        system.mode = GizmoMode::Rotate;
        let batch = build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, 5.0),
            Quat::IDENTITY,
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::new(1280.0, 720.0),
        )
        .unwrap();
        assert!(!batch.indices.is_empty());
        assert_eq!(batch.indices.len() % 3, 0);
    }

    #[test]
    fn behind_camera_or_invalid_viewport_is_not_drawn() {
        let system = GizmoSystem::new();
        assert!(build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, -5.0),
            Quat::IDENTITY,
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::new(1280.0, 720.0),
        )
        .is_none());
        assert!(build_gizmo_ui_batch(
            &system,
            Vec3::new(0.0, 0.0, 5.0),
            Quat::IDENTITY,
            Mat4::IDENTITY,
            visible_projection(),
            Vec2::ZERO,
        )
        .is_none());
    }
}
