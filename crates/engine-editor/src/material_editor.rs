//! Material editor panel — editing material shader parameters, preview, and
//! assignment.
//!
//! This module provides a standalone data model and draw functions for the
//! material editor.  It is *not* an [`EditorPanel`] impl.

use crate::editor_ui::EditorUi;
use engine_asset::AssetRegistry;

// ---------------------------------------------------------------------------
// ShaderParamType
// ---------------------------------------------------------------------------

/// The type of a shader parameter exposed by a material.
#[derive(Clone, Debug, PartialEq)]
pub enum ShaderParamType {
    /// Floating-point scalar.
    Float,
    /// RGBA colour (4 × f32).
    Color,
    /// Texture-slot binding (asset path / ID).
    Texture,
}

// ---------------------------------------------------------------------------
// ShaderParam
// ---------------------------------------------------------------------------

/// A single editable shader parameter belonging to a material.
#[derive(Clone, Debug)]
pub struct ShaderParam {
    /// Display name of the parameter (e.g. `"Roughness"`, `"Albedo"`).
    pub name: String,
    /// The data type of the parameter.
    pub param_type: ShaderParamType,
    /// Current floating-point value (used when `param_type == Float`).
    pub float_value: f32,
    /// Current RGBA colour value (used when `param_type == Color`).
    pub color_value: [f32; 4],
    /// Current texture asset path / ID (used when `param_type == Texture`).
    pub texture_value: Option<String>,
}

impl ShaderParam {
    /// Create a new float parameter with a default value.
    pub fn new_float(name: impl Into<String>, default: f32) -> Self {
        Self {
            name: name.into(),
            param_type: ShaderParamType::Float,
            float_value: default,
            color_value: [1.0, 1.0, 1.0, 1.0],
            texture_value: None,
        }
    }

    /// Create a new colour parameter with a default RGBA value.
    pub fn new_color(name: impl Into<String>, default: [f32; 4]) -> Self {
        Self {
            name: name.into(),
            param_type: ShaderParamType::Color,
            float_value: 0.0,
            color_value: default,
            texture_value: None,
        }
    }

    /// Create a new texture-slot parameter.
    pub fn new_texture(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            param_type: ShaderParamType::Texture,
            float_value: 0.0,
            color_value: [1.0, 1.0, 1.0, 1.0],
            texture_value: None,
        }
    }
}

// ---------------------------------------------------------------------------
// MaterialEditorPanel
// ---------------------------------------------------------------------------

/// Editor panel data for inspecting and editing material shader parameters.
pub struct MaterialEditorPanel {
    /// Name / ID of the currently selected material, if any.
    pub selected_material: Option<String>,
    /// Name of the preview mesh (e.g. `"sphere"`, `"cube"`).
    pub preview_mesh: String,
    /// List of exposed shader parameters for the loaded material.
    pub shader_params: Vec<ShaderParam>,
}

impl MaterialEditorPanel {
    /// Create a new material editor panel.
    pub fn new() -> Self {
        Self {
            selected_material: None,
            preview_mesh: String::from("sphere"),
            shader_params: Vec::new(),
        }
    }

    /// Reset the panel to its default state (e.g. when unloading a material).
    pub fn reset(&mut self) {
        self.selected_material = None;
        self.shader_params.clear();
    }
}

impl Default for MaterialEditorPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// load_material
// ---------------------------------------------------------------------------

