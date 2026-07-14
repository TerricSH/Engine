use crate::{Diagnostic, DiagnosticSeverity, LightKind, RenderFrameInput, ShadowMode, ViewCompose};
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
        ("ToneMap", tone_map.len()),
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

    if let (Some(&opaque), Some(&tone_map), Some(&present)) =
        (opaque.first(), tone_map.first(), present.first())
    {
        let shadow_is_ordered = shadow.first().map_or(true, |shadow| *shadow < opaque);
        let present_is_final = enabled
            .last()
            .is_some_and(|(index, kind)| *index == present && *kind == "Present");
        if !(shadow_is_ordered && opaque < tone_map && tone_map < present && present_is_final) {
            diagnostics.push(
                Diagnostic::new(
                    "RV0019",
                    DiagnosticSeverity::Error,
                    "engine-renderer",
                    "render graph order must be DirectionalShadow (optional), OpaquePbrForward, optional custom passes, ToneMap, then terminal Present",
                )
                .contract("RendererInput-v0", input.contract_version.clone())
                .path("render_options.pass_graph_config.passes"),
            );
        }
    }
}
