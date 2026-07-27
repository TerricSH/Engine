use crate::{
    BonePaletteLayout, Diagnostic, DiagnosticSeverity, LightKind, PassGraphOutputMode,
    RenderFrameInput, ShadowMode, ToneMapping, Transparency, ViewCompose,
};
use std::collections::BTreeSet;

pub fn validate_frame_input(input: &RenderFrameInput) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if !input.contract_version.starts_with("RendererInput-v0") {
        diagnostics.push(
            Diagnostic::new(
                "RV0012",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "renderer input contract version is not RendererInput-v0",
            )
            .contract("RendererInput-v0", input.contract_version.clone()),
        );
    }
    if input.views.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                "RV0013",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "renderer input contains no render views",
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("views"),
        );
    }
    if input
        .render_options
        .exposure_ev100
        .is_some_and(|exposure| !exposure.is_finite())
    {
        diagnostics.push(
            Diagnostic::new(
                "RV0022",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "render_options.exposure_ev100 must be finite when provided",
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.exposure_ev100"),
        );
    }
    if let Some(message) = validate_post_process_settings(&input.render_options.post_process) {
        diagnostics.push(
            Diagnostic::new(
                "RV0025",
                DiagnosticSeverity::Error,
                "engine-renderer",
                message,
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.post_process"),
        );
    }
    if let Some(message) = validate_environment_settings(&input.render_options.environment) {
        diagnostics.push(
            Diagnostic::new(
                "RV0026",
                DiagnosticSeverity::Error,
                "engine-renderer",
                message,
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.environment"),
        );
    }

    let mut view_ids = BTreeSet::new();
    for (view_index, view) in input.views.iter().enumerate() {
        if !view_ids.insert(view.view_id) {
            diagnostics.push(
                Diagnostic::new(
                    "RV0014",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "RenderView.view_id values must be unique",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path("views.view_id"),
            );
        }
        for (field, rect) in [
            ("viewport", view.viewport),
            ("viewport_rect_normalized", view.viewport_rect_normalized),
        ] {
            if !rect.is_valid_normalized() {
                diagnostics.push(
                    Diagnostic::new(
                        "RV0023",
                        DiagnosticSeverity::Error,
                        "engine-renderer",
                        "RenderView viewport rectangles must be finite, positive, and contained in [0, 1]",
                    )
                    .contract("RendererInput-v0", input.contract_version.clone())
                    .path(format!("views[{view_index}].{field}")),
                );
            }
        }
    }
    for view in &input.views {
        if let ViewCompose::Overlay { base_view_id, .. } = view.compose {
            if !view_ids.contains(&base_view_id) {
                diagnostics.push(
                    Diagnostic::new(
                        "RV0007",
                        DiagnosticSeverity::Warning,
                        "engine-renderer",
                        "overlay render view references a missing base view",
                    )
                    .contract("RendererInput-v0", input.contract_version.clone()),
                );
            }
        }
    }

    for (item_index, item) in input.skinned_items.iter().enumerate() {
        let count = match item.bone_palette_layout {
            BonePaletteLayout::Full4x4 { count } => count,
            BonePaletteLayout::Packed3x4 { .. } => {
                diagnostics.push(
                    Diagnostic::new(
                        "RV0020",
                        DiagnosticSeverity::Error,
                        "engine-renderer",
                        "Packed3x4 bone palettes are not supported by the runtime backends",
                    )
                    .contract("RendererInput-v0", input.contract_version.clone())
                    .path(format!("skinned_items[{item_index}].bone_palette_layout")),
                );
                continue;
            }
        };
        let palette_is_valid = count as usize == item.bone_palette.len()
            && (1..=64).contains(&item.bone_palette.len())
            && item
                .bone_palette
                .iter()
                .flatten()
                .all(|value| value.is_finite());
        if !palette_is_valid {
            diagnostics.push(
                Diagnostic::new(
                    "RV0020",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "skinned item requires 1..=64 finite Full4x4 bones and a matching count",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("skinned_items[{item_index}].bone_palette")),
            );
        }
        let morph_is_valid = match &item.morph_target_set {
            Some(_) => {
                !item.morph_weights.is_empty()
                    && item.morph_weights.len() <= crate::MAX_MORPH_TARGETS
                    && item
                        .morph_weights
                        .iter()
                        .all(|weight| weight.is_finite() && (-1.0..=1.0).contains(weight))
            }
            None => item.morph_weights.is_empty(),
        };
        if !morph_is_valid {
            diagnostics.push(
                Diagnostic::new(
                    "RV0027",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "morph weights require a target set, 1..=8 finite values, and range [-1, 1]",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("skinned_items[{item_index}].morph_weights")),
            );
        }
    }

    for (batch_index, batch) in input.particle_batches.iter().enumerate() {
        let bounds_valid = batch
            .bounds
            .min
            .into_iter()
            .chain(batch.bounds.max)
            .all(f32::is_finite)
            && (0..3).all(|axis| batch.bounds.min[axis] <= batch.bounds.max[axis]);
        let cpu_instances_valid = !batch.instances.is_empty()
            && batch.instances.len() <= 65_536
            && batch.instances.iter().all(|instance| {
                instance
                    .position
                    .into_iter()
                    .chain([
                        instance.size,
                        instance.rotation_radians,
                        instance.normalized_age,
                    ])
                    .all(f32::is_finite)
                    && instance.size >= 0.0
                    && (0.0..=1.0).contains(&instance.normalized_age)
            });
        let gpu_valid = batch.gpu_simulation.is_some_and(|simulation| {
            [
                simulation.elapsed,
                simulation.emission_duration,
                simulation.emission_rate,
                simulation.lifetime_min,
                simulation.lifetime_max,
                simulation.speed_min,
                simulation.speed_max,
                simulation.start_size,
                simulation.end_size,
                simulation.spread_angle_radians,
                simulation.drag,
                simulation.turbulence_strength,
                simulation.turbulence_frequency,
                simulation.angular_velocity_min,
                simulation.angular_velocity_max,
            ]
            .into_iter()
            .chain(simulation.origin)
            .chain(simulation.direction)
            .chain(simulation.acceleration)
            .all(f32::is_finite)
                && simulation.elapsed >= 0.0
                && simulation.emission_duration >= 0.0
                && simulation.emission_rate >= 0.0
                && simulation.max_particles > 0
                && simulation.max_particles <= 1_048_576
                && simulation.spawned_count() <= u64::from(u32::MAX)
                && simulation.lifetime_min > 0.0
                && simulation.lifetime_max >= simulation.lifetime_min
                && simulation.speed_max >= simulation.speed_min
                && simulation.start_size >= 0.0
                && simulation.end_size >= 0.0
                && simulation.drag >= 0.0
                && simulation.turbulence_strength >= 0.0
                && simulation.turbulence_frequency > 0.0
                && (0.0..=std::f32::consts::PI).contains(&simulation.spread_angle_radians)
                && simulation.angular_velocity_max >= simulation.angular_velocity_min
        });
        let instances_valid = match batch.gpu_simulation {
            Some(_) => batch.instances.is_empty() && gpu_valid,
            None => cpu_instances_valid,
        };
        if batch.mesh.id.trim().is_empty()
            || batch.material.id.trim().is_empty()
            || batch.render_layer.trim().is_empty()
            || !bounds_valid
            || !instances_valid
        {
            diagnostics.push(
                Diagnostic::new(
                    "RV0028",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "particle batches require mesh/material/layer ids, ordered finite bounds, and either 1..=65536 CPU instances or one valid GPU simulation up to 1048576 slots",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("particle_batches[{batch_index}]")),
            );
        }
    }

    for (material_index, material) in input.materials.iter().enumerate() {
        if let Transparency::Masked { cutoff } = &material.transparency {
            if !cutoff.is_finite() || !(0.0..=1.0).contains(cutoff) {
                diagnostics.push(
                    Diagnostic::new(
                        "RV0024",
                        DiagnosticSeverity::Error,
                        "engine-renderer",
                        "masked material cutoff must be finite and in [0, 1]",
                    )
                    .contract("RendererInput-v0", input.contract_version.clone())
                    .path(format!("materials[{material_index}].transparency.cutoff")),
                );
            }
        }
    }

    // Light validation diagnostics (Gate 3 acceptance)
    for (light_idx, light) in input.lights.iter().enumerate() {
        // ShadowMode::Hard or Soft on point/spot lights produces diagnostic
        // and is downgraded (the frame never aborts — Warning only)
        if matches!(light.kind, LightKind::Point | LightKind::Spot)
            && matches!(light.shadow_mode, ShadowMode::Hard | ShadowMode::Soft)
        {
            let entity_id = light
                .entity
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            diagnostics.push(
                Diagnostic::new(
                    "RV0015",
                    DiagnosticSeverity::Warning,
                    "engine-renderer",
                    format!(
                        "ShadowMode::{:?} is not supported for {:?} light (entity {}); downgraded to Off",
                        light.shadow_mode, light.kind, entity_id
                    ),
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("lights[{light_idx}].shadow_mode")),
            );
        }

        // Intensity must be positive
        if light.intensity <= 0.0 {
            diagnostics.push(
                Diagnostic::new(
                    "RV0016",
                    DiagnosticSeverity::Warning,
                    "engine-renderer",
                    format!(
                        "Light intensity must be positive (got {}) for {:?} light",
                        light.intensity, light.kind
                    ),
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path(format!("lights[{light_idx}].intensity")),
            );
        }
    }

    validate_pass_graph_config(input, &mut diagnostics);

    diagnostics
}

pub fn validate_post_process_settings(
    settings: &crate::PostProcessSettings,
) -> Option<&'static str> {
    let bloom = settings.bloom;
    if [bloom.threshold, bloom.intensity, bloom.radius]
        .into_iter()
        .any(|value| !value.is_finite() || value < 0.0)
    {
        return Some("bloom threshold, intensity, and radius must be finite and non-negative");
    }

    let grading = settings.color_grading;
    if grading
        .color_filter
        .into_iter()
        .chain([grading.saturation, grading.contrast])
        .chain(grading.lift)
        .chain(grading.gamma)
        .chain(grading.gain)
        .any(|value| !value.is_finite())
    {
        return Some("color-grading values must be finite");
    }
    if grading.saturation < 0.0
        || grading.contrast < 0.0
        || grading.gamma.into_iter().any(|value| value <= 0.0)
        || grading.gain.into_iter().any(|value| value < 0.0)
    {
        return Some(
            "color-grading saturation/contrast/gain must be non-negative and gamma must be positive",
        );
    }

    let vignette = settings.vignette;
    if [vignette.intensity, vignette.smoothness, vignette.roundness]
        .into_iter()
        .any(|value| !value.is_finite())
        || !(0.0..=1.0).contains(&vignette.intensity)
        || !(f32::EPSILON..=1.0).contains(&vignette.smoothness)
        || !(0.0..=1.0).contains(&vignette.roundness)
    {
        return Some("vignette intensity/roundness must be in [0, 1] and smoothness in (0, 1]");
    }
    None
}

pub fn validate_environment_settings(
    settings: &crate::EnvironmentSettings,
) -> Option<&'static str> {
    if !settings.intensity.is_finite()
        || settings.intensity < 0.0
        || !settings.rotation_radians.is_finite()
    {
        return Some("environment intensity must be finite/non-negative and rotation finite");
    }
    for probe in &settings.reflection_probes {
        let bounds_valid = probe
            .position
            .into_iter()
            .chain(probe.half_extents)
            .chain([probe.blend_distance])
            .all(f32::is_finite)
            && probe.half_extents.into_iter().all(|value| value > 0.0)
            && probe.blend_distance >= 0.0;
        if !bounds_valid || probe.environment_map.id.trim().is_empty() {
            return Some(
                "reflection probes require a map id, finite bounds, positive extents, and non-negative blend distance",
            );
        }
    }
    None
}

fn validate_pass_graph_config(input: &RenderFrameInput, diagnostics: &mut Vec<Diagnostic>) {
    let config = &input.render_options.pass_graph_config;
    if !config.enabled {
        return;
    }

    let enabled: Vec<(usize, &str)> = config
        .passes
        .iter()
        .enumerate()
        .filter(|(_, pass)| pass.enabled)
        .map(|(index, pass)| (index, pass.kind.as_str()))
        .collect();
    let positions = |kind: &str| {
        enabled
            .iter()
            .filter(|(_, candidate)| *candidate == kind)
            .map(|(index, _)| *index)
            .collect::<Vec<_>>()
    };
    let opaque = positions("OpaquePbrForward");
    let tone_map = positions("ToneMap");
    let present = positions("Present");
    let shadow = positions("DirectionalShadow");

    for (kind, found) in [
        ("OpaquePbrForward", opaque.len()),
        ("Present", present.len()),
    ] {
        if found != 1 {
            diagnostics.push(
                Diagnostic::new(
                    "RV0017",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    format!(
                        "enabled render graph must contain exactly one {kind} pass; found {found}"
                    ),
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path("render_options.pass_graph_config.passes"),
            );
        }
    }
    let direct_to_swapchain = config.output_mode == PassGraphOutputMode::DirectToSwapchain;
    let expected_tone_map_count = usize::from(!direct_to_swapchain);
    if tone_map.len() != expected_tone_map_count {
        diagnostics.push(
            Diagnostic::new(
                "RV0017",
                DiagnosticSeverity::Error,
                "engine-renderer",
                format!(
                    "render graph output mode {:?} must contain exactly {expected_tone_map_count} enabled ToneMap pass; found {}",
                    config.output_mode,
                    tone_map.len()
                ),
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.pass_graph_config.passes"),
        );
    }
    let direct_output_has_tone_mapping =
        direct_to_swapchain && input.render_options.tone_mapping != ToneMapping::None;
    if direct_output_has_tone_mapping {
        diagnostics.push(
            Diagnostic::new(
                "RV0017",
                DiagnosticSeverity::Error,
                "engine-renderer",
                format!(
                    "DirectToSwapchain output cannot apply ToneMapping::{:?}; select ToneMapping::None or use HdrThenToneMap",
                    input.render_options.tone_mapping
                ),
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.tone_mapping"),
        );
    }
    if direct_to_swapchain
        && input.render_options.transparency_mode == crate::TransparencyMode::WeightedBlendedOit
    {
        diagnostics.push(
            Diagnostic::new(
                "RV0061",
                DiagnosticSeverity::Error,
                "renderer.contract",
                "weighted blended OIT requires HdrThenToneMap output so accumulation targets can be resolved",
            )
            .path("render_options.transparency_mode"),
        );
    }
    if shadow.len() > 1 {
        diagnostics.push(
            Diagnostic::new(
                "RV0018",
                DiagnosticSeverity::Error,
                "engine-renderer",
                "enabled render graph may contain at most one DirectionalShadow pass",
            )
            .contract("RendererInput-v0", input.contract_version.clone())
            .path("render_options.pass_graph_config.passes"),
        );
    }

    let required_pass_counts_are_valid = opaque.len() == 1
        && present.len() == 1
        && tone_map.len() == expected_tone_map_count
        && shadow.len() <= 1
        && !direct_output_has_tone_mapping;
    if required_pass_counts_are_valid {
        let opaque = opaque[0];
        let present = present[0];
        let shadow_is_ordered = shadow.first().is_none_or(|shadow| *shadow < opaque);
        let tone_map_is_ordered = tone_map
            .first()
            .is_none_or(|tone_map| opaque < *tone_map && *tone_map < present);
        let present_is_final = enabled
            .last()
            .is_some_and(|(index, kind)| *index == present && *kind == "Present");
        if !(shadow_is_ordered && opaque < present && tone_map_is_ordered && present_is_final) {
            diagnostics.push(
                Diagnostic::new(
                    "RV0019",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "render graph order must be DirectionalShadow (optional), OpaquePbrForward, optional custom passes, ToneMap (for HdrThenToneMap output), then terminal Present",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path("render_options.pass_graph_config.passes"),
            );
        }
    }
}

/// Validate the serialized render-graph portion of scene settings without
/// requiring a fabricated camera or drawable frame.
///
/// Runtime frame validation calls the same implementation, so editor
/// authoring and rendering cannot disagree about required passes or output
/// mode/tone-mapping compatibility.
pub fn validate_pass_graph_settings(
    config: &crate::PassGraphConfig,
    tone_mapping: ToneMapping,
) -> Vec<Diagnostic> {
    let mut input = RenderFrameInput::empty(0);
    input.render_options.pass_graph_config = config.clone();
    input.render_options.tone_mapping = tone_mapping;
    let mut diagnostics = Vec::new();
    validate_pass_graph_config(&input, &mut diagnostics);
    diagnostics
}
