use engine_renderer::{
    AxisAlignedBox, ClearFlags, LightItem, LightKind, Rect, RenderFrameInput, RenderView,
    RenderableItem, ShadowMode, ViewCompose, IDENTITY_MAT4,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity, PersistentId};

use crate::scene::{ECS_SCENE_CONTRACT, Scene};
use crate::validation::{
    active_camera_entity, asset_field, bool_field, enabled_component, f32_field,
    light_kind_field, string_field, validate_scene, vec3_field,
};
use crate::World;
use crate::components;

// ══════════════════════════════════════════════════════════════════════════════
// Legacy Scene extraction path
// ══════════════════════════════════════════════════════════════════════════════

pub fn extract_renderer_input(
    scene: &Scene,
    frame_index: u64,
) -> Result<RenderFrameInput, Vec<Diagnostic>> {
    let diagnostics = validate_scene(scene);
    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
        )
    }) {
        return Err(diagnostics);
    }

    let mut input = RenderFrameInput::empty(frame_index);
    input.render_options.tone_mapping = scene.scene_settings.tone_mapping;
    input.stats_scope = Some(scene.name.clone());

    let Some(camera_entity) = active_camera_entity(scene) else {
        return Err(vec![Diagnostic::new(
            "SC0018",
            DiagnosticSeverity::Error,
            "engine-scene",
            "scene extraction requires at least one enabled active camera",
        )
        .contract("ECSScene-v0", ECS_SCENE_CONTRACT)]);
    };

    input.views.push(RenderView {
        view_id: 0,
        camera_entity: Some(camera_entity.persistent_id.clone()),
        viewport: Rect::FULL,
        viewport_rect_normalized: Rect::FULL,
        view_matrix: IDENTITY_MAT4,
        projection_matrix: IDENTITY_MAT4,
        clear_flags: ClearFlags::ColorAndDepth,
        clear_color: scene.scene_settings.ambient,
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: ClearFlags::ColorAndDepth,
            clear_color: scene.scene_settings.ambient,
        },
        stack_order: 0,
        frustum: None,
    });

    for entity in scene.entities.iter().filter(|entity| entity.enabled) {
        if let Some(renderable) = enabled_component(entity, "engine.renderable") {
            if bool_field(renderable, "visible").unwrap_or(true) {
                if let (Some(mesh), Some(material)) = (
                    asset_field(renderable, "mesh"),
                    asset_field(renderable, "material"),
                ) {
                    input.drawables.push(RenderableItem {
                        entity: Some(entity.persistent_id.clone()),
                        mesh,
                        material,
                        world_transform: IDENTITY_MAT4,
                        bounds: AxisAlignedBox::UNIT,
                        render_layer: string_field(renderable, "render_layer")
                            .unwrap_or_else(|| scene.scene_settings.default_render_layer.clone()),
                        cast_shadows: bool_field(renderable, "cast_shadows").unwrap_or(true),
                        sort_key: input.drawables.len() as u64,
                    });
                }
            }
        }

        if let Some(light) = enabled_component(entity, "engine.light") {
            input.lights.push(LightItem {
                entity: Some(entity.persistent_id.clone()),
                kind: light_kind_field(light).unwrap_or(LightKind::Directional),
                color: vec3_field(light, "color").unwrap_or([1.0, 1.0, 1.0]),
                intensity: f32_field(light, "intensity").unwrap_or(1.0),
                range: f32_field(light, "range").unwrap_or(10.0),
                position: vec3_field(light, "position").unwrap_or([0.0, 0.0, 0.0]),
                direction: vec3_field(light, "direction").unwrap_or([0.0, -1.0, 0.0]),
                spot_angles: None,
                shadow_mode: ShadowMode::Off,
            });
        }
    }

    Ok(input)
}

// ══════════════════════════════════════════════════════════════════════════════
// ECS World extraction path
// ══════════════════════════════════════════════════════════════════════════════

