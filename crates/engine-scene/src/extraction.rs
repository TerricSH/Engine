use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use engine_renderer::{
    AxisAlignedBox, BlendMode, ClearFlags, ExtractionStats, LightItem, Rect, RenderFrameInput,
    RenderView, RenderableItem, ShadowMode, ViewCompose,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, PersistentId};

use crate::components;
use crate::scene::ECS_SCENE_CONTRACT;
use crate::World;

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

    fn compose(self, camera_rect: Rect) -> Rect {
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

    fn aspect_ratio(self, viewport: Rect) -> f32 {
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

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// Canonical World extraction path
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

/// Extract renderer input from the canonical ECS [`World`].
///
/// Iterates all entities with [`Camera`] 鈫?[`RenderView`],
/// [`Renderable`] + [`Transform`] + [`Bounds`] 鈫?[`RenderableItem`],
/// and [`Light`] 鈫?[`LightItem`]. Performs frustum culling against the
/// first camera's view-projection frustum.
pub fn extract_renderer_input_from_world(
    world: &World,
    frame_index: u64,
) -> Result<RenderFrameInput, Vec<Diagnostic>> {
    extract_renderer_input_from_world_with_viewport(
        world,
        frame_index,
        RenderViewportContext::default(),
    )
}

/// Extract renderer input for a concrete surface viewport.
///
/// This is the same canonical World extractor used by
/// [`extract_renderer_input_from_world`]. It additionally supplies the host
/// surface geometry required to compose camera viewports and calculate their
/// real projection aspect ratios.
pub fn extract_renderer_input_from_world_with_viewport(
    world: &World,
    frame_index: u64,
    viewport_context: RenderViewportContext,
) -> Result<RenderFrameInput, Vec<Diagnostic>> {
    let mut input = RenderFrameInput::empty(frame_index);
    input.render_options.tone_mapping = world.scene_settings().tone_mapping;
    input.render_options.pass_graph_config = world.scene_settings().pass_graph_config.clone();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Resolve every Transform once up front. Hierarchy corruption is fatal:
    // rendering a partially-local, partially-world-space frame is less safe
    // than rejecting the frame with a structured diagnostic.
    let world_matrices = resolve_world_transforms(world)?;

    // 鈹€鈹€ Camera pass: build RenderViews 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

    // Collect all cameras with their transforms, sorted by priority/stack_order.
    type CameraEntry = (
        i32,
        Option<PersistentId>,
        components::Camera,
        glam::Mat4,
        crate::Entity,
    );
    let mut cameras: Vec<CameraEntry> = Vec::new();

    for (entity, camera_ref) in world.query::<components::Camera>() {
        let camera = camera_ref.clone();
        let pid = world.persistent_id(entity).map(|s| s.to_string());
        let world_matrix = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let priority = camera.priority;

        // Validate camera near/far.
        if camera.near <= 0.0 {
            diagnostics.push(
                Diagnostic::new(
                    "SC0022",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' has non-positive near plane ({})",
                        pid.as_deref().unwrap_or("?"),
                        camera.near
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .entity(pid.clone()),
            );
        }
        if camera.far <= camera.near {
            diagnostics.push(
                Diagnostic::new(
                    "SC0023",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' far plane ({}) must be greater than near plane ({})",
                        pid.as_deref().unwrap_or("?"),
                        camera.far,
                        camera.near
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .entity(pid.clone()),
            );
        }
        if let Some([x, y, width, height]) = camera.viewport_rect {
            let viewport_is_valid = [x, y, width, height].into_iter().all(f32::is_finite)
                && x >= 0.0
                && y >= 0.0
                && width > 0.0
                && height > 0.0
                && x + width <= 1.0
                && y + height <= 1.0;
            if !viewport_is_valid {
                diagnostics.push(
                    Diagnostic::new(
                        "SC0031",
                        DiagnosticSeverity::Error,
                        "engine-scene",
                        format!(
                            "Camera '{}' viewport_rect must be finite, positive, and contained in [0, 1]",
                            pid.as_deref().unwrap_or("?")
                        ),
                    )
                    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                    .path("components.engine.camera.viewport_rect")
                    .entity(pid.clone()),
                );
            }
        }
        if !matches!(camera.msaa_samples, 1 | 2 | 4 | 8) {
            diagnostics.push(
                Diagnostic::new(
                    "SC0032",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' msaa_samples must be one of 1, 2, 4, or 8 (got {})",
                        pid.as_deref().unwrap_or("?"),
                        camera.msaa_samples
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("components.engine.camera.msaa_samples")
                .entity(pid.clone()),
            );
        }
        if physical_exposure_ev100(
            camera.aperture,
            camera.shutter_speed,
            camera.iso,
            camera.ev_compensation,
        )
        .is_none()
        {
            diagnostics.push(
                Diagnostic::new(
                    "SC0034",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' exposure requires finite aperture/shutter/ISO values greater than zero and finite EV compensation",
                        pid.as_deref().unwrap_or("?")
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("components.engine.camera.exposure")
                .entity(pid.clone()),
            );
        }

        let determinant = world_matrix.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            diagnostics.push(
                Diagnostic::new(
                    "SC0029",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' has a non-invertible world transform",
                        pid.as_deref().unwrap_or("?")
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("components.engine.transform")
                .entity(pid.clone()),
            );
        }

        let projection =
            compute_projection_matrix(&camera, effective_camera_aspect(&camera, viewport_context));
        if !projection.is_finite() {
            diagnostics.push(
                Diagnostic::new(
                    "SC0030",
                    DiagnosticSeverity::Error,
                    "engine-scene",
                    format!(
                        "Camera '{}' produces a non-finite projection matrix",
                        pid.as_deref().unwrap_or("?")
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .path("components.engine.camera")
                .entity(pid.clone()),
            );
        }

        cameras.push((priority, pid, camera, world_matrix, entity));
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

    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
        )
    }) {
        return Err(diagnostics);
    }

    // The active camera is the single Base view in v0. Other cameras become
    // deterministic overlays. Worlds authored directly (without scene
    // settings) use the lowest-priority camera as their Base view.
    let active_camera = world.scene_settings().active_camera.as_deref();
    cameras.sort_by(|left, right| {
        let left_is_active = left.1.as_deref() == active_camera;
        let right_is_active = right.1.as_deref() == active_camera;
        right_is_active
            .cmp(&left_is_active)
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.cmp(&right.1))
    });
    let base_camera = &cameras[0].2;
    input.render_options.msaa_samples = base_camera.msaa_samples;
    input.render_options.exposure_ev100 = physical_exposure_ev100(
        base_camera.aperture,
        base_camera.shutter_speed,
        base_camera.iso,
        base_camera.ev_compensation,
    );

    let camera_frustums: Vec<[glam::Vec4; 6]> = cameras
        .iter()
        .map(|(_, _, camera, world_matrix, _)| {
            extract_frustum_planes(
                &(compute_projection_matrix(
                    camera,
                    effective_camera_aspect(camera, viewport_context),
                ) * compute_view_matrix(*world_matrix)),
            )
        })
        .collect();

    for (view_idx, (priority, pid, camera, world_matrix, _entity)) in cameras.iter().enumerate() {
        let view = compute_view_matrix(*world_matrix);
        let proj =
            compute_projection_matrix(camera, effective_camera_aspect(camera, viewport_context));

        let clear_color = camera.clear_color;
        let clear_flags = map_clear_flags(camera.clear_flags);

        let frustum = Some(extract_frustum_planes(&(proj * view)));

        let viewport = effective_camera_viewport(camera, viewport_context);

        let (view_clear_flags, compose) = if view_idx == 0 {
            (
                clear_flags,
                ViewCompose::Base {
                    clear: clear_flags,
                    clear_color,
                },
            )
        } else {
            (
                ClearFlags::Nothing,
                ViewCompose::Overlay {
                    base_view_id: 0,
                    blend_mode: BlendMode::Replace,
                },
            )
        };

        input.views.push(RenderView {
            view_id: view_idx as u32,
            camera_entity: pid.clone(),
            viewport,
            viewport_rect_normalized: viewport,
            view_matrix: view.to_cols_array(),
            projection_matrix: proj.to_cols_array(),
            clear_flags: view_clear_flags,
            clear_color,
            render_layer_mask: camera.render_layer_mask,
            msaa_samples: camera.msaa_samples,
            compose,
            stack_order: *priority,
            frustum: frustum.map(|f| f.map(|p| p.to_array())),
        });
    }

    // 鈹€鈹€ Renderable pass: build Drawables 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

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
        let bounds = world.get::<components::Bounds>(entity);

        let world_matrix = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let world_mat = world_matrix.to_cols_array();
        let (center, half_extents, world_bounds) = transform_bounds_to_world(bounds, world_matrix);

        let Some(layer_bit) = render_layer_bit(&renderable.render_layer) else {
            diagnostics.push(unknown_render_layer_diagnostic(
                &renderable.render_layer,
                pid.clone(),
            ));
            continue;
        };
        let layer_mask = 1_u32 << layer_bit;
        let is_visible =
            cameras
                .iter()
                .zip(&camera_frustums)
                .any(|((_, _, camera, _, _), frustum)| {
                    camera.render_layer_mask & layer_mask != 0
                        && aabb_in_frustum(center, half_extents, frustum)
                });

        if is_visible {
            visible_drawables += 1;
        } else {
            culled_drawables += 1;
            continue;
        }

        let mesh = engine_serialize::AssetId::new(&renderable.mesh_asset);
        let material = engine_serialize::AssetId::new(&renderable.material_asset);
        let sk = batch_sort_key(&material, &mesh);

        input.drawables.push(RenderableItem {
            entity: pid,
            mesh,
            material,
            world_transform: world_mat,
            bounds: world_bounds,
            render_layer: renderable.render_layer.clone(),
            cast_shadows: renderable.cast_shadows,
            sort_key: sk,
        });
    }

    // Sort drawables by (material, mesh) for efficient batching.
    input.drawables.sort_by_key(|d| d.sort_key);

    // 鈹€鈹€ Light pass: build LightItems 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

    let mut visible_lights: u32 = 0;
    let mut culled_lights: u32 = 0;

    for (entity, light) in world.query::<components::Light>() {
        let pid = world.persistent_id(entity).map(|s| s.to_string());

        // Validate light values.
        if light.intensity < 0.0 {
            diagnostics.push(
                Diagnostic::new(
                    "SC0024",
                    DiagnosticSeverity::Warning,
                    "engine-scene",
                    format!(
                        "Light '{}' has negative intensity ({})",
                        pid.as_deref().unwrap_or("?"),
                        light.intensity
                    ),
                )
                .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
                .entity(pid.clone()),
            );
        }
        if light.range < 0.0 {
            diagnostics.push(
                Diagnostic::new(
                    "SC0025",
                    DiagnosticSeverity::Warning,
                    "engine-scene",
                    format!(
                        "Light '{}' has negative range ({})",
                        pid.as_deref().unwrap_or("?"),
                        light.range
                    ),
                )
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

        let world_matrix = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let position = world_matrix.transform_point3(glam::Vec3::ZERO).to_array();
        let direction = world_matrix
            .transform_vector3(glam::Vec3::from(light.direction))
            .normalize_or_zero()
            .to_array();

        let spot_angles = light
            .spot_angles
            .map(|[inner, outer]| engine_renderer::SpotAngles { inner, outer });

        input.lights.push(LightItem {
            entity: pid,
            kind: map_light_kind(light.kind),
            color: light.color,
            intensity: light.intensity,
            range: light.range,
            position,
            direction,
            spot_angles,
            shadow_mode: map_shadow_mode(light.shadow_mode),
        });
    }

    // Attach culling stats to the input (stored in stats_scope).
    input.stats_scope = Some(format!(
        "World | drawables: {}/{} culled, lights: {}/{} culled",
        culled_drawables,
        visible_drawables + culled_drawables,
        culled_lights,
        visible_lights + culled_lights,
    ));
    input.extraction_stats = Some(ExtractionStats {
        visible_drawables,
        culled_drawables,
        visible_lights,
        culled_lights,
    });

    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.severity,
            DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
        )
    }) {
        return Err(diagnostics);
    }

    Ok(input)
}

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// Frustum culling
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

/// Resolve a registered v0 render-layer name to its frozen bit index.
/// Matching is ASCII case-insensitive; `Opaque` is retained as a compatibility
/// alias for `Default`. User-reserved slots are named `User0` through `User26`.
pub fn render_layer_bit(name: &str) -> Option<u32> {
    let normalized = name.trim().to_ascii_lowercase().replace(['_', '-'], "");
    match normalized.as_str() {
        "default" | "opaque" => Some(0),
        "transparent" => Some(1),
        "ui" => Some(2),
        "postprocess" => Some(3),
        "debug" => Some(4),
        _ => normalized
            .strip_prefix("user")
            .and_then(|suffix| suffix.parse::<u32>().ok())
            .filter(|index| *index <= 26)
            .map(|index| index + 5),
    }
}

fn unknown_render_layer_diagnostic(layer: &str, entity: Option<PersistentId>) -> Diagnostic {
    Diagnostic::new(
        "SC0033",
        DiagnosticSeverity::Error,
        "engine-scene",
        format!(
            "unknown render layer '{layer}'; expected Default, Transparent, UI, PostProcess, Debug, or User0..User26"
        ),
    )
    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
    .path("components.engine.renderable.render_layer")
    .entity(entity)
}

/// Extract the six frustum planes from a right-handed, zero-to-one-depth
/// view-projection matrix.
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

    // X/Y use [-w, w]. Z uses [0, w], matching Vulkan and glam's RH helpers.
    let mut planes = [
        row3 + row0, // left:   -x - w >= 0  鈫? -(row0路p) - (row3路p) >= 0  鈫? (row3 + row0)路p >= 0
        row3 - row0, // right:   x - w <= 0  鈫?  (row0路p) - (row3路p) <= 0  鈫? (row3 - row0)路p >= 0
        row3 + row1, // bottom: -y - w >= 0
        row3 - row1, // top:     y - w <= 0
        row2,        // near for zero-to-one clip depth: z >= 0
        row3 - row2, // far: z <= w
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
/// Uses the center/half-extents test against each frustum plane.
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

        // If the entire box is behind this plane 鈫?outside.
        if d + r < 0.0 {
            return false;
        }
    }

    true
}

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// Internal helpers
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

/// Resolve local TRS values into one cached world matrix per entity.
fn resolve_world_transforms(
    world: &World,
) -> Result<HashMap<crate::Entity, glam::Mat4>, Vec<Diagnostic>> {
    let entries: Vec<_> = world
        .query_all::<components::Transform>()
        .map(|(entity, transform)| (entity, transform.clone()))
        .collect();
    let transforms: HashMap<_, _> = entries.iter().cloned().collect();
    let mut local_matrices = HashMap::with_capacity(entries.len());
    let mut world_matrices = HashMap::with_capacity(entries.len());
    let mut invalid = HashSet::new();
    let mut diagnostics = Vec::new();

    for (entity, transform) in &entries {
        let local = glam::Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );
        if local.is_finite() {
            local_matrices.insert(*entity, local);
        } else {
            invalid.insert(*entity);
            diagnostics.push(non_finite_transform_diagnostic(
                world,
                *entity,
                "local TRS contains non-finite values",
            ));
        }
    }

    for (start, _) in &entries {
        if world_matrices.contains_key(start) || invalid.contains(start) {
            continue;
        }

        let mut chain = Vec::new();
        let mut positions = HashMap::new();
        let mut cursor = *start;

        let base_world = loop {
            if let Some(cached) = world_matrices.get(&cursor) {
                break Some(*cached);
            }
            if invalid.contains(&cursor) {
                break None;
            }
            if let Some(&cycle_start) = positions.get(&cursor) {
                let cycle = &chain[cycle_start..];
                diagnostics.push(transform_cycle_diagnostic(world, cycle));
                invalid.extend(chain.iter().copied());
                break None;
            }

            // An alive parent without a Transform is a valid identity root.
            let Some(transform) = transforms.get(&cursor) else {
                world_matrices.insert(cursor, glam::Mat4::IDENTITY);
                break Some(glam::Mat4::IDENTITY);
            };

            positions.insert(cursor, chain.len());
            chain.push(cursor);

            let Some(parent) = transform.parent else {
                break Some(glam::Mat4::IDENTITY);
            };
            if !world.is_alive(parent) {
                diagnostics.push(invalid_parent_diagnostic(world, cursor, parent));
                invalid.extend(chain.iter().copied());
                break None;
            }
            cursor = parent;
        };

        let Some(mut accumulated) = base_world else {
            invalid.extend(chain.iter().copied());
            continue;
        };

        let root_to_leaf: Vec<_> = chain.into_iter().rev().collect();
        for (index, entity) in root_to_leaf.iter().copied().enumerate() {
            let Some(local) = local_matrices.get(&entity) else {
                invalid.extend(root_to_leaf[index..].iter().copied());
                break;
            };
            accumulated *= *local;
            if !accumulated.is_finite() {
                diagnostics.push(non_finite_transform_diagnostic(
                    world,
                    entity,
                    "resolved world matrix is non-finite",
                ));
                invalid.extend(root_to_leaf[index..].iter().copied());
                break;
            }
            world_matrices.insert(entity, accumulated);
        }
    }

    if diagnostics.is_empty() {
        Ok(world_matrices)
    } else {
        Err(diagnostics)
    }
}

fn invalid_parent_diagnostic(
    world: &World,
    child: crate::Entity,
    parent: crate::Entity,
) -> Diagnostic {
    let child_pid = world.persistent_id(child).map(str::to_owned);
    let mut diagnostic = Diagnostic::new(
        "SC0026",
        DiagnosticSeverity::Error,
        "engine-scene",
        format!(
            "Transform parent {}:{} for entity {}:{} is stale or belongs to another World/domain",
            parent.index(),
            parent.generation(),
            child.index(),
            child.generation()
        ),
    )
    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
    .path("components.engine.transform.parent")
    .entity(child_pid);
    diagnostic
        .fields
        .insert("reason".to_string(), "stale_or_foreign_domain".to_string());
    diagnostic
        .fields
        .insert("parent_index".to_string(), parent.index().to_string());
    diagnostic.fields.insert(
        "parent_generation".to_string(),
        parent.generation().to_string(),
    );
    diagnostic
}

fn transform_cycle_diagnostic(world: &World, cycle: &[crate::Entity]) -> Diagnostic {
    let owner = cycle.first().copied();
    let cycle_path = cycle
        .iter()
        .chain(cycle.first())
        .map(|entity| format!("{}:{}", entity.index(), entity.generation()))
        .collect::<Vec<_>>()
        .join(" -> ");
    let owner_pid = owner.and_then(|entity| world.persistent_id(entity).map(str::to_owned));
    let mut diagnostic = Diagnostic::new(
        "SC0027",
        DiagnosticSeverity::Error,
        "engine-scene",
        format!("Transform parent chain contains a cycle: {cycle_path}"),
    )
    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
    .path("components.engine.transform.parent")
    .entity(owner_pid);
    diagnostic
        .fields
        .insert("reason".to_string(), "parent_cycle".to_string());
    diagnostic.fields.insert("cycle".to_string(), cycle_path);
    diagnostic
}

fn non_finite_transform_diagnostic(
    world: &World,
    entity: crate::Entity,
    reason: &str,
) -> Diagnostic {
    let pid = world.persistent_id(entity).map(str::to_owned);
    let mut diagnostic = Diagnostic::new(
        "SC0028",
        DiagnosticSeverity::Error,
        "engine-scene",
        format!(
            "Transform for entity {}:{} is invalid: {reason}",
            entity.index(),
            entity.generation()
        ),
    )
    .contract("ECSScene-v0", ECS_SCENE_CONTRACT)
    .path("components.engine.transform")
    .entity(pid);
    diagnostic
        .fields
        .insert("reason".to_string(), reason.to_string());
    diagnostic
}

fn compute_view_matrix(world_matrix: glam::Mat4) -> glam::Mat4 {
    world_matrix.inverse()
}

/// Compute a 4 x 4 projection matrix from camera parameters.
fn compute_projection_matrix(camera: &components::Camera, aspect: f32) -> glam::Mat4 {
    // Default aspect ratio (16:9); in production this comes from the viewport.
    match camera.projection {
        components::CameraProjection::Perspective => {
            glam::Mat4::perspective_rh(camera.fov_y, aspect, camera.near, camera.far)
        }
        components::CameraProjection::Orthographic => {
            let half_w = camera.ortho_half_height * aspect;
            let half_h = camera.ortho_half_height;
            glam::Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, camera.near, camera.far)
        }
    }
}

