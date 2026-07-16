//! Material editor panel — editing material shader parameters, preview, and
//! assignment.
//!
//! This module provides a standalone data model and draw functions for the
//! material editor.  It is *not* an [`EditorPanel`] impl.

use crate::editor_ui::EditorUi;
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
    /// Name of the preview mesh (e.g. `"sphere"`, `"cube"`).
    pub preview_mesh: String,
    /// List of exposed shader parameters for the loaded material.
    pub shader_params: Vec<ShaderParam>,
    /// Texture produced by the renderer's offscreen material-preview pass.
    pub preview_texture: Option<String>,
    preview_dirty: bool,
    preview_revision: u64,
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

/// A persistence request emitted by the material panel. The preview texture
/// is intentionally absent: only authoring parameters enter MaterialSource-v0.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSaveRequest {
    pub material_asset: String,
    pub source: MaterialSource,
}

/// Work item consumed by the editor host's offscreen preview renderer.
#[derive(Clone, Debug)]
pub struct MaterialPreviewRequest {
    pub material_asset: String,
    pub preview_mesh: String,
    pub shader_params: Vec<ShaderParam>,
    pub revision: u64,
}

/// Render a deterministic shaded material preview into an sRGB RGBA8 image.
///
/// This software fallback keeps the editor preview functional through the
/// portable texture-upload API until the renderer exposes sampled offscreen
/// render targets. The resulting texture is still displayed by the normal GPU
/// UI path.
pub fn render_material_preview_rgba8(
    request: &MaterialPreviewRequest,
    width: u32,
    height: u32,
) -> Vec<u8> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let float_param = |name: &str, fallback: f32| {
        request
            .shader_params
            .iter()
            .find(|param| {
                param.param_type == ShaderParamType::Float && param.name.eq_ignore_ascii_case(name)
            })
            .map_or(fallback, |param| param.float_value)
    };
    let color_param = |name: &str, fallback: [f32; 4]| {
        request
            .shader_params
            .iter()
            .find(|param| {
                param.param_type == ShaderParamType::Color && param.name.eq_ignore_ascii_case(name)
            })
            .map_or(fallback, |param| param.color_value)
    };

    let roughness = float_param("Roughness", 0.5).clamp(0.04, 1.0);
    let metallic = float_param("Metallic", 0.0).clamp(0.0, 1.0);
    let albedo_rgba = color_param("Albedo", [0.8, 0.2, 0.2, 1.0]);
    let emissive_rgba = color_param("Emissive", [0.0, 0.0, 0.0, 1.0]);
    let albedo = glam::Vec3::new(albedo_rgba[0], albedo_rgba[1], albedo_rgba[2]);
    let emissive = glam::Vec3::new(emissive_rgba[0], emissive_rgba[1], emissive_rgba[2]);
    let light = glam::Vec3::new(-0.42, 0.68, 0.60).normalize();
    let view = glam::Vec3::Z;
    let half_vector = (light + view).normalize();
    let f0 = glam::Vec3::splat(0.04).lerp(albedo, metallic);
    let shininess = 4.0 + (1.0 - roughness).powi(2) * 124.0;
    let aspect = width as f32 / height as f32;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);

    for y in 0..height {
        for x in 0..width {
            let px = ((x as f32 + 0.5) / width as f32 * 2.0 - 1.0) * aspect;
            let py = 1.0 - (y as f32 + 0.5) / height as f32 * 2.0;
            let radius = 0.78;
            let distance_squared = px * px + py * py;
            let linear = if distance_squared <= radius * radius {
                let normal = glam::Vec3::new(
                    px / radius,
                    py / radius,
                    (1.0 - distance_squared / (radius * radius)).sqrt(),
                )
                .normalize();
                let diffuse_light = normal.dot(light).max(0.0);
                let ambient = 0.10 + 0.12 * normal.y.max(0.0);
                let diffuse = albedo * (1.0 - metallic) * (ambient + 0.88 * diffuse_light);
                let specular = f0 * normal.dot(half_vector).max(0.0).powf(shininess);
                let rim = (1.0 - normal.dot(view).max(0.0)).powi(3) * 0.08;
                diffuse + specular + glam::Vec3::splat(rim) + emissive
            } else {
                let checker = ((x / 16) + (y / 16)) % 2;
                glam::Vec3::splat(if checker == 0 { 0.055 } else { 0.075 })
            };
            let srgb = linear
                .max(glam::Vec3::ZERO)
                .min(glam::Vec3::ONE)
                .powf(1.0 / 2.2);
            pixels.extend([
                (srgb.x * 255.0).round() as u8,
                (srgb.y * 255.0).round() as u8,
                (srgb.z * 255.0).round() as u8,
                255,
            ]);
        }
    }
    pixels
}

