// ────────────────────────────────────────────────────────────────────────────
use super::*;

// Camera-relative rendering (ENG-01)
// ────────────────────────────────────────────────────────────────────────────

/// Ordering for render views: the scene's active camera first, then
/// ascending priority, then persistent id for determinism. Shared by
/// extraction and [`camera_relative_render_origin`] so both always agree on
/// which camera supplies the base view — and therefore the render origin.
pub(super) fn compare_camera_order(
    active_camera: Option<&str>,
    left_priority: i32,
    left_pid: &Option<PersistentId>,
    right_priority: i32,
    right_pid: &Option<PersistentId>,
) -> std::cmp::Ordering {
    let left_is_active = left_pid.as_deref() == active_camera;
    let right_is_active = right_pid.as_deref() == active_camera;
    right_is_active
        .cmp(&left_is_active)
        .then_with(|| left_priority.cmp(&right_priority))
        .then_with(|| left_pid.cmp(right_pid))
}

/// World-space position of the base-view camera — the camera-relative
/// render origin used when `SceneSettings::camera_relative_rendering` is
/// enabled.
///
/// The base camera is chosen with the same active-camera/priority ordering
/// as renderer extraction, so callers stay consistent with the extracted
/// [`RenderView`]s. Returns `None` when the flag is disabled or the world
/// has no camera.
///
/// Render extension producers that emit world-space items outside the
/// canonical extraction path (for example skinned meshes) must translate
/// them by `-origin` when this returns `Some`; otherwise those items render
/// offset from the camera-relative frame by the origin magnitude.
pub fn camera_relative_render_origin(world: &World) -> Option<glam::Vec3> {
    if !world.scene_settings().camera_relative_rendering {
        return None;
    }
    active_camera_world_position(world)
}

/// World-space position of one entity, resolved through its `Transform`
/// parent chain.
///
/// Returns `None` when the entity has no `Transform` component or its chain
/// is invalid (non-finite TRS or a parent cycle), mirroring extraction's
/// tolerance. The position is **origin-relative**: add
/// [`World::world_origin`] for the logical position.
pub fn entity_world_position(world: &World, entity: crate::Entity) -> Option<glam::Vec3> {
    entity_world_transform(world, entity).map(|matrix| matrix.transform_point3(glam::Vec3::ZERO))
}

/// World-space transform of one entity, resolved through its parent chain.
///
/// Returns `None` when the entity has no `Transform` or any transform in the
/// chain is invalid. The matrix is origin-relative, matching
/// [`entity_world_position`].
pub fn entity_world_transform(world: &World, entity: crate::Entity) -> Option<glam::Mat4> {
    world.get::<components::Transform>(entity)?;
    resolve_world_transforms(world).ok()?.get(&entity).copied()
}

/// World-space position of the base-view camera, independent of any render
/// option flags.
///
/// The base camera is chosen with the exact active-camera/priority ordering
/// used by renderer extraction, so the returned position is always the one
/// the primary [`RenderView`] renders from. Returns `None` when the world
/// has no enabled camera or the camera transform chain is invalid.
///
/// Runtime systems that need to know where the player is looking from —
/// world-partition cell streaming, LOD selection, audio listener fallbacks —
/// should use this instead of re-implementing camera selection.
pub fn active_camera_world_position(world: &World) -> Option<glam::Vec3> {
    let world_matrices = resolve_world_transforms(world).ok()?;
    let active_camera = world.scene_settings().active_camera.as_deref();
    let mut cameras: Vec<(i32, Option<PersistentId>, crate::Entity)> = world
        .query::<components::Camera>()
        .map(|(entity, camera)| {
            (
                camera.priority,
                world.persistent_id(entity).map(str::to_owned),
                entity,
            )
        })
        .collect();
    cameras.sort_by(|left, right| {
        compare_camera_order(active_camera, left.0, &left.1, right.0, &right.1)
    });
    let base_entity = cameras.first()?.2;
    let world_matrix = world_matrices
        .get(&base_entity)
        .copied()
        .unwrap_or(glam::Mat4::IDENTITY);
    Some(world_matrix.transform_point3(glam::Vec3::ZERO))
}