fn authored_camera_viewport(camera: &components::Camera) -> Rect {
    let viewport = match camera.viewport_rect {
        Some([x, y, width, height]) => Rect {
            min: [x, y],
            max: [x + width, y + height],
        },
        None => Rect::FULL,
    };
    // Invalid authored values are diagnosed before projection. Use a stable
    // fallback here so validation errors do not also manufacture NaN matrices.
    if viewport.is_valid_normalized() {
        viewport
    } else {
        Rect::FULL
    }
}

fn effective_camera_viewport(camera: &components::Camera, context: RenderViewportContext) -> Rect {
    context.compose(authored_camera_viewport(camera))
}

fn effective_camera_aspect(camera: &components::Camera, context: RenderViewportContext) -> f32 {
    context.aspect_ratio(effective_camera_viewport(camera, context))
}

/// Compute a sort key that groups drawables by material then mesh.
fn batch_sort_key(material: &AssetId, mesh: &AssetId) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    material.hash(&mut hasher);
    mesh.hash(&mut hasher);
    hasher.finish()
}

/// Transform a local-space AABB into a conservative world-space AABB.
fn transform_bounds_to_world(
    bounds: Option<&components::Bounds>,
    world_matrix: glam::Mat4,
) -> ([f32; 3], [f32; 3], AxisAlignedBox) {
    let (local_center, local_half_extents) = match bounds {
        Some(bounds) => (
            glam::Vec3::from(bounds.center),
            glam::Vec3::from(bounds.half_extents).abs(),
        ),
        None => (glam::Vec3::ZERO, glam::Vec3::splat(0.5)),
    };
    let center = world_matrix.transform_point3(local_center);
    let x_axis = world_matrix.x_axis.truncate().abs();
    let y_axis = world_matrix.y_axis.truncate().abs();
    let z_axis = world_matrix.z_axis.truncate().abs();
    let half_extents = x_axis * local_half_extents.x
        + y_axis * local_half_extents.y
        + z_axis * local_half_extents.z;
    let min = center - half_extents;
    let max = center + half_extents;
    (
        center.to_array(),
        half_extents.to_array(),
        AxisAlignedBox {
            min: min.to_array(),
            max: max.to_array(),
        },
    )
}

