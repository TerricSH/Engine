use engine_scene::Scene;
use engine_script::ScriptComponent;
use engine_serialize::{PersistentId, Value};

use crate::commands::{Command, SetComponentField};
use crate::editor_ui::EditorUi;

// -------------------------------------------------------------------
// InspectorPanel
// -------------------------------------------------------------------

/// Component inspector panel that shows the selected entity's
/// components and allows editing of their fields.
///
/// The [`ui`] method returns [`SetComponentField`] commands that the
/// caller should apply to the scene.
pub struct InspectorPanel {
    visible: bool,
    name: String,
}

impl InspectorPanel {
    /// Create a new inspector panel.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
        }
    }

    /// Panel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the panel is visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide the panel.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Render the inspector for the given selected entity.
    ///
    /// Returns a list of commands that the caller must apply to the
    /// scene via [`EditorScene::execute`].
    pub fn ui(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        selected: Option<&PersistentId>,
    ) -> Vec<Box<dyn Command>> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();

        ui.collapsing_header("Inspector", true);

        let entity = match selected.and_then(|id| scene.entities.iter().find(|e| e.persistent_id == *id)) {
            Some(e) => e,
            None => {
                // No entity selected – show placeholder.
                ui.text_field("Entity", "(none selected)");
                ui.separator();
                ui.collapsing_header("Components", true);
                ui.text_field("Hint", "Select an entity in the Hierarchy panel");
                return commands;
            }
        };

        // ── Entity header ────────────────────────────────────────
        ui.separator();
        ui.collapsing_header(&format!("Entity [{}]", entity.persistent_id), true);

        // Name
        let current_name = entity.name.clone().unwrap_or_default();
        if let Some(edited) = ui.text_field("Name", &current_name) {
            let new_name = if edited.is_empty() { None } else { Some(edited) };
            commands.push(Box::new(crate::commands::SetEntityName::new(
                entity.persistent_id.clone(),
                new_name,
            )));
        }

        // Enabled
        // (Read-only in the inspector for now; could be toggled later)

        ui.separator();

        // ── Components ────────────────────────────────────────────
        let expanded = ui.collapsing_header("Components", true);
        if expanded {
            for (comp_type, comp_record) in &entity.components {
                let comp_header = format!("{comp_type} [{}]", if comp_record.enabled { "x" } else { " " });
                let comp_open = ui.collapsing_header(&comp_header, false);
                if comp_open {
                    for (field_name, value) in &comp_record.fields {
                        let label = format!("{comp_type}/{field_name}");
                        if let Some(cmd) = edit_value(ui, &label, value, &entity.persistent_id, comp_type, field_name) {
                            commands.push(cmd);
                        }
                    }
                }
            }
        }

        commands
    }
    /// Render the inspector for scene components **and** script components.
    ///
    /// The scene-level components are shown first (same as [`ui`]), followed
    /// by a "Script Components" section that lists every attached
    /// [`ScriptComponent`] with editable field widgets.
    pub fn ui_with_script_data(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        selected: Option<&PersistentId>,
        script_components: &[ScriptComponent],
    ) -> Vec<Box<dyn Command>> {
        // Reuse the core inspector to show scene-level components.
        let mut commands = self.ui(ui, scene, selected);

        if script_components.is_empty() {
            return commands;
        }

        ui.separator();
        let script_open = ui.collapsing_header("Script Components", true);
        if !script_open {
            return commands;
        }

        for (idx, sc) in script_components.iter().enumerate() {
            let header = format!(
                "{} [{}] ({})",
                sc.class_name,
                if sc.enabled { "x" } else { " " },
                sc.assembly_id,
            );
            let comp_open = ui.collapsing_header(&header, false);
            if !comp_open {
                continue;
            }

            // Read-only metadata
            ui.text_field("Assembly", &sc.assembly_id);
            ui.text_field("Class", &sc.class_name);

            // Editable fields
            for (field_name, sv) in &sc.fields {
                let label = format!("script.{idx}.{field_name}");
                let comp_type = format!("__script_component__.{idx}");
                // Use the selected entity ID if available
                let entity_id = selected.unwrap_or(&sc.class_name);
                if let Some(cmd) = edit_script_value(
                    ui,
                    &label,
                    sv,
                    entity_id,
                    &comp_type,
                    field_name,
                ) {
                    commands.push(cmd);
                }
            }
        }

        commands
    }
}