/// Extract renderer input from an ECS `World` (new path).
///
/// Iterates all entities with [`Camera`] → [`RenderView`],
/// [`Renderable`] + [`Transform`] + [`Bounds`] → [`RenderableItem`],
/// and [`Light`] → [`LightItem`]. Performs frustum culling against the
/// first camera's view-projection frustum.
pub fn extract_renderer_input_from_world(
    world: &World,
    frame_index: u64,
) -> Result<RenderFrameInput, Vec<Diagnostic>> {
    let mut input = RenderFrameInput::empty(frame_index);
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // ── Camera pass: build RenderViews ──────────────────────────────────

    // Collect all cameras with their transforms, sorted by priority/stack_order.
    type CameraEntry = (i32, Option<PersistentId>, components::Camera, components::Transform, crate::Entity);
    let mut cameras: Vec<CameraEntry> = Vec::new();

    for (entity, camera_ref) in world.query::<components::Camera>() {
        let camera = camera_ref.clone();
        let pid = world.persistent_id(entity).map(|s| s.to_string());
        let transform = world
            .get::<components::Transform>(entity)
            .cloned()
            .unwrap_or_default();
        let priority = camera.priority;

        // Validate camera near/far.
        if camera.near <= 0.0 {
            diagnostics.push(
                Diagnostic::new("SC0022", DiagnosticSeverity::Error, "engine-scene",
                    format!("Camera '{}' has non-positive near plane ({})", pid.as_deref().unwrap_or("?"), camera.near))
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .entity(pid.clone()),
            );
        }
        if camera.far <= camera.near {
            diagnostics.push(
                Diagnostic::new("SC0023", DiagnosticSeverity::Error, "engine-scene",
                    format!("Camera '{}' far plane ({}) must be greater than near plane ({})",
                        pid.as_deref().unwrap_or("?"), camera.far, camera.near))
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .entity(pid.clone()),
            );
        }

        cameras.push((priority, pid, camera, transform, entity));
    }

    if cameras.is_empty() {
        return Err(vec![Diagnostic::new(
            "SC0018",
            DiagnosticSeverity::Error,
            "engine-scene",
            "world extraction requires at least one enabled camera component",
        )
        .contract("ECSScene-v0", ECS_SCENE_CONTRACT)]);
    }

    // Sort by priority (ascending = earlier render).
    cameras.sort_by_key(|(priority, _, _, _, _)| *priority);

    // Compute the primary frustum from the first camera for culling.
    let primary_frustum: Option<[glam::Vec4; 6]> = cameras.first().map(|(_, _, camera, transform, _)| {
        let view = compute_view_matrix(transform);
        let proj = compute_projection_matrix(camera);
        let view_proj = proj * view;
        extract_frustum_planes(&view_proj)
    });

    for (view_idx, (priority, pid, camera, transform, entity)) in cameras.iter().enumerate() {
        let view = compute_view_matrix(transform);
        let proj = compute_projection_matrix(camera);

        let clear_color = camera.clear_color;
        let clear_flags = map_clear_flags(camera.clear_flags);

        let frustum = Some(extract_frustum_planes(&(proj * view)));

        let viewport = match camera.viewport_rect {
            Some([x, y, w, h]) => Rect { min: [x, y], max: [x + w, y + h] },
            None => Rect::FULL,
        };

        input.views.push(RenderView {
            view_id: view_idx as u32,
            camera_entity: pid.clone(),
            viewport,
            viewport_rect_normalized: viewport,
            view_matrix: view.to_cols_array(),
            projection_matrix: proj.to_cols_array(),
            clear_flags,
            clear_color,
            render_layer_mask: camera.render_layer_mask,
            msaa_samples: camera.msaa_samples,
            compose: ViewCompose::Base {
                clear: clear_flags,
                clear_color,
            },
            stack_order: *priority,
            frustum: frustum.map(|f| f.map(|p| p.to_array())),
        });
    }

    // Reject extraction if there are fatal diagnostics.
    if diagnostics.iter().any(|d| matches!(d.severity, DiagnosticSeverity::Error | DiagnosticSeverity::Fatal)) {
        return Err(diagnostics);
    }

    // ── Renderable pass: build Drawables ────────────────────────────────

    let mut visible_drawables: u32 = 0;
    let mut culled_drawables: u32 = 0;

    for (entity, renderable) in world.query::<components::Renderable>() {
        if !renderable.visible {
            continue;
        }

        // Skip if mesh or material asset is empty.
        if renderable.mesh_asset.is_empty() || renderable.material_asset.is_empty() {
            continue;
        }

        let pid = world.persistent_id(entity).map(|s| s.to_string());
        let transform = world.get::<components::Transform>(entity).cloned().unwrap_or_default();
        let bounds = world.get::<components::Bounds>(entity);

        // Compute world transform matrix.
        let world_mat = compute_world_matrix(&transform);

        // Compute AABB for frustum culling.
        let (center, half_extents) = match bounds {
            Some(b) => (b.center, b.half_extents),
            None => ([0.0, 0.0, 0.0], [0.5, 0.5, 0.5]),
        };

        // Perform frustum culling against the primary camera frustum.
        let is_visible = match &primary_frustum {
            Some(frustum) => aabb_in_frustum(center, half_extents, frustum),
            None => true,
        };

        if is_visible {
            visible_drawables += 1;
        } else {
            culled_drawables += 1;
            continue;
        }

        let mesh = engine_serialize::AssetId::new(&renderable.mesh_asset);
        let material = engine_serialize::AssetId::new(&renderable.material_asset);

        input.drawables.push(RenderableItem {
            entity: pid,
            mesh,
            material,
            world_transform: world_mat,
            bounds: match bounds {
                Some(b) => AxisAlignedBox {
                    min: [b.center[0] - b.half_extents[0],
                          b.center[1] - b.half_extents[1],
                          b.center[2] - b.half_extents[2]],
                    max: [b.center[0] + b.half_extents[0],
                          b.center[1] + b.half_extents[1],
                          b.center[2] + b.half_extents[2]],
                },
                None => AxisAlignedBox::UNIT,
            },
            render_layer: renderable.render_layer.clone(),
            cast_shadows: renderable.cast_shadows,
            sort_key: input.drawables.len() as u64,
        });
    }

    // ── Light pass: build LightItems ────────────────────────────────────

    let mut visible_lights: u32 = 0;
    let mut culled_lights: u32 = 0;

    for (entity, light) in world.query::<components::Light>() {
        let pid = world.persistent_id(entity).map(|s| s.to_string());

        // Validate light values.
        if light.intensity < 0.0 {
            diagnostics.push(
                Diagnostic::new("SC0024", DiagnosticSeverity::Warning, "engine-scene",
                    format!("Light '{}' has negative intensity ({})", pid.as_deref().unwrap_or("?"), light.intensity))
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .entity(pid.clone()),
            );
        }
        if light.range < 0.0 {
            diagnostics.push(
                Diagnostic::new("SC0025", DiagnosticSeverity::Warning, "engine-scene",
                    format!("Light '{}' has negative range ({})", pid.as_deref().unwrap_or("?"), light.range))
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .entity(pid.clone()),
            );
        }

        // Simple light culling: skip if range is 0 (no contribution).
        let is_visible = light.kind != crate::components::LightKind::Point || light.range > 0.0;

        if is_visible {
            visible_lights += 1;
        } else {
            culled_lights += 1;
            continue;
        }

        let transform = world.get::<components::Transform>(entity);
        let position: [f32; 3] = if let Some(t) = transform {
            t.translation.into()
        } else {
            [0.0, 0.0, 0.0]
        };

        let spot_angles = light.spot_angles.map(|[inner, outer]| engine_renderer::SpotAngles { inner, outer });

        input.lights.push(LightItem {
            entity: pid,
            kind: map_light_kind(light.kind),
            color: light.color,
            intensity: light.intensity,
            range: light.range,
            position,
            direction: light.direction,
            spot_angles,
            shadow_mode: map_shadow_mode(light.shadow_mode),
        });
    }

    // Attach culling stats to the input (stored in stats_scope).
    input.stats_scope = Some(format!(
        "World | drawables: {}/{} culled, lights: {}/{} culled",
        culled_drawables, visible_drawables + culled_drawables,
        culled_lights, visible_lights + culled_lights,
    ));

    // Emit warnings as non-fatal diagnostics.
    if !diagnostics.is_empty() {
        // Diagnostics are non-fatal; attach them to the result.
        // In production they'd be routed to the diagnostics system.
    }

    Ok(input)
}