/// Map the engine's camera `clear_flags` bitmask to the renderer's [`ClearFlags`].
fn map_clear_flags(flags: u8) -> ClearFlags {
    if flags & 0b100 != 0 {
        ClearFlags::Skybox
    } else if flags & 0b11 == 0b11 {
        ClearFlags::ColorAndDepth
    } else if flags & 0b10 != 0 {
        ClearFlags::DepthOnly
    } else {
        ClearFlags::Nothing
    }
}

/// Compute physical EV100 for the renderer-wide exposure override.
///
/// RendererInput-v0 has no per-view exposure field yet, so the sorted base
/// camera supplies the override. Positive compensation brightens the image by
/// lowering the effective EV.
fn physical_exposure_ev100(
    aperture: f32,
    shutter_seconds: f32,
    iso: f32,
    ev_compensation: f32,
) -> Option<f32> {
    if !aperture.is_finite()
        || aperture <= 0.0
        || !shutter_seconds.is_finite()
        || shutter_seconds <= 0.0
        || !iso.is_finite()
        || iso <= 0.0
        || !ev_compensation.is_finite()
    {
        return None;
    }

    let aperture = f64::from(aperture);
    let shutter_seconds = f64::from(shutter_seconds);
    let iso = f64::from(iso);
    let ev100 = ((aperture * aperture / shutter_seconds) * (100.0 / iso)).log2()
        - f64::from(ev_compensation);
    (ev100.is_finite() && ev100 >= f32::MIN as f64 && ev100 <= f32::MAX as f64)
        .then_some(ev100 as f32)
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

// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲
// Tests
// 鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲鈺愨晲

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;

    fn assert_mat4_approx(actual: &[f32; 16], expected: glam::Mat4) {
        for (index, (actual, expected)) in actual
            .iter()
            .zip(expected.to_cols_array().iter())
            .enumerate()
        {
            assert!(
                (actual - expected).abs() <= 1.0e-5,
                "matrix element {index} differs: actual={actual}, expected={expected}"
            );
        }
    }

    fn add_default_camera(world: &mut World) -> crate::Entity {
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(camera, components::Transform::default());
        camera
    }

    // 鈹€鈹€ Frustum culling tests 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

    #[test]
    fn frustum_planes_from_identity() {
        let view_proj = glam::Mat4::IDENTITY;
        let planes = extract_frustum_planes(&view_proj);
        assert_eq!(planes.len(), 6);
        // All planes should be normalised.
        for (i, plane) in planes.iter().enumerate() {
            let len = plane.truncate().length();
            assert!(
                (len - 1.0).abs() < 1e-6,
                "plane {} not normalised (len={})",
                i,
                len
            );
        }
    }

    #[test]
    fn aabb_inside_default_frustum() {
        // A simple perspective frustum looking down -Z.
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box at origin (in front of camera).
        assert!(aabb_in_frustum([0.0, 0.0, -5.0], [0.5, 0.5, 0.5], &frustum));
    }

    #[test]
    fn aabb_outside_frustum_culled() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far behind the camera.
        assert!(!aabb_in_frustum(
            [0.0, 0.0, 10.0],
            [0.5, 0.5, 0.5],
            &frustum
        ));
    }

    #[test]
    fn aabb_far_beyond_far_plane() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far beyond the far plane.
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -200.0],
            [1.0, 1.0, 1.0],
            &frustum
        ));
    }

    #[test]
    fn aabb_partially_inside_is_visible() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Large box straddling the camera should be visible.
        assert!(aabb_in_frustum(
            [0.0, 0.0, -2.0],
            [10.0, 10.0, 10.0],
            &frustum
        ));
    }

    #[test]
    fn zero_to_one_frustum_uses_exact_near_and_far_planes() {
        let near = 1.0;
        let far = 10.0;
        let projection = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
        let frustum = extract_frustum_planes(&projection);

        assert!(aabb_in_frustum(
            [0.0, 0.0, -(near + 0.01)],
            [0.0; 3],
            &frustum
        ));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(near - 0.1)],
            [0.0; 3],
            &frustum
        ));
        assert!(aabb_in_frustum(
            [0.0, 0.0, -(far - 0.01)],
            [0.0; 3],
            &frustum
        ));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(far + 0.1)],
            [0.0; 3],
            &frustum
        ));

        let near_clip = projection * glam::Vec4::new(0.0, 0.0, -near, 1.0);
        let far_clip = projection * glam::Vec4::new(0.0, 0.0, -far, 1.0);
        assert!((near_clip.z / near_clip.w).abs() <= 1.0e-6);
        assert!((far_clip.z / far_clip.w - 1.0).abs() <= 1.0e-6);
    }

    #[test]
    fn zero_to_one_orthographic_frustum_uses_exact_near_and_far_planes() {
        let near = 2.0;
        let far = 6.0;
        let projection = glam::Mat4::orthographic_rh(-2.0, 2.0, -2.0, 2.0, near, far);
        let frustum = extract_frustum_planes(&projection);

        assert!(aabb_in_frustum([0.0, 0.0, -near], [0.0; 3], &frustum));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(near - 0.1)],
            [0.0; 3],
            &frustum
        ));
        assert!(aabb_in_frustum([0.0, 0.0, -far], [0.0; 3], &frustum));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(far + 0.1)],
            [0.0; 3],
            &frustum
        ));
    }

    // 鈹€鈹€ World extraction tests 鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€鈹€

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
    fn concrete_surface_context_composes_base_and_overlay_viewports_and_projection_aspects() {
        let mut world = World::new();
        let base = world.create_entity();
        let base_camera = components::Camera::default();
        world.add_component(base, base_camera.clone());
        world.add_component(base, components::Transform::default());

        let overlay = world.create_entity();
        world.add_component(
            overlay,
            components::Camera {
                viewport_rect: Some([0.25, 0.0, 0.5, 1.0]),
                priority: 1,
                ..base_camera.clone()
            },
        );
        world.add_component(overlay, components::Transform::default());

        let output = Rect {
            min: [0.2, 0.25],
            max: [0.8, 0.75],
        };
        let context = RenderViewportContext::new(1000, 800, output).unwrap();
        let input = extract_renderer_input_from_world_with_viewport(&world, 4, context).unwrap();

        assert_eq!(input.views[0].viewport_rect_normalized, output);
        assert_eq!(input.views[0].viewport, output);
        let overlay_viewport = input.views[1].viewport_rect_normalized;
        for (actual, expected) in overlay_viewport
            .min
            .into_iter()
            .chain(overlay_viewport.max)
            .zip([0.35, 0.25, 0.65, 0.75])
        {
            assert!((actual - expected).abs() <= 1.0e-6);
        }
        assert_mat4_approx(
            &input.views[0].projection_matrix,
            glam::Mat4::perspective_rh(
                base_camera.fov_y,
                600.0 / 400.0,
                base_camera.near,
                base_camera.far,
            ),
        );
        assert_mat4_approx(
            &input.views[1].projection_matrix,
            glam::Mat4::perspective_rh(
                base_camera.fov_y,
                300.0 / 400.0,
                base_camera.near,
                base_camera.far,
            ),
        );
    }

    #[test]
    fn world_extraction_preserves_scene_render_options_and_camera_exposure() {
        let mut world = World::new();
        world.scene_settings.tone_mapping = engine_renderer::ToneMapping::Reinhard;
        world.scene_settings.pass_graph_config.enabled = false;
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                aperture: 2.0,
                shutter_speed: 0.25,
                iso: 100.0,
                ev_compensation: 1.0,
                msaa_samples: 4,
                ..Default::default()
            },
        );
        world.add_component(camera, components::Transform::default());

        let input = extract_renderer_input_from_world(&world, 9).expect("valid extraction");

        assert_eq!(
            input.render_options.tone_mapping,
            engine_renderer::ToneMapping::Reinhard
        );
        assert!(!input.render_options.pass_graph_config.enabled);
        assert_eq!(input.render_options.msaa_samples, 4);
        assert_eq!(input.views[0].msaa_samples, 4);
        assert!((input.render_options.exposure_ev100.unwrap() - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn world_extraction_rejects_invalid_physical_exposure() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                aperture: 0.0,
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("invalid exposure must not reach tone mapping");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0034"));
    }

    #[test]
    fn extract_from_world_without_camera_fails() {
        let world = World::new();
        let result = extract_renderer_input_from_world(&world, 0);
        assert!(
            result.is_err(),
            "expected extraction to fail without camera"
        );
    }

    #[test]
    fn registered_render_layers_have_stable_bits() {
        assert_eq!(render_layer_bit("Default"), Some(0));
        assert_eq!(render_layer_bit("opaque"), Some(0));
        assert_eq!(render_layer_bit("Transparent"), Some(1));
        assert_eq!(render_layer_bit("UI"), Some(2));
        assert_eq!(render_layer_bit("post-process"), Some(3));
        assert_eq!(render_layer_bit("Debug"), Some(4));
        assert_eq!(render_layer_bit("User0"), Some(5));
        assert_eq!(render_layer_bit("User26"), Some(31));
        assert_eq!(render_layer_bit("User27"), None);
        assert_eq!(render_layer_bit("unregistered"), None);
    }

    #[test]
    fn camera_layer_mask_culls_non_matching_drawables() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                render_layer_mask: 1 << 1,
                ..Default::default()
            },
        );

        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-layered".into(),
                material_asset: "material-layered".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -5.0),
                ..Default::default()
            },
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("valid extraction");
        assert!(input.drawables.is_empty());
        assert_eq!(input.extraction_stats.unwrap().culled_drawables, 1);
    }

    #[test]
    fn unregistered_render_layer_fails_closed() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-unknown-layer".into(),
                material_asset: "material-unknown-layer".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "GameplaySecret".into(),
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("unknown layers must not be rendered implicitly");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0033"));
    }

    #[test]
    fn additional_cameras_extract_as_non_clearing_overlays() {
        let mut world = World::new();
        let overlay = world.create_entity();
        world.add_component(
            overlay,
            components::Camera {
                priority: 10,
                ..Default::default()
            },
        );
        let base = world.create_entity();
        world.add_component(
            base,
            components::Camera {
                priority: -10,
                ..Default::default()
            },
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("valid extraction");
        assert!(matches!(input.views[0].compose, ViewCompose::Base { .. }));
        assert!(matches!(
            input.views[1].compose,
            ViewCompose::Overlay {
                base_view_id: 0,
                blend_mode: BlendMode::Replace
            }
        ));
        assert_eq!(input.views[1].clear_flags, ClearFlags::Nothing);
    }

    #[test]
    fn camera_skybox_clear_flag_maps_to_renderer_contract() {
        assert_eq!(map_clear_flags(0b100), ClearFlags::Skybox);
        assert_eq!(map_clear_flags(0b111), ClearFlags::Skybox);
        assert_eq!(map_clear_flags(0b011), ClearFlags::ColorAndDepth);
    }

    #[test]
    fn invalid_camera_viewport_and_msaa_are_rejected() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                viewport_rect: Some([0.75, 0.0, 0.5, 1.0]),
                msaa_samples: 3,
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("invalid camera settings must fail extraction");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0031"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0032"));
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
        world.add_component(
            e_front,
            components::Renderable {
                mesh_asset: "mesh-visible".into(),
                material_asset: "mat-default".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            e_front,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -5.0),
                ..Default::default()
            },
        );
        world.add_component(
            e_front,
            components::Bounds {
                center: [0.0, 0.0, 0.0],
                half_extents: [0.5, 0.5, 0.5],
            },
        );

        // Renderable behind camera (should be culled).
        let e_back = world.create_entity();
        world.add_component(
            e_back,
            components::Renderable {
                mesh_asset: "mesh-culled".into(),
                material_asset: "mat-default".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            e_back,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, 10.0),
                ..Default::default()
            },
        );
        world.add_component(
            e_back,
            components::Bounds {
                center: [0.0, 0.0, 0.0],
                half_extents: [0.5, 0.5, 0.5],
            },
        );

        let result = extract_renderer_input_from_world(&world, 1);
        assert!(result.is_ok(), "extraction failed: {:?}", result.err());
        let input = result.unwrap();

        // Only the front drawable should survive culling.
        assert_eq!(input.drawables.len(), 1, "expected 1 visible drawable");
        assert_eq!(input.drawables[0].mesh.id, "mesh-visible");
        assert_eq!(
            input.extraction_stats,
            Some(ExtractionStats {
                visible_drawables: 1,
                culled_drawables: 1,
                visible_lights: 0,
                culled_lights: 0,
            })
        );
    }

    #[test]
    fn world_extraction_with_light_produces_light_item() {
        let mut world = World::new();
        let e_cam = world.create_entity();
        world.add_component(e_cam, components::Camera::default());
        world.add_component(e_cam, components::Transform::default());

        let e_light = world.create_entity();
        world.add_component(
            e_light,
            crate::components::Light {
                kind: crate::components::LightKind::Point,
                color: [1.0, 0.5, 0.2],
                intensity: 100.0,
                range: 20.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        let input = extract_renderer_input_from_world(&world, 2).expect("world extraction OK");
        assert_eq!(input.lights.len(), 1);
        assert_eq!(input.lights[0].color, [1.0, 0.5, 0.2]);
        assert_eq!(input.lights[0].intensity, 100.0);
        assert_eq!(input.lights[0].range, 20.0);
    }

    #[test]
    fn drawable_uses_cached_multilevel_parent_world_transform() {
        let mut world = World::new();
        add_default_camera(&mut world);

        let root = world.create_entity();
        let root_transform = components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, -8.0),
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::splat(2.0),
            parent: None,
        };
        world.add_component(root, root_transform.clone());

        let middle = world.create_entity();
        let middle_transform = components::Transform {
            translation: glam::Vec3::X,
            rotation: glam::Quat::from_rotation_y(0.25),
            scale: glam::Vec3::new(0.5, 1.0, 0.5),
            parent: Some(root),
        };
        world.add_component(middle, middle_transform.clone());

        let drawable = world.create_entity();
        let drawable_transform = components::Transform {
            translation: glam::Vec3::Y,
            rotation: glam::Quat::from_rotation_x(-0.4),
            scale: glam::Vec3::new(1.0, 0.75, 1.25),
            parent: Some(middle),
        };
        world.add_component(drawable, drawable_transform.clone());
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "hierarchy-mesh".into(),
                material_asset: "hierarchy-material".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Bounds {
                center: [0.0; 3],
                half_extents: [0.1; 3],
            },
        );

        let input = extract_renderer_input_from_world(&world, 3).expect("hierarchy extracts");
        let item = input
            .drawables
            .iter()
            .find(|item| item.mesh.id == "hierarchy-mesh")
            .expect("drawable remains visible");
        let expected = glam::Mat4::from_scale_rotation_translation(
            root_transform.scale,
            root_transform.rotation,
            root_transform.translation,
        ) * glam::Mat4::from_scale_rotation_translation(
            middle_transform.scale,
            middle_transform.rotation,
            middle_transform.translation,
        ) * glam::Mat4::from_scale_rotation_translation(
            drawable_transform.scale,
            drawable_transform.rotation,
            drawable_transform.translation,
        );
        assert_mat4_approx(&item.world_transform, expected);
        let expected_center = expected.transform_point3(glam::Vec3::ZERO);
        let actual_center = glam::Vec3::from_array([
            (item.bounds.min[0] + item.bounds.max[0]) * 0.5,
            (item.bounds.min[1] + item.bounds.max[1]) * 0.5,
            (item.bounds.min[2] + item.bounds.max[2]) * 0.5,
        ]);
        assert!(actual_center.abs_diff_eq(expected_center, 1.0e-5));
    }

    #[test]
    fn parented_camera_view_is_inverse_of_resolved_world_transform() {
        let mut world = World::new();
        let parent = world.create_entity();
        let parent_transform = components::Transform {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::new(2.0, 1.0, 0.5),
            parent: None,
        };
        world.add_component(parent, parent_transform.clone());

        let camera = world.create_entity();
        let camera_transform = components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, 2.0),
            rotation: glam::Quat::from_rotation_x(0.2),
            scale: glam::Vec3::ONE,
            parent: Some(parent),
        };
        world.add_component(camera, camera_transform.clone());
        world.add_component(camera, components::Camera::default());

        let input =
            extract_renderer_input_from_world(&world, 4).expect("camera hierarchy extracts");
        let parent_world = glam::Mat4::from_scale_rotation_translation(
            parent_transform.scale,
            parent_transform.rotation,
            parent_transform.translation,
        );
        let local_camera = glam::Mat4::from_scale_rotation_translation(
            camera_transform.scale,
            camera_transform.rotation,
            camera_transform.translation,
        );
        assert_mat4_approx(
            &input.views[0].view_matrix,
            (parent_world * local_camera).inverse(),
        );
    }

    #[test]
    fn parented_light_uses_world_position_and_rotated_direction() {
        let mut world = World::new();
        add_default_camera(&mut world);

        let parent = world.create_entity();
        let parent_transform = components::Transform {
            translation: glam::Vec3::new(1.0, 2.0, -5.0),
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::new(2.0, 3.0, 4.0),
            parent: None,
        };
        world.add_component(parent, parent_transform.clone());

        let light = world.create_entity();
        let light_transform = components::Transform {
            translation: glam::Vec3::X,
            parent: Some(parent),
            ..Default::default()
        };
        world.add_component(light, light_transform.clone());
        world.add_component(
            light,
            components::Light {
                kind: components::LightKind::Directional,
                color: [1.0; 3],
                intensity: 1.0,
                range: 10.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [1.0, 0.0, 0.0],
            },
        );

        let input = extract_renderer_input_from_world(&world, 5).expect("light hierarchy extracts");
        let parent_world = glam::Mat4::from_scale_rotation_translation(
            parent_transform.scale,
            parent_transform.rotation,
            parent_transform.translation,
        );
        let local_light = glam::Mat4::from_scale_rotation_translation(
            light_transform.scale,
            light_transform.rotation,
            light_transform.translation,
        );
        let light_world = parent_world * local_light;
        let expected_position = light_world.transform_point3(glam::Vec3::ZERO);
        let expected_direction = light_world.transform_vector3(glam::Vec3::X).normalize();
        assert!(glam::Vec3::from(input.lights[0].position).abs_diff_eq(expected_position, 1.0e-5));
        assert!(glam::Vec3::from(input.lights[0].direction).abs_diff_eq(expected_direction, 1.0e-5));
    }

    #[test]
    fn stale_parent_fails_closed_with_structured_diagnostic() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let stale_parent = world.create_entity();
        assert!(world.destroy_entity(stale_parent));

        let child = world.create_entity();
        world.add_component(
            child,
            components::Transform {
                parent: Some(stale_parent),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 6)
            .expect_err("stale parent must reject extraction");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SC0026")
            .expect("invalid-parent diagnostic");
        assert_eq!(
            diagnostic.fields.get("reason").map(String::as_str),
            Some("stale_or_foreign_domain")
        );
        assert_eq!(
            diagnostic.fields.get("parent_generation"),
            Some(&stale_parent.generation().to_string())
        );
    }

    #[test]
    fn foreign_world_parent_fails_closed_with_structured_diagnostic() {
        let mut foreign_world = World::new();
        let foreign_parent = foreign_world.create_entity();

        let mut world = World::new();
        add_default_camera(&mut world);
        let child = world.create_entity();
        world.add_component(
            child,
            components::Transform {
                parent: Some(foreign_parent),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 7)
            .expect_err("foreign parent must reject extraction");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "SC0026"
                && diagnostic.fields.get("reason").map(String::as_str)
                    == Some("stale_or_foreign_domain")
        }));
    }

    #[test]
    fn parent_cycle_fails_closed_without_recursing_forever() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let first = world.create_entity();
        let second = world.create_entity();
        world.add_component(
            first,
            components::Transform {
                parent: Some(second),
                ..Default::default()
            },
        );
        world.add_component(
            second,
            components::Transform {
                parent: Some(first),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 8)
            .expect_err("cyclic hierarchy must reject extraction");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SC0027")
            .expect("cycle diagnostic");
        assert_eq!(
            diagnostic.fields.get("reason").map(String::as_str),
            Some("parent_cycle")
        );
        assert!(diagnostic.fields.get("cycle").is_some_and(|cycle| {
            cycle.contains(&format!("{}:{}", first.index(), first.generation()))
                && cycle.contains(&format!("{}:{}", second.index(), second.generation()))
        }));
    }
}