impl MaterialEditorPanel {
    /// Create a new material editor panel.
    pub fn new() -> Self {
        Self {
            selected_material: None,
            preview_mesh: String::from("sphere"),
            shader_params: Vec::new(),
            preview_texture: None,
            preview_dirty: false,
            preview_revision: 0,
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
        self.preview_texture = None;
        self.preview_dirty = false;
        self.save_access =
            MaterialSaveAccess::ReadOnly("Select a project material to enable saving.".to_string());
        self.save_requested = false;
        self.save_status = None;
        // Invalidate any offscreen work that may still be in flight for the
        // material being unloaded.
        self.preview_revision = self.preview_revision.wrapping_add(1);
    }

    /// Return the latest preview request once after material parameters change.
    pub fn take_preview_request(&mut self) -> Option<MaterialPreviewRequest> {
        if !self.preview_dirty {
            return None;
        }
        let material_asset = self.selected_material.clone()?;
        self.preview_dirty = false;
        Some(MaterialPreviewRequest {
            material_asset,
            preview_mesh: self.preview_mesh.clone(),
            shader_params: self.shader_params.clone(),
            revision: self.preview_revision,
        })
    }

    /// Publish an offscreen preview only if it still matches the current edit.
    pub fn complete_preview(&mut self, revision: u64, texture_id: impl Into<String>) -> bool {
        let texture_id = texture_id.into();
        if revision != self.preview_revision
            || self.selected_material.is_none()
            || texture_id.trim().is_empty()
        {
            return false;
        }
        self.preview_texture = Some(texture_id);
        true
    }

    /// Requeue the current request after a transient render/upload failure.
    pub fn fail_preview(&mut self, revision: u64) -> bool {
        if revision != self.preview_revision || self.selected_material.is_none() {
            return false;
        }
        self.preview_dirty = true;
        true
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

    fn invalidate_preview(&mut self) {
        self.preview_revision = self.preview_revision.wrapping_add(1);
        self.preview_dirty = true;
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
        panel.invalidate_preview();
        return;
    }

    // Keep an editable fallback for assets that have not entered the typed
    // runtime registry yet.
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
    panel.invalidate_preview();
}

// ---------------------------------------------------------------------------
// draw_material_editor
// ---------------------------------------------------------------------------

/// Draw the material editor panel using [`EditorUi`] primitives.
///
/// Layout (v0):
/// - Material name header
/// - Texture-backed preview viewport fed by the host's offscreen renderer
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

    match &panel.save_access {
        MaterialSaveAccess::Writable if panel.selected_material.is_some() => {
            if ui.button("Save Material") {
                panel.save_requested = true;
            }
        }
        MaterialSaveAccess::Writable => {
            ui.label_value("Material Save", "No material is selected.");
        }
        MaterialSaveAccess::ReadOnly(reason) => {
            ui.label_value("Material Save", reason);
        }
    }
    if let Some(status) = panel.save_status.as_deref() {
        ui.label_value("Material Save Status", status);
    }

    ui.separator();

    // ── Preview viewport ─────────────────────────────────────────────
    if let Some(preview_mesh) = ui.text_field("Preview Mesh", &panel.preview_mesh) {
        panel.preview_mesh = preview_mesh;
        panel.invalidate_preview();
    }
    let preview_open = ui.collapsing_header("Preview Viewport", false);
    if preview_open {
        if let Some(texture) = panel.preview_texture.as_deref() {
            ui.image(texture, 180.0);
        } else if panel.selected_material.is_some() {
            ui.text_field("Preview", "Waiting for offscreen preview render");
        } else {
            ui.text_field("Preview", "Load a material to render a preview");
        }
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

    let mut preview_changed = false;
    for (i, param) in panel.shader_params.iter_mut().enumerate() {
        let param_label = format!("{}. {}", i + 1, param.name);

        match param.param_type {
            ShaderParamType::Float => {
                if let Some(val) = ui.slider_f32(&param_label, param.float_value, 0.0, 1.0) {
                    param.float_value = val;
                    preview_changed = true;
                    tracing::debug!(name = %param.name, value = val, "material param updated");
                }
            }
            ShaderParamType::Color => {
                if let Some(new_color) = ui.color_edit(&param_label, param.color_value) {
                    param.color_value = new_color;
                    preview_changed = true;
                    tracing::debug!(name = %param.name, color = ?new_color, "material color updated");
                }
            }
            ShaderParamType::Texture => {
                let current = param.texture_value.as_deref().unwrap_or("(none)");
                let new_val = ui.text_field(&param_label, current);
                if let Some(val) = new_val {
                    param.texture_value = Some(val);
                    preview_changed = true;
                    tracing::debug!(name = %param.name, "material texture updated");
                }
                let _ = ui.button("Pick…");
            }
        }
    }

    if preview_changed {
        panel.invalidate_preview();
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

    fn save_test_registry() -> AssetRegistry {
        use engine_renderer::Transparency;

        let mut registry = AssetRegistry::new();
        registry.insert_typed(
            AssetId::new("mat-project"),
            MaterialUpload {
                material_id: AssetId::new("mat-project"),
                base_color: [0.1, 0.2, 0.3, 1.0],
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
    fn material_edits_emit_versioned_preview_requests() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "preview-mat", &registry);

        let request = panel.take_preview_request().expect("preview request");
        assert_eq!(request.material_asset, "preview-mat");
        assert_eq!(request.preview_mesh, "sphere");
        assert!(!request.shader_params.is_empty());
        assert!(panel.take_preview_request().is_none());

        assert!(panel.complete_preview(request.revision, "editor/preview/1"));
        assert_eq!(panel.preview_texture.as_deref(), Some("editor/preview/1"));
    }

    #[test]
    fn stale_preview_results_are_rejected() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "preview-mat", &registry);
        let old = panel.take_preview_request().expect("first request");
        panel.invalidate_preview();

        assert!(!panel.complete_preview(old.revision, "editor/preview/stale"));
        assert!(panel.preview_texture.is_none());
    }

    #[test]
    fn failed_current_preview_is_requeued() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "preview-mat", &registry);
        let failed = panel.take_preview_request().expect("preview request");

        assert!(panel.fail_preview(failed.revision));
        assert_eq!(
            panel
                .take_preview_request()
                .expect("retried request")
                .revision,
            failed.revision
        );
    }