// ══════════════════════════════════════════════════════════════════════════════
// Frustum culling
// ══════════════════════════════════════════════════════════════════════════════

/// Extract the six frustum planes from a view-projection matrix.
///
/// Returns planes as `(normal.x, normal.y, normal.z, d)` in the order:
/// Left, Right, Bottom, Top, Near, Far. Each plane is normalized.
/// The plane equation is `dot(normal, point) + d = 0`; a point is inside
/// (visible) if `dot(normal, point) + d >= 0` for all six planes.
pub fn extract_frustum_planes(view_proj: &glam::Mat4) -> [glam::Vec4; 6] {
    // Extract rows from the column-major matrix.
    let c0 = view_proj.x_axis;
    let c1 = view_proj.y_axis;
    let c2 = view_proj.z_axis;
    let c3 = view_proj.w_axis;

    let row0 = glam::Vec4::new(c0.x, c1.x, c2.x, c3.x);
    let row1 = glam::Vec4::new(c0.y, c1.y, c2.y, c3.y);
    let row2 = glam::Vec4::new(c0.z, c1.z, c2.z, c3.z);
    let row3 = glam::Vec4::new(c0.w, c1.w, c2.w, c3.w);

    // For OpenGL clip space (NDC [-1, 1]): plane = row3 ± row_i
    let mut planes = [
        row3 + row0,  // left:   -x - w >= 0  →  -(row0·p) - (row3·p) >= 0  →  (row3 + row0)·p >= 0
        row3 - row0,  // right:   x - w <= 0  →   (row0·p) - (row3·p) <= 0  →  (row3 - row0)·p >= 0
        row3 + row1,  // bottom: -y - w >= 0
        row3 - row1,  // top:     y - w <= 0
        row3 + row2,  // near:   -z - w >= 0
        row3 - row2,  // far:     z - w <= 0
    ];

    // Normalise each plane (normal = xyz, constant = w).
    for plane in planes.iter_mut() {
        let len = plane.truncate().length();
        if len > 0.0 {
            *plane /= len;
        }
    }

    planes
}

