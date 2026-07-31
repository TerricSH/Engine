use super::*;

pub(super) fn editor_render_viewport(
    viewport: ScreenRect,
    scale_factor: f64,
    window_size: Vec2,
) -> Option<(Vec2, Vec2, RenderViewportContext)> {
    if !viewport.x.is_finite()
        || !viewport.y.is_finite()
        || !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || !scale_factor.is_finite()
        || scale_factor <= 0.0
        || viewport.width <= 0.0
        || viewport.height <= 0.0
        || !window_size.is_finite()
        || window_size.x <= 0.0
        || window_size.y <= 0.0
    {
        return None;
    }

    // DOMRect coordinates are CSS logical pixels. Mapping both edges independently keeps
    // adjacent dock regions seam-free on fractional DPI scales.
    let scale = scale_factor as f32;
    let min = Vec2::new((viewport.x * scale).round(), (viewport.y * scale).round())
        .clamp(Vec2::ZERO, window_size);
    let max = Vec2::new(
        ((viewport.x + viewport.width) * scale).round(),
        ((viewport.y + viewport.height) * scale).round(),
    )
    .clamp(Vec2::ZERO, window_size);
    if max.x <= min.x || max.y <= min.y {
        return None;
    }

    let normalized = RendererRect {
        min: [min.x / window_size.x, min.y / window_size.y],
        max: [max.x / window_size.x, max.y / window_size.y],
    };
    let render_viewport = RenderViewportContext::new(
        window_size.x.round() as u32,
        window_size.y.round() as u32,
        normalized,
    )?;
    Some((min, max, render_viewport))
}

pub(super) fn log_scene_diagnostics(context: &str, diagnostics: Vec<engine_serialize::Diagnostic>) {
    for diagnostic in diagnostics {
        tracing::error!(
            code = diagnostic.code,
            entity = ?diagnostic.entity,
            message = diagnostic.message,
            "{context}"
        );
    }
}

pub(super) fn summarize_scene_diagnostics(diagnostics: &[engine_serialize::Diagnostic]) -> String {
    diagnostics
        .iter()
        .take(4)
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn editor_camera_entity(scene: &Scene) -> EntityRecord {
    let mut suffix = 0_u64;
    let persistent_id = loop {
        let candidate = if suffix == 0 {
            EDITOR_CAMERA_ID_PREFIX.to_string()
        } else {
            format!("{EDITOR_CAMERA_ID_PREFIX}_{suffix}")
        };
        if scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != candidate)
        {
            break candidate;
        }
        suffix += 1;
    };
    let record = |fields| ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    };
    EntityRecord {
        persistent_id,
        parent: None,
        name: Some("Editor Camera".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            (
                "engine.camera".into(),
                record(std::collections::BTreeMap::from([
                    ("clear_flags".into(), Value::UInt(3)),
                    (
                        "clear_color".into(),
                        Value::Color([0.055, 0.06, 0.075, 1.0]),
                    ),
                    ("aperture".into(), Value::Float32(1.0)),
                    ("shutter_speed".into(), Value::Float32(1.0)),
                    ("iso".into(), Value::Float32(100.0)),
                ])),
            ),
            (
                "engine.transform".into(),
                record(std::collections::BTreeMap::from([
                    ("translation".into(), Value::Vec3([0.0, 0.0, 5.0])),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
                ])),
            ),
        ]),
    }
}

pub(super) fn editor_light_entity(scene: &Scene) -> EntityRecord {
    let mut suffix = 0_u64;
    let persistent_id = loop {
        let candidate = if suffix == 0 {
            EDITOR_LIGHT_ID_PREFIX.to_string()
        } else {
            format!("{EDITOR_LIGHT_ID_PREFIX}_{suffix}")
        };
        if scene
            .entities
            .iter()
            .all(|entity| entity.persistent_id != candidate)
        {
            break candidate;
        }
        suffix += 1;
    };
    EntityRecord {
        persistent_id,
        parent: None,
        name: Some("Editor Light".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([(
            "engine.light".into(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::from([
                    ("kind".into(), Value::Enum("Directional".into())),
                    ("color".into(), Value::Vec3([1.0, 0.96, 0.9])),
                    ("intensity".into(), Value::Float32(2.5)),
                    ("direction".into(), Value::Vec3([-0.35, -0.8, -0.45])),
                    ("shadow_mode".into(), Value::UInt(0)),
                ]),
            },
        )]),
    }
}

pub(super) fn editor_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
) -> (Scene, Vec<Diagnostic>) {
    authoring_preview_scene(runtime, authoring_scene, true)
}