// -------------------------------------------------------------------
// Value editing
// -------------------------------------------------------------------

/// Render an editable widget for a [`Value`] and return a
/// [`SetComponentField`] command if the user changed it.
fn edit_value(
    ui: &mut EditorUi,
    label: &str,
    value: &Value,
    entity_id: &PersistentId,
    comp_type: &str,
    field_name: &str,
) -> Option<Box<dyn Command>> {
    match value {
        Value::Bool(b) => {
            let new_val = ui.checkbox(label, *b);
            if new_val != *b {
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Bool(new_val),
                )));
            }
        }
        Value::Int(i) => {
            let current = i.to_string();
            if let Some(edited) = ui.text_field(label, &current) {
                if let Ok(parsed) = edited.parse::<i64>() {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Int(parsed),
                    )));
                }
            }
        }
        Value::UInt(u) => {
            let current = u.to_string();
            if let Some(edited) = ui.text_field(label, &current) {
                if let Ok(parsed) = edited.parse::<u64>() {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::UInt(parsed),
                    )));
                }
            }
        }
        Value::Float32(f) => {
            if let Some(new_f) = ui.slider_f32(label, *f, -10_000.0, 10_000.0) {
                if (new_f - *f).abs() > f32::EPSILON {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Float32(new_f),
                    )));
                }
            }
        }
        Value::Float64(f) => {
            let as_f32 = *f as f32;
            if let Some(new_f) = ui.slider_f32(label, as_f32, -10_000.0, 10_000.0) {
                if (new_f - as_f32).abs() > f32::EPSILON {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Float64(new_f as f64),
                    )));
                }
            }
        }
        Value::Str(s) => {
            if let Some(edited) = ui.text_field(label, s) {
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Str(edited),
                )));
            }
        }
        Value::Vec3(arr) => {
            // Show each component as a slider
            if let Some(new_x) = ui.slider_f32(&format!("{label}.x"), arr[0], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[0] = new_x;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
            if let Some(new_y) = ui.slider_f32(&format!("{label}.y"), arr[1], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[1] = new_y;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
            if let Some(new_z) = ui.slider_f32(&format!("{label}.z"), arr[2], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[2] = new_z;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
        }
        Value::Quat(arr) => {
            if let Some(new_x) = ui.slider_f32(&format!("{label}.x"), arr[0], -1.0, 1.0) {
                let mut new_arr = *arr;
                new_arr[0] = new_x;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Quat(new_arr),
                )));
            }
            if let Some(new_y) = ui.slider_f32(&format!("{label}.y"), arr[1], -1.0, 1.0) {
                let mut new_arr = *arr;
                new_arr[1] = new_y;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Quat(new_arr),
                )));
            }
            if let Some(new_z) = ui.slider_f32(&format!("{label}.z"), arr[2], -1.0, 1.0) {
                let mut new_arr = *arr;
                new_arr[2] = new_z;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Quat(new_arr),
                )));
            }
            if let Some(new_w) = ui.slider_f32(&format!("{label}.w"), arr[3], -1.0, 1.0) {
                let mut new_arr = *arr;
                new_arr[3] = new_w;
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Quat(new_arr),
                )));
            }
        }
        Value::Color(arr) => {
            if let Some(new_color) = ui.color_edit(label, *arr) {
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Color(new_color),
                )));
            }
        }
        Value::Asset(asset_id) => {
            let display = if let Some(ref path) = asset_id.logical_path {
                format!("{} ({})", asset_id.id, path)
            } else {
                asset_id.id.clone()
            };
            ui.text_field(label, &display);
        }
        Value::Entity(eid) => {
            ui.text_field(label, eid);
        }
        Value::Enum(s) => {
            ui.text_field(label, s);
        }
        Value::List(items) => {
            let open = ui.collapsing_header(label, false);
            if open {
                for (i, item) in items.iter().enumerate() {
                    let item_label = format!("{label}[{i}]");
                    let _ = edit_value(ui, &item_label, item, entity_id, comp_type, field_name);
                }
            }
        }
        Value::Map(map) => {
            let open = ui.collapsing_header(label, false);
            if open {
                for (key, val) in map {
                    let entry_label = format!("{label}.{key}");
                    let _ = edit_value(ui, &entry_label, val, entity_id, comp_type, field_name);
                }
            }
        }
    }

    None
}

