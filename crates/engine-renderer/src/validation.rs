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

    diagnostics
}