pub(super) fn game_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
) -> (Scene, Vec<Diagnostic>) {
    authoring_preview_scene(runtime, authoring_scene, false)
}

pub(super) fn authoring_preview_scene(
    runtime: &engine_core::EngineRuntime,
    authoring_scene: &Scene,
    use_editor_camera: bool,
) -> (Scene, Vec<Diagnostic>) {
    let mut preview = authoring_scene.clone();
    let mut diagnostics = Vec::new();
    for entity in &mut preview.entities {
        // Edit mode displays the authoring scene but must not instantiate game
        // behaviours.  Otherwise every inspector edit or gizmo commit reloads
        // the preview and repeatedly invokes OnDestroy/OnCreate outside Play.
        // The component remains intact in `authoring_scene` and is restored by
        // the normal Play transition.
        entity.components.remove("engine.script");
        if use_editor_camera {
            if let Some(camera) = entity.components.get_mut("engine.camera") {
                // Authoring cameras remain as selectable entities (including
                // their Transform), but only the dedicated editor camera renders
                // while outside Play.
                camera.enabled = false;
            }
        }

        let Some(renderable) = entity.components.get_mut("engine.renderable") else {
            continue;
        };
        for (field, fallback) in [("mesh", "mesh-cube"), ("material", "mat-default")] {
            let Some(Value::Asset(asset)) = renderable.fields.get(field) else {
                continue;
            };
            if runtime.asset_registry().contains(asset) {
                continue;
            }
            let missing = asset.clone();
            renderable
                .fields
                .insert(field.into(), Value::Asset(AssetId::new(fallback)));
            let mut diagnostic = Diagnostic::new(
                    "EDASSET_MISSING",
                    DiagnosticSeverity::Warning,
                    "editor.asset-browser",
                    format!(
                        "asset '{}' is missing; editor preview uses '{}' until the authoring reference is repaired",
                        missing.id, fallback
                    ),
                )
                .entity(entity.persistent_id.clone())
                .path(format!(
                    "entities[{}].components[engine.renderable].fields[{field}]",
                    entity.persistent_id
                ));
            diagnostic.asset = Some(missing);
            diagnostics.push(diagnostic);
        }
    }
    if use_editor_camera {
        let editor_camera = editor_camera_entity(&preview);
        preview.scene_settings.active_camera = Some(editor_camera.persistent_id.clone());
        preview.entities.push(editor_camera);
        let has_scene_light = preview.entities.iter().any(|entity| {
            entity.enabled
                && entity
                    .components
                    .get("engine.light")
                    .is_some_and(|light| light.enabled)
        });
        if !has_scene_light {
            let editor_light = editor_light_entity(&preview);
            preview.entities.push(editor_light);
        }
    }
    (preview, diagnostics)
}

pub(super) fn restore_editor_preview(
    game_loop: &mut GameLoop,
    authoring_scene: &Scene,
) -> Result<Vec<Diagnostic>, Vec<Diagnostic>> {
    let (preview_scene, warnings) = editor_preview_scene(&game_loop.runtime, authoring_scene);
    game_loop.load_scene(preview_scene)?;
    game_loop.init_physics();
    // Play-attachment errors were copied into the editor diagnostics by the
    // caller; do not let them poison later edit-mode health checks.
    game_loop.runtime.diagnostics_collector_mut().clear_frame();
    Ok(warnings)
}

pub(super) fn synchronize_editor_preview(game_loop: &mut GameLoop, editor_scene: &mut EditorScene) {
    let (preview_scene, missing_diagnostics) =
        editor_preview_scene(&game_loop.runtime, &editor_scene.scene);
    editor_scene.diagnostics.clear();
    editor_scene.diagnostics.push_many(missing_diagnostics);
    if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
        log_scene_diagnostics("editor scene synchronisation failed", diagnostics);
    } else {
        game_loop.init_physics();
    }
}

pub(super) fn synchronize_game_preview(game_loop: &mut GameLoop, editor_scene: &mut EditorScene) {
    let (preview_scene, missing_diagnostics) =
        game_preview_scene(&game_loop.runtime, &editor_scene.scene);
    editor_scene.diagnostics.clear();
    editor_scene.diagnostics.push_many(missing_diagnostics);
    if let Err(diagnostics) = game_loop.load_scene(preview_scene) {
        log_scene_diagnostics("game-view preview synchronisation failed", diagnostics);
    } else {
        game_loop.init_physics();
    }
}