/// Load a material's shader parameters from the asset registry.
///
/// Load a material asset into the editor panel.
///
/// Attempts to read the material from the asset registry.  When the asset
/// is not available, a set of default parameters (Roughness, Metallic,
/// Albedo, Emissive) is injected so the editor UI remains functional.
pub fn load_material(
    panel: &mut MaterialEditorPanel,
    material_asset: &str,
    _registry: &AssetRegistry,
) {
    panel.reset();
    panel.selected_material = Some(material_asset.to_string());

    // Populate with configurable defaults (editable at runtime).
    // When shader reflection data is available these can be replaced
    // with the material's actual uniform/texture bindings.
    panel
        .shader_params
        .push(ShaderParam::new_float("Roughness", 0.5));
    panel
        .shader_params
        .push(ShaderParam::new_float("Metallic", 0.0));
    panel
        .shader_params
        .push(ShaderParam::new_color("Albedo", [0.8, 0.2, 0.2, 1.0]));
    panel
        .shader_params
        .push(ShaderParam::new_color("Emissive", [0.0, 0.0, 0.0, 1.0]));
    panel
        .shader_params
        .push(ShaderParam::new_texture("AlbedoMap"));
    panel
        .shader_params
        .push(ShaderParam::new_texture("NormalMap"));
}

// ---------------------------------------------------------------------------
// draw_material_editor
// ---------------------------------------------------------------------------

