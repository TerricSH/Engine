//! Material editor panel for editing and persisting material shader parameters.
//!
//! This module provides the UI-toolkit-independent data model exposed to the
//! React editor through the typed editor protocol.

use engine_asset::cook::{MaterialSource, MATERIAL_SOURCE_SCHEMA};
use engine_asset::AssetRegistry;
use engine_renderer::MaterialUpload;
use engine_serialize::AssetId;

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
    /// List of exposed shader parameters for the loaded material.
    pub shader_params: Vec<ShaderParam>,
    save_access: MaterialSaveAccess,
    save_requested: bool,
    save_status: Option<String>,
}

/// Whether the selected material has a project-owned source file that can be
/// persisted by the editor host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialSaveAccess {
    Writable,
    ReadOnly(String),
}

/// A persistence request emitted by the material panel.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSaveRequest {
    pub material_asset: String,
    pub source: MaterialSource,
}

impl MaterialEditorPanel {
    /// Create a new material editor panel.
    pub fn new() -> Self {
        Self {
            selected_material: None,
            shader_params: Vec::new(),
            save_access: MaterialSaveAccess::ReadOnly(
                "Select a project material to enable saving.".to_string(),
            ),
            save_requested: false,
            save_status: None,
        }
    }

    /// Reset the panel to its default state (e.g. when unloading a material).
    pub fn reset(&mut self) {
        self.selected_material = None;
        self.shader_params.clear();
        self.save_access =
            MaterialSaveAccess::ReadOnly("Select a project material to enable saving.".to_string());
        self.save_requested = false;
        self.save_status = None;
    }

    /// Configure whether the host can map the selected asset back to one
    /// unambiguous project Material source entry.
    pub fn set_save_access(&mut self, access: MaterialSaveAccess) {
        self.save_access = access;
        self.save_requested = false;
    }

    pub fn save_access(&self) -> &MaterialSaveAccess {
        &self.save_access
    }

    pub fn save_status(&self) -> Option<&str> {
        self.save_status.as_deref()
    }

    /// Queue persistence from the editor shell.
    pub fn request_save(&mut self) {
        self.save_requested = true;
    }

    pub fn report_save_success(&mut self, message: impl Into<String>) {
        self.save_status = Some(message.into());
    }

    pub fn report_save_failure(&mut self, message: impl Into<String>) {
        self.save_status = Some(format!("Save failed: {}", message.into()));
    }

