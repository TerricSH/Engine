use engine_renderer::{
    AxisAlignedBox, ClearFlags, LightItem, LightKind, Rect, RenderFrameInput, RenderView,
    RenderableItem, ShadowMode, ViewCompose, IDENTITY_MAT4,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::scene::{ECS_SCENE_CONTRACT, Scene};
use crate::validation::{
    active_camera_entity, asset_field, bool_field, enabled_component, f32_field,
    light_kind_field, string_field, validate_scene, vec3_field,
};

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