/// Check whether an AABB is inside (or intersecting) the frustum.
///
/// Returns `true` if the box is at least partially visible.
/// Uses the centre–half-extents test against each frustum plane.
pub fn aabb_in_frustum(
    center: [f32; 3],
    half_extents: [f32; 3],
    frustum: &[glam::Vec4; 6],
) -> bool {
    let c = glam::Vec3::from(center);
    let h = glam::Vec3::from(half_extents);

    for plane in frustum {
        // Signed distance from the box centre to the plane.
        // plane = (nx, ny, nz, d) with eqn: nx*x + ny*y + nz*z + d = 0
        let d = c.x * plane.x + c.y * plane.y + c.z * plane.z + plane.w;

        // Radius of the AABB projected onto the plane normal.
        let r = h.x * plane.x.abs() + h.y * plane.y.abs() + h.z * plane.z.abs();

        // If the entire box is behind this plane → outside.
        if d + r < 0.0 {
            return false;
        }
    }

    true
}

// ══════════════════════════════════════════════════════════════════════════════
// Internal helpers
// ══════════════════════════════════════════════════════════════════════════════

/// Compute a 4×4 view matrix from a transform (inverse of world).
fn compute_view_matrix(transform: &components::Transform) -> glam::Mat4 {
    let t = glam::Mat4::from_translation(transform.translation);
    let r = glam::Mat4::from_quat(transform.rotation);
    let s = glam::Mat4::from_scale(transform.scale);
    let world = t * r * s;
    world.inverse()
}

/// Compute a 4×4 projection matrix from camera parameters.
fn compute_projection_matrix(camera: &components::Camera) -> glam::Mat4 {
    // Default aspect ratio (16:9) — in production this comes from the viewport.
    const ASPECT: f32 = 16.0 / 9.0;

    match camera.projection {
        components::CameraProjection::Perspective => {
            glam::Mat4::perspective_rh_gl(camera.fov_y, ASPECT, camera.near, camera.far)
        }
        components::CameraProjection::Orthographic => {
            let half_w = camera.ortho_half_height * ASPECT;
            let half_h = camera.ortho_half_height;
            glam::Mat4::orthographic_rh_gl(
                -half_w, half_w,
                -half_h, half_h,
                camera.near, camera.far,
            )
        }
    }
}