// -------------------------------------------------------------------
// ScriptValue editing
// -------------------------------------------------------------------

/// Render an editable widget for a [`engine_script::ScriptValue`] and
/// return a [`SetComponentField`] command if the user changed it.
fn edit_script_value(
    ui: &mut EditorUi,
    label: &str,
    value: &engine_script::ScriptValue,
    entity_id: &PersistentId,
    comp_type: &str,
    field_name: &str,
) -> Option<Box<dyn Command>> {
    match value {
        engine_script::ScriptValue::Null => {
            ui.text_field(label, "null");
        }
        engine_script::ScriptValue::Bool(b) => {
            let new_val = ui.checkbox(label, *b);
            if new_val != *b {
                return Some(Box::new(SetComponentField::new(
                    entity_id.to_string(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Bool(new_val),
                )));
            }
        }
        engine_script::ScriptValue::Int(i) => {
            let current = i.to_string();
            if let Some(edited) = ui.text_field(label, &current) {
                if let Ok(parsed) = edited.parse::<i64>() {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Int(parsed),
                    )));
                }
            }
        }
        engine_script::ScriptValue::Float(f) => {
            let as_f32 = *f as f32;
            if let Some(new_f) = ui.slider_f32(label, as_f32, -10_000.0, 10_000.0) {
                if (new_f - as_f32).abs() > f32::EPSILON {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Float64(new_f as f64),
                    )));
                }
            }
        }
        engine_script::ScriptValue::String(s) => {
            if let Some(edited) = ui.text_field(label, s) {
                return Some(Box::new(SetComponentField::new(
                    entity_id.to_string(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Str(edited),
                )));
            }
        }
        engine_script::ScriptValue::Vec3(arr) => {
            if let Some(new_x) = ui.slider_f32(&format!("{label}.x"), arr[0], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[0] = new_x;
                return Some(Box::new(SetComponentField::new(
                    entity_id.to_string(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
            if let Some(new_y) = ui.slider_f32(&format!("{label}.y"), arr[1], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[1] = new_y;
                return Some(Box::new(SetComponentField::new(
                    entity_id.to_string(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
            if let Some(new_z) = ui.slider_f32(&format!("{label}.z"), arr[2], -10_000.0, 10_000.0) {
                let mut new_arr = *arr;
                new_arr[2] = new_z;
                return Some(Box::new(SetComponentField::new(
                    entity_id.to_string(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Vec3(new_arr),
                )));
            }
        }
        engine_script::ScriptValue::Vec4(arr) => {
            // Display as read-only text since Value has no Vec4 variant
            ui.text_field(label, &format!("[{}, {}, {}, {}]", arr[0], arr[1], arr[2], arr[3]));
        }
        engine_script::ScriptValue::EntityId(eid) => {
            ui.text_field(label, eid);
        }
        engine_script::ScriptValue::AssetIdWrapper(aid) => {
            ui.text_field(label, aid);
        }
        engine_script::ScriptValue::Array(items) => {
            let open = ui.collapsing_header(label, false);
            if open {
                for (i, item) in items.iter().enumerate() {
                    let item_label = format!("{label}[{i}]");
                    let _ = edit_script_value(ui, &item_label, item, entity_id, comp_type, field_name);
                }
            }
        }
        engine_script::ScriptValue::Map(map) => {
            let open = ui.collapsing_header(label, false);
            if open {
                for (key, val) in map {
                    let entry_label = format!("{label}.{key}");
                    let _ = edit_script_value(ui, &entry_label, val, entity_id, comp_type, field_name);
                }
            }
        }
    }

    None
}