/// Resolve the renderer's base camera and its exact surface projection.
pub fn active_camera_view(
    world: &World,
    viewport_context: RenderViewportContext,
) -> Option<ActiveCameraView> {
    let world_matrices = resolve_world_transforms(world).ok()?;
    let active_camera = world.scene_settings().active_camera.as_deref();
    let mut cameras = world
        .query::<components::Camera>()
        .map(|(entity, camera)| {
            (
                camera.priority,
                world.persistent_id(entity).map(str::to_owned),
                entity,
                camera.clone(),
            )
        })
        .collect::<Vec<_>>();
    cameras.sort_by(|left, right| {
        compare_camera_order(active_camera, left.0, &left.1, right.0, &right.1)
    });
    let (_, entity_id, entity, camera) = cameras.into_iter().next()?;
    let world_matrix = world_matrices.get(&entity).copied()?;
    let viewport = effective_camera_viewport(&camera, viewport_context);
    let surface = viewport_context.surface_size();
    let viewport_pixels = [
        viewport.min[0] * surface[0] as f32,
        viewport.min[1] * surface[1] as f32,
        viewport.width() * surface[0] as f32,
        viewport.height() * surface[1] as f32,
    ];
    let projection =
        compute_projection_matrix(&camera, effective_camera_aspect(&camera, viewport_context));
    let view_projection = projection * compute_view_matrix(world_matrix);
    let inverse_view_projection = view_projection.inverse();
    if !view_projection.is_finite() || !inverse_view_projection.is_finite() {
        return None;
    }
    let right = world_matrix.x_axis.truncate().normalize_or_zero();
    let up = world_matrix.y_axis.truncate().normalize_or_zero();
    let forward = -world_matrix.z_axis.truncate().normalize_or_zero();
    Some(ActiveCameraView {
        entity_id,
        perspective: matches!(camera.projection, components::CameraProjection::Perspective),
        position: world_matrix.transform_point3(glam::Vec3::ZERO),
        forward,
        right,
        up,
        viewport_pixels,
        view_projection,
        inverse_view_projection,
    })
}

/// Translate every position carried by debug primitives. Pure translation:
/// radii, half-extents, and rotations are unaffected.
pub(super) fn translate_debug_primitives(primitives: &mut [DebugPrimitive], offset: glam::Vec3) {
    for primitive in primitives {
        match &mut primitive.primitive_kind {
            DebugPrimitiveKind::Line { from, to } => {
                *from = (glam::Vec3::from(*from) + offset).to_array();
                *to = (glam::Vec3::from(*to) + offset).to_array();
            }
            DebugPrimitiveKind::Triangle { a, b, c } => {
                *a = (glam::Vec3::from(*a) + offset).to_array();
                *b = (glam::Vec3::from(*b) + offset).to_array();
                *c = (glam::Vec3::from(*c) + offset).to_array();
            }
            DebugPrimitiveKind::Sphere { center, .. } | DebugPrimitiveKind::Box { center, .. } => {
                *center = (glam::Vec3::from(*center) + offset).to_array();
            }
            DebugPrimitiveKind::Text { position, .. } => {
                *position = (glam::Vec3::from(*position) + offset).to_array();
            }
        }
    }
}

// Frustum culling

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

pub(super) fn unknown_render_layer_diagnostic(
    layer: &str,
    entity: Option<PersistentId>,
) -> Diagnostic {
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
        row3 + row0, // left:  -x - w >= 0 -> (row3 + row0) dot p >= 0
        row3 - row0, // right:  x - w <= 0 -> (row3 - row0) dot p >= 0
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

        // If the entire box is behind this plane, it is outside.
        if d + r < 0.0 {
            return false;
        }
    }

    true
}

// Internal helpers

/// Resolve local TRS values into one cached world matrix per entity.
pub(super) fn resolve_world_transforms(
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

pub(super) fn compute_view_matrix(world_matrix: glam::Mat4) -> glam::Mat4 {
    world_matrix.inverse()
}

/// Compute a 4 x 4 projection matrix from camera parameters.
pub(super) fn compute_projection_matrix(camera: &components::Camera, aspect: f32) -> glam::Mat4 {
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

pub(super) fn effective_camera_viewport(
    camera: &components::Camera,
    context: RenderViewportContext,
) -> Rect {
    context.compose(authored_camera_viewport(camera))
}

pub(super) fn effective_camera_aspect(
    camera: &components::Camera,
    context: RenderViewportContext,
) -> f32 {
    context.aspect_ratio(effective_camera_viewport(camera, context))
}

/// Compute a sort key that groups drawables by material then mesh.
pub(super) fn batch_sort_key(material: &AssetId, mesh: &AssetId) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    material.hash(&mut hasher);
    mesh.hash(&mut hasher);
    hasher.finish()
}

/// Transform a local-space AABB into a conservative world-space AABB.
pub(super) fn transform_bounds_to_world(
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
pub(super) fn map_clear_flags(flags: u8) -> ClearFlags {
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
pub(super) fn physical_exposure_ev100(
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

/// Map the engine's `shadow_mode` byte to the renderer's [`ShadowMode`].
pub(super) fn map_shadow_mode(mode: u8) -> ShadowMode {
    match mode {
        0 => ShadowMode::Off,
        1 => ShadowMode::Hard,
        2 => ShadowMode::Soft,
        _ => ShadowMode::Off,
    }
}