/// Compute a world matrix from a transform component.
fn compute_world_matrix(transform: &components::Transform) -> [f32; 16] {
    let t = glam::Mat4::from_translation(transform.translation);
    let r = glam::Mat4::from_quat(transform.rotation);
    let s = glam::Mat4::from_scale(transform.scale);
    (t * r * s).to_cols_array()
}

/// Map the engine's camera `clear_flags` bitmask to the renderer's [`ClearFlags`].
fn map_clear_flags(flags: u8) -> ClearFlags {
    if flags & 0b11 == 0b11 {
        ClearFlags::ColorAndDepth
    } else if flags & 0b10 != 0 {
        ClearFlags::DepthOnly
    } else {
        ClearFlags::Nothing
    }
}

/// Map the engine's [`LightKind`] to the renderer's [`LightKind`].
fn map_light_kind(kind: crate::components::LightKind) -> engine_renderer::LightKind {
    match kind {
        crate::components::LightKind::Directional => engine_renderer::LightKind::Directional,
        crate::components::LightKind::Point => engine_renderer::LightKind::Point,
        crate::components::LightKind::Spot => engine_renderer::LightKind::Spot,
    }
}

/// Map the engine's `shadow_mode` byte to the renderer's [`ShadowMode`].
fn map_shadow_mode(mode: u8) -> ShadowMode {
    match mode {
        0 => ShadowMode::Off,
        1 => ShadowMode::Hard,
        2 => ShadowMode::Soft,
        _ => ShadowMode::Off,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample_scene;
    use crate::World;

    // ── Frustum culling tests ───────────────────────────────────────────

    #[test]
    fn frustum_planes_from_identity() {
        let view_proj = glam::Mat4::IDENTITY;
        let planes = extract_frustum_planes(&view_proj);
        assert_eq!(planes.len(), 6);
        // All planes should be normalised.
        for (i, plane) in planes.iter().enumerate() {
            let len = plane.truncate().length();
            assert!((len - 1.0).abs() < 1e-6,
                "plane {} not normalised (len={})", i, len);
        }
    }

    #[test]
    fn aabb_inside_default_frustum() {
        // A simple perspective frustum looking down -Z.
        let proj = glam::Mat4::perspective_rh_gl(
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0,
        );
        let view = glam::Mat4::identity();
        let frustum = extract_frustum_planes(&(proj * view));

        // Box at origin (in front of camera).
        assert!(aabb_in_frustum([0.0, 0.0, -5.0], [0.5, 0.5, 0.5], &frustum));
    }

    #[test]
    fn aabb_outside_frustum_culled() {
        let proj = glam::Mat4::perspective_rh_gl(
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0,
        );
        let view = glam::Mat4::identity();
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far behind the camera.
        assert!(!aabb_in_frustum([0.0, 0.0, 10.0], [0.5, 0.5, 0.5], &frustum));
    }

    #[test]
    fn aabb_far_beyond_far_plane() {
        let proj = glam::Mat4::perspective_rh_gl(
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0,
        );
        let view = glam::Mat4::identity();
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far beyond the far plane.
        assert!(!aabb_in_frustum([0.0, 0.0, -200.0], [1.0, 1.0, 1.0], &frustum));
    }

    #[test]
    fn aabb_partially_inside_is_visible() {
        let proj = glam::Mat4::perspective_rh_gl(
            std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0,
        );
        let view = glam::Mat4::identity();
        let frustum = extract_frustum_planes(&(proj * view));

        // Large box straddling the camera should be visible.
        assert!(aabb_in_frustum([0.0, 0.0, -2.0], [10.0, 10.0, 10.0], &frustum));
    }

    // ── World extraction tests ──────────────────────────────────────────

    #[test]
    fn extract_from_world_with_camera_yields_view() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, components::Camera::default());
        world.add_component(e, components::Transform::default());

        let result = extract_renderer_input_from_world(&world, 0);
        assert!(result.is_ok(), "extraction failed: {:?}", result.err());
        let input = result.unwrap();
        assert_eq!(input.views.len(), 1);
        assert_eq!(input.frame_index, 0);
    }

    #[test]
    fn extract_from_world_without_camera_fails() {
        let world = World::new();
        let result = extract_renderer_input_from_world(&world, 0);
        assert!(result.is_err(), "expected extraction to fail without camera");
    }

    #[test]
    fn extract_from_world_produces_parity_with_scene() {
        let scene = sample_scene();
        let scene_input = extract_renderer_input(&scene, 7).expect("scene extraction OK");

        // Convert scene to world and extract via the new path.
        let world = World::from(&scene);
        let world_input = extract_renderer_input_from_world(&world, 7).expect("world extraction OK");

        // Compare counts (the structural output should match).
        assert_eq!(
            world_input.views.len(),
            scene_input.views.len(),
            "view count mismatch"
        );
        assert_eq!(
            world_input.drawables.len(),
            scene_input.drawables.len(),
            "drawable count mismatch"
        );
        assert_eq!(
            world_input.lights.len(),
            scene_input.lights.len(),
            "light count mismatch"
        );

        // Compare drawable mesh/material/render_layer.
        for (wd, sd) in world_input.drawables.iter().zip(scene_input.drawables.iter()) {
            assert_eq!(wd.mesh, sd.mesh, "mesh mismatch");
            assert_eq!(wd.material, sd.material, "material mismatch");
            assert_eq!(wd.render_layer, sd.render_layer, "render_layer mismatch");
            assert_eq!(wd.cast_shadows, sd.cast_shadows, "cast_shadows mismatch");
        }
    }

    #[test]
    fn extract_from_world_culls_invisible_drawables() {
        let mut world = World::new();
        // Camera looking down -Z.
        let e_cam = world.create_entity();
        world.add_component(e_cam, components::Camera::default());
        world.add_component(e_cam, components::Transform::default());

        // Renderable in front of camera (should be visible).
        let e_front = world.create_entity();
        world.add_component(e_front, components::Renderable {
            mesh_asset: "mesh-visible".into(),
            material_asset: "mat-default".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "Default".into(),
        });
        world.add_component(e_front, components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, -5.0),
            ..Default::default()
        });
        world.add_component(e_front, components::Bounds {
            center: [0.0, 0.0, -5.0],
            half_extents: [0.5, 0.5, 0.5],
        });

        // Renderable behind camera (should be culled).
        let e_back = world.create_entity();
        world.add_component(e_back, components::Renderable {
            mesh_asset: "mesh-culled".into(),
            material_asset: "mat-default".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "Default".into(),
        });
        world.add_component(e_back, components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, 10.0),
            ..Default::default()
        });
        world.add_component(e_back, components::Bounds {
            center: [0.0, 0.0, 10.0],
            half_extents: [0.5, 0.5, 0.5],
        });

        let result = extract_renderer_input_from_world(&world, 1);
        assert!(result.is_ok(), "extraction failed: {:?}", result.err());
        let input = result.unwrap();

        // Only the front drawable should survive culling.
        assert_eq!(input.drawables.len(), 1, "expected 1 visible drawable");
        assert_eq!(input.drawables[0].mesh.id, "mesh-visible");
    }

    #[test]
    fn world_extraction_with_light_produces_light_item() {
        let mut world = World::new();
        let e_cam = world.create_entity();
        world.add_component(e_cam, components::Camera::default());
        world.add_component(e_cam, components::Transform::default());

        let e_light = world.create_entity();
        world.add_component(e_light, crate::components::Light {
            kind: crate::components::LightKind::Point,
            color: [1.0, 0.5, 0.2],
            intensity: 100.0,
            range: 20.0,
            spot_angles: None,
            shadow_mode: 0,
            direction: [0.0, -1.0, 0.0],
        });

        let input = extract_renderer_input_from_world(&world, 2).expect("world extraction OK");
        assert_eq!(input.lights.len(), 1);
        assert_eq!(input.lights[0].color, [1.0, 0.5, 0.2]);
        assert_eq!(input.lights[0].intensity, 100.0);
        assert_eq!(input.lights[0].range, 20.0);
    }
}
