use super::spatial::{
    batch_sort_key, compare_camera_order, compute_projection_matrix, compute_view_matrix,
    effective_camera_aspect, effective_camera_viewport, map_clear_flags, map_shadow_mode,
    physical_exposure_ev100, resolve_world_transforms, transform_bounds_to_world,
    translate_debug_primitives, unknown_render_layer_diagnostic,
};
use super::*;

// Canonical World extraction path

/// Extract renderer input from the canonical ECS [`World`].
///
/// Iterates all entities with [`Camera`] into [`RenderView`],
/// [`Renderable`] + [`Transform`] + [`Bounds`] into [`RenderableItem`],
/// and [`Light`] into [`LightItem`]. Performs frustum culling against the
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
    let scene_settings = world.scene_settings();
    input.render_options.tone_mapping = scene_settings.tone_mapping;
    input.render_options.transparency_mode = scene_settings.transparency_mode;
    input.render_options.pass_graph_config = scene_settings.pass_graph_config.clone();
    input.render_options.environment = engine_renderer::EnvironmentSettings {
        environment_map: scene_settings.environment_map.clone(),
        intensity: scene_settings.environment_intensity,
        rotation_radians: scene_settings.environment_rotation_radians,
        reflection_probes: scene_settings.reflection_probes.clone(),
    };
    input.render_options.post_process = scene_settings.post_process;
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Resolve every Transform once up front. Hierarchy corruption is fatal:
    // rendering a partially-local, partially-world-space frame is less safe
    // than rejecting the frame with a structured diagnostic.
    let world_matrices = resolve_world_transforms(world)?;

    // Camera pass: build RenderViews.

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
        compare_camera_order(active_camera, left.0, &left.1, right.0, &right.1)
    });
    let base_camera = &cameras[0].2;
    input.render_options.msaa_samples = base_camera.msaa_samples;
    input.render_options.exposure_ev100 = physical_exposure_ev100(
        base_camera.aperture,
        base_camera.shutter_speed,
        base_camera.iso,
        base_camera.ev_compensation,
    );

    // ENG-01 camera-relative rendering: when enabled, every emitted view,
    // drawable, light, and debug primitive is translated by `-origin`, where
    // `origin` is the base camera's resolved world position. The base view
    // matrix becomes translation-free, so the shader chain
    // `proj * view * (model * pos)` evaluates at the magnitude of
    // camera-relative offsets instead of absolute world coordinates.
    // Frustum culling stays in absolute space, so the flag never changes
    // which drawables and lights are culled.
    let relative_origin = world
        .scene_settings()
        .camera_relative_rendering
        .then(|| cameras[0].3.transform_point3(glam::Vec3::ZERO));
    let relative_shift = relative_origin.map(|origin| glam::Mat4::from_translation(-origin));

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
        // Camera-relative v1 shifts every view by the *base* camera origin.
        // For the base view this removes the translation exactly; overlay
        // views keep their offset from the base camera, so an overlay far
        // from the base camera is only as precise as that relative offset.
        let render_world = relative_shift
            .map(|shift| shift * *world_matrix)
            .unwrap_or(*world_matrix);
        let view = compute_view_matrix(render_world);
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

    // Renderable pass: build Drawables.

    let mut visible_drawables: u32 = 0;
    let mut culled_drawables: u32 = 0;
    let base_camera_position = cameras[0].3.transform_point3(glam::Vec3::ZERO);
    let mut active_hlod_clusters = HashSet::new();
    let mut culled_hlod_clusters = HashSet::new();
    for (entity, cluster) in world.query::<components::HlodCluster>() {
        if cluster.role != components::HlodRole::Proxy {
            continue;
        }
        let Some(renderable) = world.get::<components::Renderable>(entity) else {
            continue;
        };
        if !renderable.visible {
            continue;
        }
        let absolute_world = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        let (center, _, _) =
            transform_bounds_to_world(world.get::<components::Bounds>(entity), absolute_world);
        let distance = glam::Vec3::from_array(center).distance(base_camera_position);
        if cluster.proxy_is_active(distance) {
            active_hlod_clusters.insert(cluster.cluster_id.clone());
        } else if cluster.cull_distance > 0.0 && distance >= cluster.cull_distance {
            culled_hlod_clusters.insert(cluster.cluster_id.clone());
        }
    }
    for active in &active_hlod_clusters {
        culled_hlod_clusters.remove(active);
    }

    for (entity, renderable) in world.query::<components::Renderable>() {
        if !renderable.visible {
            continue;
        }

        // Skip if mesh or material asset is empty.
        if renderable.mesh_asset.is_empty() || renderable.material_asset.is_empty() {
            continue;
        }
        if let Some(cluster) = world.get::<components::HlodCluster>(entity) {
            let cluster_active = active_hlod_clusters.contains(&cluster.cluster_id);
            let cluster_culled = culled_hlod_clusters.contains(&cluster.cluster_id);
            let visible_for_role = match cluster.role {
                components::HlodRole::Source => !cluster_active && !cluster_culled,
                components::HlodRole::Proxy => cluster_active,
            };
            if !visible_for_role {
                culled_drawables = culled_drawables.saturating_add(1);
                continue;
            }
        }

        let pid = world.persistent_id(entity).map(|s| s.to_string());
        let bounds = world.get::<components::Bounds>(entity);

        let absolute_world = world_matrices
            .get(&entity)
            .copied()
            .unwrap_or(glam::Mat4::IDENTITY);
        // Culling uses the absolute world transform against absolute camera
        // frustums, so the flag never changes culling decisions. Emitted
        // transforms and bounds are then translated into camera-relative
        // space when the flag is enabled.
        let (center, half_extents, _) = transform_bounds_to_world(bounds, absolute_world);
        let selected_assets = world.get::<components::LodGroup>(entity).map_or(
            Some((
                renderable.mesh_asset.as_str(),
                renderable.material_asset.as_str(),
            )),
            |group| {
                group.select_assets(
                    glam::Vec3::from_array(center).distance(base_camera_position),
                    &renderable.mesh_asset,
                    &renderable.material_asset,
                )
            },
        );
        let Some((selected_mesh, selected_material)) = selected_assets else {
            culled_drawables += 1;
            continue;
        };
        let render_world = relative_shift
            .map(|shift| shift * absolute_world)
            .unwrap_or(absolute_world);
        let world_mat = render_world.to_cols_array();
        let (_, _, world_bounds) = transform_bounds_to_world(bounds, render_world);

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

        let mesh = engine_serialize::AssetId::new(selected_mesh);
        let material = engine_serialize::AssetId::new(selected_material);
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
            radial_vertex_morph: world
                .get::<components::VertexGeomorph>(entity)
                .map(|morph| engine_renderer::RadialVertexMorph {
                    factor: morph.factor.clamp(0.0, 1.0),
                    delta_scale: morph.delta_scale.max(0.0),
                    local_origin: morph.local_origin,
                }),
            triplanar_material_mapping: world
                .get::<components::TriplanarMaterialMapping>(entity)
                .filter(|mapping| {
                    mapping.local_origin.into_iter().all(f32::is_finite)
                        && mapping.meters_per_tile.is_finite()
                        && mapping.meters_per_tile > 0.0
                        && mapping.blend_sharpness.is_finite()
                })
                .map(|mapping| engine_renderer::TriplanarMaterialMapping {
                    local_origin: mapping.local_origin,
                    meters_per_tile: mapping.meters_per_tile,
                    blend_sharpness: mapping.blend_sharpness.clamp(1.0, 32.0),
                }),
        });
    }

    // Sort drawables by (material, mesh) for efficient batching.
    input.drawables.sort_by_key(|d| d.sort_key);

    // Light pass: build LightItems.

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
        // A pure translation shift does not change `transform_vector3`, so
        // light directions are unaffected; positions become camera-relative.
        let render_world = relative_shift
            .map(|shift| shift * world_matrix)
            .unwrap_or(world_matrix);
        let position = render_world.transform_point3(glam::Vec3::ZERO).to_array();
        let direction = render_world
            .transform_vector3(glam::Vec3::from(light.direction))
            .normalize_or_zero()
            .to_array();

        let spot_angles = light
            .spot_angles
            .map(|[inner, outer]| engine_renderer::SpotAngles { inner, outer });

        input.lights.push(LightItem {
            entity: pid,
            kind: light.kind,
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

    // Extraction emits no debug primitives today, but any that arrive on
    // this path are shifted like everything else. Primitives injected by
    // render extension producers after extraction are the producer's
    // responsibility (see [`camera_relative_render_origin`]).
    if let Some(origin) = relative_origin {
        translate_debug_primitives(&mut input.debug_primitives, -origin);
    }

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