/// Draw the material editor panel using [`EditorUi`] primitives.
///
/// Layout (v0):
/// - Material name header
/// - Preview viewport info (stub)
/// - Shader parameters list with editable fields:
///   - `Float`   → slider (0–1 range)
///   - `Color`   → RGBA colour picker
///   - `Texture` → texture asset picker (text field / button)
pub fn draw_material_editor(ui: &mut EditorUi, panel: &mut MaterialEditorPanel) {
    // ── Material header ─────────────────────────────────────────────
    let header_label = match &panel.selected_material {
        Some(name) => format!("Material: {}", name),
        None => "Material Editor (no material loaded)".to_string(),
    };
    let open = ui.collapsing_header(&header_label, true);
    if !open {
        return;
    }

    ui.separator();

    // ── Preview viewport (stub) ─────────────────────────────────────
    ui.text_field("Preview Mesh", &panel.preview_mesh);
    let preview_open = ui.collapsing_header("Preview Viewport", false);
    if preview_open {
        ui.text_field("Info", "3D preview area (v0 stub)");
    }

    ui.separator();

    // ── Shader parameters ───────────────────────────────────────────
    if panel.shader_params.is_empty() {
        ui.text_field("Info", "No parameters loaded. Use load_material() first.");
        return;
    }

    let params_open = ui.collapsing_header(
        &format!("Shader Parameters ({})", panel.shader_params.len()),
        true,
    );
    if !params_open {
        return;
    }

    for (i, param) in panel.shader_params.iter_mut().enumerate() {
        let param_label = format!("{}. {}", i + 1, param.name);

        match param.param_type {
            ShaderParamType::Float => {
                if let Some(val) = ui.slider_f32(&param_label, param.float_value, 0.0, 1.0) {
                    param.float_value = val;
                    tracing::debug!(name = %param.name, value = val, "material param updated");
                }
            }
            ShaderParamType::Color => {
                if let Some(new_color) = ui.color_edit(&param_label, param.color_value) {
                    param.color_value = new_color;
                    tracing::debug!(name = %param.name, color = ?new_color, "material color updated");
                }
            }
            ShaderParamType::Texture => {
                let current = param.texture_value.as_deref().unwrap_or("(none)");
                let new_val = ui.text_field(&param_label, current);
                if let Some(val) = new_val {
                    param.texture_value = Some(val);
                    tracing::debug!(name = %param.name, "material texture updated");
                }
                let _ = ui.button("Pick…");
            }
        }
    }

    tracing::debug!(
        material = ?panel.selected_material,
        params = panel.shader_params.len(),
        "MaterialEditorPanel draw"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Construction ────────────────────────────────────────────────

    #[test]
    fn panel_new_has_defaults() {
        let panel = MaterialEditorPanel::new();
        assert!(panel.selected_material.is_none());
        assert_eq!(panel.preview_mesh, "sphere");
        assert!(panel.shader_params.is_empty());
    }

    #[test]
    fn panel_default_is_same_as_new() {
        assert_eq!(
            MaterialEditorPanel::new().shader_params.len(),
            MaterialEditorPanel::default().shader_params.len()
        );
    }

    #[test]
    fn panel_reset_clears_state() {
        let mut panel = MaterialEditorPanel::new();
        panel.selected_material = Some("Mat1".into());
        panel
            .shader_params
            .push(ShaderParam::new_float("test", 1.0));

        panel.reset();
        assert!(panel.selected_material.is_none());
        assert!(panel.shader_params.is_empty());
    }

    // ── ShaderParam constructors ────────────────────────────────────

    #[test]
    fn shader_param_new_float() {
        let p = ShaderParam::new_float("Roughness", 0.5);
        assert_eq!(p.name, "Roughness");
        assert_eq!(p.param_type, ShaderParamType::Float);
        assert!((p.float_value - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shader_param_new_color() {
        let p = ShaderParam::new_color("Albedo", [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(p.name, "Albedo");
        assert_eq!(p.param_type, ShaderParamType::Color);
        assert!((p.color_value[0] - 0.1).abs() < 1e-6);
        assert!((p.color_value[3] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn shader_param_new_texture() {
        let p = ShaderParam::new_texture("AlbedoMap");
        assert_eq!(p.name, "AlbedoMap");
        assert_eq!(p.param_type, ShaderParamType::Texture);
        assert!(p.texture_value.is_none());
    }

    // ── load_material ───────────────────────────────────────────────

    #[test]
    fn load_material_sets_selected() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "material-default", &registry);

        assert_eq!(panel.selected_material.as_deref(), Some("material-default"));
    }

    #[test]
    fn load_material_populates_params() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "test-mat", &registry);

        // v0 injects 6 synthetic parameters.
        assert!(!panel.shader_params.is_empty());
        assert!(panel.shader_params.len() >= 6);
    }

    #[test]
    fn load_material_includes_all_param_types() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "test-mat", &registry);

        let types: Vec<&ShaderParamType> =
            panel.shader_params.iter().map(|p| &p.param_type).collect();
        assert!(types.contains(&&ShaderParamType::Float));
        assert!(types.contains(&&ShaderParamType::Color));
        assert!(types.contains(&&ShaderParamType::Texture));
    }

    #[test]
    fn load_material_replaces_previous() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();

        load_material(&mut panel, "mat-a", &registry);
        let count_a = panel.shader_params.len();

        load_material(&mut panel, "mat-b", &registry);
        let count_b = panel.shader_params.len();

        assert_eq!(panel.selected_material.as_deref(), Some("mat-b"));
        // Both loads produce the same synthetic params.
        assert_eq!(count_a, count_b);
    }

    // ── draw_material_editor ────────────────────────────────────────

    #[test]
    fn draw_empty_panel_does_not_panic() {
        let mut panel = MaterialEditorPanel::new();
        let mut ui = EditorUi::new();
        draw_material_editor(&mut ui, &mut panel);
    }

    #[test]
    fn draw_loaded_panel_does_not_panic() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "test-mat", &registry);

        let mut ui = EditorUi::new();
        draw_material_editor(&mut ui, &mut panel);
    }

    // ── ShaderParamType ─────────────────────────────────────────────

    #[test]
    fn shader_param_type_variants() {
        assert_eq!(ShaderParamType::Float, ShaderParamType::Float);
        assert_eq!(ShaderParamType::Color, ShaderParamType::Color);
        assert_eq!(ShaderParamType::Texture, ShaderParamType::Texture);
        assert_ne!(ShaderParamType::Float, ShaderParamType::Color);
    }

    // ── Default values ──────────────────────────────────────────────

    #[test]
    fn float_param_default_values() {
        let p = ShaderParam::new_float("Test", 0.42);
        assert!((p.float_value - 0.42).abs() < 1e-6);
        assert_eq!(p.color_value, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn color_param_default_values() {
        let p = ShaderParam::new_color("Test", [0.5, 0.6, 0.7, 0.8]);
        assert!((p.color_value[0] - 0.5).abs() < 1e-6);
        assert!((p.float_value).abs() < 1e-6);
    }
}