pub(super) fn recover_play_after_script_error(
    play_session: &mut EditorPlaySession,
    game_loop: &mut GameLoop,
    error: impl Into<String>,
) -> Vec<Diagnostic> {
    let error = error.into();
    let mut diagnostics = vec![Diagnostic::new(
        "EDPLAY_SCRIPT_UPDATE_FAILED",
        DiagnosticSeverity::Error,
        "editor.play-mode",
        format!("Play stopped after a game script update failed: {error}"),
    )];
    let mut preview_warnings = Vec::new();
    let restore_result = play_session.stop(|authoring_scene| {
        match restore_editor_preview(game_loop, &authoring_scene) {
            Ok(warnings) => {
                preview_warnings = warnings;
                Ok(())
            }
            Err(diagnostics) => Err(diagnostics),
        }
    });
    match restore_result {
        Ok(true) => diagnostics.extend(preview_warnings),
        Ok(false) => {}
        Err(restore_diagnostics) => {
            // A failed rollback must not spin the same failing script on every
            // redraw. Leave the snapshot intact in Paused mode so Stop can be
            // retried after the underlying issue is repaired.
            let _ = play_session.pause();
            diagnostics.extend(restore_diagnostics);
        }
    }
    diagnostics
}

pub(super) fn recover_play_after_scene_transition_error(
    play_session: &mut EditorPlaySession,
    game_loop: &mut GameLoop,
    error: impl Into<String>,
) -> Vec<Diagnostic> {
    let error = error.into();
    let mut diagnostics = vec![Diagnostic::new(
        "EDPLAY_SCENE_TRANSITION_FAILED",
        DiagnosticSeverity::Error,
        "editor.play-mode",
        format!("Play stopped after a scene transition failed: {error}"),
    )];
    let mut preview_warnings = Vec::new();
    let restore_result = play_session.stop(|authoring_scene| {
        match restore_editor_preview(game_loop, &authoring_scene) {
            Ok(warnings) => {
                preview_warnings = warnings;
                Ok(())
            }
            Err(diagnostics) => Err(diagnostics),
        }
    });
    match restore_result {
        Ok(true) => diagnostics.extend(preview_warnings),
        Ok(false) => {}
        Err(restore_diagnostics) => {
            let _ = play_session.pause();
            diagnostics.extend(restore_diagnostics);
        }
    }
    diagnostics
}

