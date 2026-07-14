use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use engine_renderer::{
    AxisAlignedBox, ClearFlags, ExtractionStats, LightItem, LightKind, Rect, RenderFrameInput,
    RenderView, RenderableItem, ShadowMode, ViewCompose, IDENTITY_MAT4,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity, PersistentId};

use crate::components;
use crate::scene::{Scene, ECS_SCENE_CONTRACT};
use crate::validation::{
    active_camera_entity, asset_field, bool_field, enabled_component, f32_field, light_kind_field,
    string_field, validate_scene, vec3_field,
};
use crate::World;

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
                    let sk = batch_sort_key(&material, &mesh);
                    input.drawables.push(RenderableItem {
                        entity: Some(entity.persistent_id.clone()),
                        mesh,
                        material,
                        world_transform: IDENTITY_MAT4,
                        bounds: AxisAlignedBox::UNIT,
                        render_layer: string_field(renderable, "render_layer")
                            .unwrap_or_else(|| scene.scene_settings.default_render_layer.clone()),
                        cast_shadows: bool_field(renderable, "cast_shadows").unwrap_or(true),
                        sort_key: sk,
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

    // Sort drawables by (material, mesh) for efficient batching.
    input.drawables.sort_by_key(|d| d.sort_key);
    input.extraction_stats = Some(ExtractionStats {
        visible_drawables: input.drawables.len() as u32,
        culled_drawables: 0,
        visible_lights: input.lights.len() as u32,
        culled_lights: 0,
    });

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

    // Resolve every Transform once up front. Hierarchy corruption is fatal:
    // rendering a partially-local, partially-world-space frame is less safe
    // than rejecting the frame with a structured diagnostic.
    let world_matrices = resolve_world_transforms(world)?;

    // ── Camera pass: build RenderViews ──────────────────────────────────

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

        let projection = compute_projection_matrix(&camera);
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

    // Sort by priority (ascending = earlier render).
    cameras.sort_by_key(|(priority, _, _, _, _)| *priority);

    // Compute the primary frustum from the first camera for culling.
    let primary_frustum: Option<[glam::Vec4; 6]> =
        cameras.first().map(|(_, _, camera, world_matrix, _)| {
            let view = compute_view_matrix(*world_matrix);
            let proj = compute_projection_matrix(camera);
            let view_proj = proj * view;
            extract_frustum_planes(&view_proj)
        });

    for (view_idx, (priority, pid, camera, world_matrix, _entity)) in cameras.iter().enumerate() {
        let view = compute_view_matrix(*world_matrix);
        let proj = compute_projection_matrix(camera);

        let clear_color = camera.clear_color;
        let clear_flags = map_clear_flags(camera.clear_flags);

        let frustum = Some(extract_frustum_planes(&(proj * view)));

        let viewport = match camera.viewport_rect {
            Some([x, y, w, h]) => Rect {
                min: [x, y],
                max: [x + w, y + h],
            },
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
        let bounds = world.get::<components::Bounds>(entity);

        let world_matrix = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let world_mat = world_matrix.to_cols_array();
        let (center, half_extents, world_bounds) = transform_bounds_to_world(bounds, world_matrix);

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

    // ── Light pass: build LightItems ────────────────────────────────────

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
        row3 + row0, // left:   -x - w >= 0  →  -(row0·p) - (row3·p) >= 0  →  (row3 + row0)·p >= 0
        row3 - row0, // right:   x - w <= 0  →   (row0·p) - (row3·p) <= 0  →  (row3 - row0)·p >= 0
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

/// Compute a 4×4 projection matrix from camera parameters.
fn compute_projection_matrix(camera: &components::Camera) -> glam::Mat4 {
    // Default aspect ratio (16:9) — in production this comes from the viewport.
    const ASPECT: f32 = 16.0 / 9.0;

    match camera.projection {
        components::CameraProjection::Perspective => {
            glam::Mat4::perspective_rh(camera.fov_y, ASPECT, camera.near, camera.far)
        }
        components::CameraProjection::Orthographic => {
            let half_w = camera.ortho_half_height * ASPECT;
            let half_h = camera.ortho_half_height;
            glam::Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, camera.near, camera.far)
        }
    }
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

    // ── Frustum culling tests ───────────────────────────────────────────

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
        assert!(
            result.is_err(),
            "expected extraction to fail without camera"
        );
    }

    #[test]
    fn extract_from_world_produces_parity_with_scene() {
        let scene = sample_scene();
        let scene_input = extract_renderer_input(&scene, 7).expect("scene extraction OK");

        // Convert scene to world and extract via the new path.
        let world = World::from_scene(&scene);
        let world_input =
            extract_renderer_input_from_world(&world, 7).expect("world extraction OK");

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
        assert_eq!(
            scene_input.extraction_stats,
            Some(ExtractionStats {
                visible_drawables: scene_input.drawables.len() as u32,
                culled_drawables: 0,
                visible_lights: scene_input.lights.len() as u32,
                culled_lights: 0,
            }),
            "legacy extraction must publish structured totals"
        );

        // Compare drawable mesh/material/render_layer.
        for (wd, sd) in world_input
            .drawables
            .iter()
            .zip(scene_input.drawables.iter())
        {
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