    #[test]
    fn software_preview_is_rgba_and_material_dependent() {
        let mut red = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut red, "preview-red", &registry);
        let red_request = red.take_preview_request().expect("red request");
        let red_pixels = render_material_preview_rgba8(&red_request, 32, 24);

        let mut blue_request = red_request.clone();
        blue_request
            .shader_params
            .iter_mut()
            .find(|param| param.name == "Albedo")
            .expect("albedo parameter")
            .color_value = [0.1, 0.2, 0.9, 1.0];
        let blue_pixels = render_material_preview_rgba8(&blue_request, 32, 24);

        assert_eq!(red_pixels.len(), 32 * 24 * 4);
        assert!(red_pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_ne!(red_pixels, blue_pixels);
    }

    #[test]
    fn reset_rejects_in_flight_preview_results() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "preview-mat", &registry);
        let old = panel.take_preview_request().expect("preview request");

        panel.reset();

        assert!(!panel.complete_preview(old.revision, "editor/preview/unloaded"));
        assert!(panel.preview_texture.is_none());
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

    #[test]
    fn save_material_emits_material_source_without_preview_texture() {
        let mut panel = MaterialEditorPanel::new();
        load_material(&mut panel, "mat-project", &save_test_registry());
        panel.set_save_access(MaterialSaveAccess::Writable);
        panel.preview_texture = Some("editor/material-preview".to_string());
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

        let mut ui = EditorUi::new();
        ui.inject_event(crate::editor_ui::UiEvent::ButtonClick(
            "Save Material".to_string(),
        ));
        draw_material_editor(&mut ui, &mut panel);
        let request = panel.take_save_request().unwrap().expect("save request");

        assert_eq!(request.material_asset, "mat-project");
        assert_eq!(request.source.schema, MATERIAL_SOURCE_SCHEMA);
        assert_eq!(request.source.roughness, 0.33);
        assert_eq!(request.source.base_color, [0.8, 0.6, 0.4, 1.0]);
        assert_eq!(
            request.source.base_color_texture.as_deref(),
            Some("texture-project")
        );
        assert!(!serde_json::to_string(&request.source)
            .unwrap()
            .contains("editor/material-preview"));
    }

    #[test]
    fn read_only_material_does_not_emit_save_request() {
        let mut panel = MaterialEditorPanel::new();
        load_material(&mut panel, "mat-default", &save_test_registry());
        panel.set_save_access(MaterialSaveAccess::ReadOnly(
            "Built-in material 'mat-default' is read-only.".to_string(),
        ));

        let mut ui = EditorUi::new();
        ui.inject_event(crate::editor_ui::UiEvent::ButtonClick(
            "Save Material".to_string(),
        ));
        draw_material_editor(&mut ui, &mut panel);

        assert!(panel.take_save_request().unwrap().is_none());
        assert!(matches!(
            panel.save_access(),
            MaterialSaveAccess::ReadOnly(reason) if reason.contains("Built-in")
        ));
    }

    #[test]
    fn draw_completed_preview_emits_image_texture() {
        let mut panel = MaterialEditorPanel::new();
        let registry = AssetRegistry::new();
        load_material(&mut panel, "test-mat", &registry);
        let revision = panel
            .take_preview_request()
            .expect("preview request")
            .revision;
        assert!(panel.complete_preview(revision, "editor/material-preview/test"));

        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.inject_event(crate::editor_ui::UiEvent::ButtonClick(
            "Preview Viewport".into(),
        ));
        draw_material_editor(&mut ui, &mut panel);
        let canvas = ui.end_frame();
        assert!(canvas.build_batches().iter().any(|batch| {
            batch
                .texture
                .as_ref()
                .is_some_and(|texture| texture.id == "editor/material-preview/test")
        }));
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