#[cfg(test)]
pub(super) fn execute_selected_asset_assignment(
    browser: &ProjectAssetBrowserPanel,
    editor_scene: &mut EditorScene,
) -> Result<bool, engine_editor::EditorError> {
    let Some(target_entity) = editor_scene.selected_entity.clone() else {
        return Ok(false);
    };
    let Some(command) = browser.selected_assignment_command(target_entity) else {
        return Ok(false);
    };
    editor_scene.execute(Box::new(command))?;
    Ok(true)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RuntimeGizmoView {
    pub(super) world_position: Vec3,
    pub(super) world_rotation: Quat,
    pub(super) view: Mat4,
    pub(super) projection: Mat4,
    pub(super) viewport_origin: Vec2,
    pub(super) viewport_size: Vec2,
    pub(super) interaction_min: Vec2,
    pub(super) interaction_max: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum GizmoPointerEvent {
    Press(Vec2),
    Move(Vec2),
    Release(Vec2),
    Cancel,
}

pub(super) fn resolve_runtime_world_matrix(
    world: &World,
    entity: Entity,
    visiting: &mut Vec<Entity>,
) -> Option<Mat4> {
    if visiting.contains(&entity) || !world.is_alive(entity) {
        return None;
    }
    let Some(transform) = world.get::<Transform>(entity) else {
        return Some(Mat4::IDENTITY);
    };
    if !transform.translation.is_finite()
        || !transform.rotation.is_finite()
        || transform.rotation.length_squared() <= f32::EPSILON
        || !transform.scale.is_finite()
    {
        return None;
    }

    visiting.push(entity);
    let parent_world = match transform.parent {
        Some(parent) => resolve_runtime_world_matrix(world, parent, visiting)?,
        None => Mat4::IDENTITY,
    };
    visiting.pop();
    let local = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation.normalize(),
        transform.translation,
    );
    let resolved = parent_world * local;
    resolved.is_finite().then_some(resolved)
}

pub(super) fn runtime_gizmo_view(
    runtime: &EngineRuntime,
    entity_id: &str,
    frame: u64,
    viewport_context: RenderViewportContext,
) -> Option<RuntimeGizmoView> {
    let surface_size = viewport_context.surface_size();
    let window_size = Vec2::new(surface_size[0] as f32, surface_size[1] as f32);
    runtime
        .with_world(|world| {
            let input =
                extract_renderer_input_from_world_with_viewport(world, frame, viewport_context)
                    .ok()?;
            let view = input.views.first()?;
            let entity = world.entity_by_persistent_id(entity_id)?;
            world.get::<Transform>(entity)?;
            let world_matrix = resolve_runtime_world_matrix(world, entity, &mut Vec::new())?;
            let (_scale, world_rotation, world_position) =
                world_matrix.to_scale_rotation_translation();
            if !world_position.is_finite()
                || !world_rotation.is_finite()
                || world_rotation.length_squared() <= f32::EPSILON
            {
                return None;
            }

            let viewport_min = Vec2::from_array(view.viewport_rect_normalized.min) * window_size;
            let viewport_max = Vec2::from_array(view.viewport_rect_normalized.max) * window_size;
            let viewport_size = viewport_max - viewport_min;
            if !viewport_min.is_finite()
                || !viewport_size.is_finite()
                || viewport_size.x <= 0.0
                || viewport_size.y <= 0.0
            {
                return None;
            }

            Some(RuntimeGizmoView {
                world_position,
                world_rotation: world_rotation.normalize(),
                view: Mat4::from_cols_array(&view.view_matrix),
                projection: Mat4::from_cols_array(&view.projection_matrix),
                viewport_origin: viewport_min,
                viewport_size,
                interaction_min: viewport_min,
                interaction_max: viewport_max,
            })
        })
        .flatten()
}

pub(super) fn project_world_point(
    world: Vec3,
    view: Mat4,
    projection: Mat4,
    viewport_size: Vec2,
) -> Option<(Vec2, f32)> {
    let clip = projection * view * world.extend(1.0);
    if !clip.is_finite() || clip.w <= 1.0e-5 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !ndc.is_finite() || !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some((
        Vec2::new(
            (ndc.x * 0.5 + 0.5) * viewport_size.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * viewport_size.y,
        ),
        ndc.z,
    ))
}

pub(super) fn pick_runtime_entity(
    runtime: &EngineRuntime,
    frame: u64,
    viewport_context: RenderViewportContext,
    interaction_min: Vec2,
    interaction_max: Vec2,
    pointer: Vec2,
) -> Option<PersistentId> {
    if pointer.x < interaction_min.x
        || pointer.y < interaction_min.y
        || pointer.x > interaction_max.x
        || pointer.y > interaction_max.y
    {
        return None;
    }

    runtime
        .with_world(|world| {
            let input =
                extract_renderer_input_from_world_with_viewport(world, frame, viewport_context)
                    .ok()?;
            let render_view = input.views.first()?;
            let view = Mat4::from_cols_array(&render_view.view_matrix);
            let projection = Mat4::from_cols_array(&render_view.projection_matrix);
            let surface_size = viewport_context.surface_size();
            let window_size = Vec2::new(surface_size[0] as f32, surface_size[1] as f32);
            let viewport_min =
                Vec2::from_array(render_view.viewport_rect_normalized.min) * window_size;
            let viewport_max =
                Vec2::from_array(render_view.viewport_rect_normalized.max) * window_size;
            let viewport_size = viewport_max - viewport_min;
            let mut best: Option<(f32, PersistentId)> = None;

            for drawable in &input.drawables {
                let Some(entity) = drawable.entity.clone() else {
                    continue;
                };
                let min = Vec3::from_array(drawable.bounds.min);
                let max = Vec3::from_array(drawable.bounds.max);
                let mut screen_min = Vec2::splat(f32::INFINITY);
                let mut screen_max = Vec2::splat(f32::NEG_INFINITY);
                let mut nearest_depth = f32::INFINITY;
                let mut projected_corner = false;
                for x in [min.x, max.x] {
                    for y in [min.y, max.y] {
                        for z in [min.z, max.z] {
                            let Some((screen, depth)) = project_world_point(
                                Vec3::new(x, y, z),
                                view,
                                projection,
                                viewport_size,
                            ) else {
                                continue;
                            };
                            let screen = viewport_min + screen;
                            projected_corner = true;
                            screen_min = screen_min.min(screen);
                            screen_max = screen_max.max(screen);
                            nearest_depth = nearest_depth.min(depth);
                        }
                    }
                }
                if !projected_corner {
                    continue;
                }
                // Thin or very small meshes still receive a practical click
                // target, while overlapping objects choose the nearest depth.
                let padding = Vec2::splat(6.0);
                if pointer.x >= screen_min.x - padding.x
                    && pointer.y >= screen_min.y - padding.y
                    && pointer.x <= screen_max.x + padding.x
                    && pointer.y <= screen_max.y + padding.y
                    && best
                        .as_ref()
                        .is_none_or(|(best_depth, _)| nearest_depth < *best_depth)
                {
                    best = Some((nearest_depth, entity));
                }
            }
            best.map(|(_, entity)| entity)
        })
        .flatten()
}

pub(super) fn restrict_gizmo_view_to_rect(
    mut view: RuntimeGizmoView,
    minimum: Vec2,
    maximum: Vec2,
) -> Option<RuntimeGizmoView> {
    view.interaction_min = view.viewport_origin.max(minimum);
    view.interaction_max = (view.viewport_origin + view.viewport_size).min(maximum);
    (view.interaction_max.x > view.interaction_min.x
        && view.interaction_max.y > view.interaction_min.y)
        .then_some(view)
}

pub(super) fn apply_editor_camera(runtime: &EngineRuntime, panel: &SceneViewPanel) -> bool {
    let (pitch, yaw, distance) = panel.camera_orbit();
    let target = Vec3::from_array(*panel.target());
    let pitch = pitch.to_radians();
    let yaw = yaw.to_radians();
    let offset = Vec3::new(
        distance * yaw.cos() * pitch.cos(),
        distance * pitch.sin(),
        distance * yaw.sin() * pitch.cos(),
    );
    let (translation, rotation) = engine_scene::camera_utils::setup_orbit_transform(target, offset);
    if !translation.is_finite() || !rotation.is_finite() {
        return false;
    }

    runtime
        .with_world_mut(|world| {
            let active_camera = world.scene_settings().active_camera.clone();
            let entity = active_camera
                .as_deref()
                .and_then(|id| world.entity_by_persistent_id(id))
                .or_else(|| world.query::<Camera>().next().map(|(entity, _)| entity));
            let Some(entity) = entity else {
                return false;
            };
            if let Some(transform) = world.get_mut::<Transform>(entity) {
                transform.translation = translation;
                transform.rotation = rotation;
                transform.scale = Vec3::ONE;
                transform.parent = None;
            } else {
                world.add_component(
                    entity,
                    Transform {
                        translation,
                        rotation,
                        scale: Vec3::ONE,
                        parent: None,
                    },
                );
            }
            if let Some(camera) = world.get_mut::<Camera>(entity) {
                camera.projection = if panel.orthographic() {
                    CameraProjection::Orthographic
                } else {
                    CameraProjection::Perspective
                };
                camera.ortho_half_height = distance.max(0.1);
            }
            true
        })
        .unwrap_or(false)
}

pub(super) fn synchronize_editor_preview_and_camera(
    game_loop: &mut GameLoop,
    editor_scene: &mut EditorScene,
    scene_view: &SceneViewPanel,
) {
    synchronize_editor_preview(game_loop, editor_scene);
    let _ = apply_editor_camera(&game_loop.runtime, scene_view);
}

pub(super) fn synchronize_authoring_view(
    game_loop: &mut GameLoop,
    editor_scene: &mut EditorScene,
    scene_view: &SceneViewPanel,
    viewport_tab: ViewportTab,
) {
    match viewport_tab {
        ViewportTab::Scene => {
            synchronize_editor_preview_and_camera(game_loop, editor_scene, scene_view)
        }
        ViewportTab::Game => synchronize_game_preview(game_loop, editor_scene),
    }
}

pub(super) fn sync_runtime_transform(
    runtime: &EngineRuntime,
    entity_id: &str,
    authoring: &Transform,
) -> bool {
    runtime
        .with_world_mut(|world| {
            let Some(entity) = world.entity_by_persistent_id(entity_id) else {
                return false;
            };
            let Some(runtime_transform) = world.get_mut::<Transform>(entity) else {
                return false;
            };
            runtime_transform.translation = authoring.translation;
            runtime_transform.rotation = authoring.rotation;
            runtime_transform.scale = authoring.scale;
            true
        })
        .unwrap_or(false)
}

pub(super) fn apply_gizmo_preview_delta(
    editor_scene: &mut EditorScene,
    gizmo: &mut GizmoSystem,
    runtime: &EngineRuntime,
    entity_id: &str,
) -> bool {
    let delta = gizmo.take_delta();
    if delta.length_squared() <= 0.0 || !editor_scene.preview_transform_gizmo_drag(gizmo, delta) {
        return false;
    }
    editor_scene
        .selected_transform_for_gizmo()
        .is_some_and(|transform| sync_runtime_transform(runtime, entity_id, &transform))
}

pub(super) fn offset_gizmo_batch(mut batch: UiBatch, view: RuntimeGizmoView) -> UiBatch {
    for vertex in &mut batch.vertices {
        vertex.position[0] += view.viewport_origin.x;
        vertex.position[1] += view.viewport_origin.y;
    }
    batch.clip_rect.min = view.interaction_min.to_array();
    batch.clip_rect.max = view.interaction_max.to_array();
    batch
}

pub(super) fn process_gizmo_pointer_events(
    events: Vec<GizmoPointerEvent>,
    editor_scene: &mut EditorScene,
    gizmo: &mut GizmoSystem,
    runtime: &EngineRuntime,
    entity_id: &str,
    view: RuntimeGizmoView,
) -> bool {
    let mut scene_changed = false;
    if editor_scene
        .active_transform_gizmo_entity()
        .is_some_and(|active| active != entity_id)
    {
        scene_changed |= editor_scene.cancel_transform_gizmo_drag();
        gizmo.cancel_drag();
        return scene_changed;
    }
    for event in events {
        let pointer = match event {
            GizmoPointerEvent::Press(pointer)
            | GizmoPointerEvent::Move(pointer)
            | GizmoPointerEvent::Release(pointer) => pointer,
            GizmoPointerEvent::Cancel => {
                scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                gizmo.cancel_drag();
                continue;
            }
        };
        let local_pointer = pointer - view.viewport_origin;

        match event {
            GizmoPointerEvent::Press(_) => {
                if gizmo.dragging {
                    continue;
                }
                let pointer_inside_view = pointer.x >= view.interaction_min.x
                    && pointer.y >= view.interaction_min.y
                    && pointer.x <= view.interaction_max.x
                    && pointer.y <= view.interaction_max.y;
                if !pointer_inside_view {
                    continue;
                }
                let _ = update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    true,
                );
                if gizmo.dragging && !editor_scene.begin_transform_gizmo_drag() {
                    gizmo.cancel_drag();
                }
            }
            GizmoPointerEvent::Move(_) => {
                if !gizmo.dragging {
                    continue;
                }
                if update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    true,
                ) && editor_scene.is_transform_gizmo_drag_active()
                {
                    let _ = apply_gizmo_preview_delta(editor_scene, gizmo, runtime, entity_id);
                }
            }
            GizmoPointerEvent::Release(_) => {
                // A platform is allowed to deliver the final pointer position
                // only with the button-release event.  Sample that position
                // once while the gesture is still active so the last segment
                // (or an entire press/release drag) is not lost.
                if gizmo.dragging
                    && update_gizmo(
                        gizmo,
                        view.world_position,
                        view.world_rotation,
                        &view.view,
                        &view.projection,
                        view.viewport_size,
                        local_pointer,
                        true,
                    )
                    && editor_scene.is_transform_gizmo_drag_active()
                {
                    let _ = apply_gizmo_preview_delta(editor_scene, gizmo, runtime, entity_id);
                }
                let was_dragging = gizmo.dragging;
                let _ = update_gizmo(
                    gizmo,
                    view.world_position,
                    view.world_rotation,
                    &view.view,
                    &view.projection,
                    view.viewport_size,
                    local_pointer,
                    false,
                );
                if was_dragging {
                    match editor_scene.commit_transform_gizmo_drag() {
                        Ok(changed) => scene_changed |= changed,
                        Err(error) => {
                            tracing::error!(%error, "editor gizmo commit failed");
                            scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                        }
                    }
                }
            }
            GizmoPointerEvent::Cancel => unreachable!(),
        }
    }
    scene_changed
}
