use crate::{
    BonePaletteLayout, Diagnostic, DiagnosticSeverity, LightKind, PassGraphOutputMode,
    RenderFrameInput, ShadowMode, ToneMapping, ViewCompose,
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

    let mut view_ids = BTreeSet::new();
    for view in &input.views {
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