    /// Consume one explicit `Save Material` click and translate the current
    /// controls into the portable MaterialSource-v0 authoring contract.
    pub fn take_save_request(&mut self) -> Result<Option<MaterialSaveRequest>, String> {
        if !self.save_requested {
            return Ok(None);
        }
        self.save_requested = false;

        if let MaterialSaveAccess::ReadOnly(reason) = &self.save_access {
            return Err(reason.clone());
        }
        let material_asset = self
            .selected_material
            .clone()
            .filter(|asset| !asset.trim().is_empty())
            .ok_or_else(|| "no material is selected".to_string())?;
        let base_color = self.color_parameter("Albedo")?;
        let metallic = self.float_parameter("Metallic")?;
        let roughness = self.float_parameter("Roughness")?;
        let ambient_occlusion = self.float_parameter("Ambient Occlusion")?;
        for (field, value) in [
            ("base_color[0]", base_color[0]),
            ("base_color[1]", base_color[1]),
            ("base_color[2]", base_color[2]),
            ("base_color[3]", base_color[3]),
            ("metallic", metallic),
            ("roughness", roughness),
            ("ambient_occlusion", ambient_occlusion),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "material parameter '{field}' must be finite and in the range 0..=1"
                ));
            }
        }

        Ok(Some(MaterialSaveRequest {
            material_asset,
            source: MaterialSource {
                schema: MATERIAL_SOURCE_SCHEMA.to_string(),
                base_color,
                metallic,
                roughness,
                ambient_occlusion,
                base_color_texture: self.texture_parameter("AlbedoMap"),
                transparency: "Opaque".to_string(),
                double_sided: false,
            },
        }))
    }

    fn float_parameter(&self, name: &str) -> Result<f32, String> {
        self.shader_params
            .iter()
            .find(|parameter| {
                parameter.param_type == ShaderParamType::Float
                    && parameter.name.eq_ignore_ascii_case(name)
            })
            .map(|parameter| parameter.float_value)
            .ok_or_else(|| format!("material parameter '{name}' is missing or has the wrong type"))
    }

    fn color_parameter(&self, name: &str) -> Result<[f32; 4], String> {
        self.shader_params
            .iter()
            .find(|parameter| {
                parameter.param_type == ShaderParamType::Color
                    && parameter.name.eq_ignore_ascii_case(name)
            })
            .map(|parameter| parameter.color_value)
            .ok_or_else(|| format!("material parameter '{name}' is missing or has the wrong type"))
    }

    fn texture_parameter(&self, name: &str) -> Option<String> {
        self.shader_params
            .iter()
            .find(|parameter| {
                parameter.param_type == ShaderParamType::Texture
                    && parameter.name.eq_ignore_ascii_case(name)
            })
            .and_then(|parameter| parameter.texture_value.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("(none)"))
            .map(str::to_string)
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
/// The editor only exposes data owned by the typed runtime registry. Missing
/// or failed material assets remain visibly read-only; no synthetic authoring
/// parameters are injected into the production UI.
pub fn load_material(
    panel: &mut MaterialEditorPanel,
    material_asset: &str,
    registry: &AssetRegistry,
) {
    panel.reset();
    panel.selected_material = Some(material_asset.to_string());

    if let Some(handle) = registry.get::<MaterialUpload>(&AssetId::new(material_asset)) {
        let material = handle.get();
        panel
            .shader_params
            .push(ShaderParam::new_float("Roughness", material.roughness));
        panel
            .shader_params
            .push(ShaderParam::new_float("Metallic", material.metallic));
        panel.shader_params.push(ShaderParam::new_float(
            "Ambient Occlusion",
            material.ambient_occlusion,
        ));
        panel
            .shader_params
            .push(ShaderParam::new_color("Albedo", material.base_color));
        let mut base_color_texture = ShaderParam::new_texture("AlbedoMap");
        base_color_texture.texture_value = material
            .base_color_texture
            .as_ref()
            .map(|texture| texture.id.clone());
        panel.shader_params.push(base_color_texture);
        return;
    }

    let message = format!(
        "Material '{material_asset}' is not available in AssetRegistry; reimport it and resolve its diagnostics before editing."
    );
    panel.save_access = MaterialSaveAccess::ReadOnly(message.clone());
    panel.save_status = Some(message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material_registry(material_id: &str, base_color: [f32; 4]) -> AssetRegistry {
        use engine_renderer::Transparency;

        let mut registry = AssetRegistry::new();
        registry.insert_typed(
            AssetId::new(material_id),
            MaterialUpload {
                material_id: AssetId::new(material_id),
                base_color,
                metallic: 0.25,
                roughness: 0.75,
                ambient_occlusion: 0.9,
                base_color_texture: Some(AssetId::new("texture-project")),
                transparency: Transparency::Opaque,
                double_sided: false,
                content_hash: [9; 32],
            },
        );
        registry
    }

    fn save_test_registry() -> AssetRegistry {
        material_registry("mat-project", [0.1, 0.2, 0.3, 1.0])
    }

    // ── Construction ────────────────────────────────────────────────

    #[test]
    fn panel_new_has_defaults() {
        let panel = MaterialEditorPanel::new();
        assert!(panel.selected_material.is_none());
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
    fn missing_material_stays_selected_but_cannot_be_edited() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "material-default", &registry);

        assert_eq!(panel.selected_material.as_deref(), Some("material-default"));
        assert!(panel.shader_params.is_empty());
        assert!(matches!(
            panel.save_access(),
            MaterialSaveAccess::ReadOnly(reason) if reason.contains("AssetRegistry")
        ));
        assert!(panel
            .save_status()
            .is_some_and(|status| status.contains("reimport")));
    }

    #[test]
    fn missing_material_does_not_inject_synthetic_parameters() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "test-mat", &registry);

        assert!(panel.shader_params.is_empty());
    }

    #[test]
    fn load_material_replaces_previous() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();

        load_material(&mut panel, "mat-a", &registry);
        assert!(panel.shader_params.is_empty());

        let typed = material_registry("mat-b", [0.2, 0.3, 0.4, 1.0]);
        load_material(&mut panel, "mat-b", &typed);

        assert_eq!(panel.selected_material.as_deref(), Some("mat-b"));
        assert!(!panel.shader_params.is_empty());
        assert!(panel.save_status().is_none());
    }

    #[test]
    fn load_material_uses_typed_runtime_values() {
        use engine_renderer::{MaterialUpload, Transparency};

        let mut registry = AssetRegistry::new();
        registry.insert_typed(
            AssetId::new("mat-runtime"),
            MaterialUpload {
                material_id: AssetId::new("mat-runtime"),
                base_color: [0.1, 0.3, 0.7, 1.0],
                metallic: 0.8,
                roughness: 0.2,
                ambient_occlusion: 0.6,
                base_color_texture: Some(AssetId::new("tex-runtime")),
                transparency: Transparency::Opaque,
                double_sided: false,
                content_hash: [7; 32],
            },
        );
        let mut panel = MaterialEditorPanel::new();

        load_material(&mut panel, "mat-runtime", &registry);

        assert!(panel.shader_params.iter().any(|param| {
            param.name == "Roughness" && (param.float_value - 0.2).abs() < f32::EPSILON
        }));
        assert!(panel
            .shader_params
            .iter()
            .any(|param| { param.name == "Albedo" && param.color_value == [0.1, 0.3, 0.7, 1.0] }));
        assert!(panel.shader_params.iter().any(|param| {
            param.name == "AlbedoMap" && param.texture_value.as_deref() == Some("tex-runtime")
        }));
    }

    #[test]
    fn save_material_emits_material_source() {
        let mut panel = MaterialEditorPanel::new();
        load_material(&mut panel, "mat-project", &save_test_registry());
        panel.set_save_access(MaterialSaveAccess::Writable);
        panel
            .shader_params
            .iter_mut()
            .find(|parameter| parameter.name == "Roughness")
            .unwrap()
            .float_value = 0.33;
        panel
            .shader_params
            .iter_mut()
            .find(|parameter| parameter.name == "Albedo")
            .unwrap()
            .color_value = [0.8, 0.6, 0.4, 1.0];

        panel.request_save();

        let request = panel.take_save_request().unwrap().expect("save request");

        assert_eq!(request.material_asset, "mat-project");
        assert_eq!(request.source.schema, MATERIAL_SOURCE_SCHEMA);
        assert_eq!(request.source.roughness, 0.33);
        assert_eq!(request.source.base_color, [0.8, 0.6, 0.4, 1.0]);
        assert_eq!(
            request.source.base_color_texture.as_deref(),
            Some("texture-project")
        );
    }

    #[test]
    fn read_only_material_does_not_emit_save_request() {
        let mut panel = MaterialEditorPanel::new();
        load_material(&mut panel, "mat-default", &save_test_registry());
        panel.set_save_access(MaterialSaveAccess::ReadOnly(
            "Built-in material 'mat-default' is read-only.".to_string(),
        ));

        panel.request_save();

        assert!(panel.take_save_request().is_err());
        assert!(matches!(
            panel.save_access(),
            MaterialSaveAccess::ReadOnly(reason) if reason.contains("Built-in")
        ));
    }

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
