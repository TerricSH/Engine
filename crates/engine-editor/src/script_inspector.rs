//! Script component inspector — shows editable fields from [`ScriptComponent`]s.
//!
//! For each attached script the inspector displays the assembly identifier and
//! class name (read-only), followed by each serialised field with an
//! appropriate editor widget.  Field edits are returned as
//! [`SetComponentField`] commands so the caller can apply them to the scene.

use engine_script::ScriptComponent;
use engine_script::ScriptValue;
use engine_serialize::Value;

use crate::commands::{Command, SetComponentField};
use crate::editor_ui::EditorUi;

// ---------------------------------------------------------------------------
// ScriptInspector
// ---------------------------------------------------------------------------

/// Inspector panel fragment that renders editable fields for a list of
/// [`ScriptComponent`]s attached to a single entity.
///
/// Usage:
/// ```ignore
/// let mut inspector = ScriptInspector::new();
/// let cmds = inspector.ui(&mut ui, "entity-001", &scripts);
/// for cmd in cmds { scene.execute(cmd)?; }
/// ```
pub struct ScriptInspector;

impl ScriptInspector {
    /// Create a new script inspector.
    pub fn new() -> Self {
        Self
    }

    /// Render the script component editor for the given scripts.
    ///
    /// * `entity_id` — the persistent ID of the entity these scripts belong to
    ///   (used when constructing [`SetComponentField`] commands).
    /// * `scripts` — the list of script components to inspect.
    ///
    /// Returns a list of [`Command`]s that the caller should execute on the
    /// scene (typically via [`EditorScene::execute`]).
    pub fn ui(
        &mut self,
        ui: &mut EditorUi,
        entity_id: &str,
        scripts: &[ScriptComponent],
    ) -> Vec<Box<dyn Command>> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();

        ui.separator();
        let open = ui.collapsing_header("Script Components", !scripts.is_empty());

        if !open {
            return commands;
        }

        for script in scripts {
            self.render_script(ui, entity_id, script, &mut commands);
        }

        commands
    }

    // ── Internal helpers ───────────────────────────────────────────────

    /// Render a single [`ScriptComponent`] and collect any edit commands.
    fn render_script(
        &mut self,
        ui: &mut EditorUi,
        entity_id: &str,
        script: &ScriptComponent,
        commands: &mut Vec<Box<dyn Command>>,
    ) {
        let header = format!(
            "{} [{}]",
            script.class_name,
            if script.enabled { "x" } else { " " },
        );
        let open = ui.collapsing_header(&header, false);

        if !open {
            return;
        }

        // ── Read-only metadata ────────────────────────────────────
        ui.text_field("Assembly", &script.assembly_id);
        ui.text_field("Class", &script.class_name);
        ui.separator();

        // ── Editable fields ────────────────────────────────────────
        let fields_open = ui.collapsing_header("Fields", !script.fields.is_empty());
        if fields_open {
            for (field_name, sv) in &script.fields {
                let label = format!("{}/{}", script.class_name, field_name);
                Self::edit_script_value(
                    ui,
                    &label,
                    sv,
                    entity_id,
                    &script.assembly_id,
                    field_name,
                    commands,
                );
            }
        }
    }

    /// Render an editable widget for a [`ScriptValue`] and push a
    /// [`SetComponentField`] command on edit.
    ///
    /// The `comp_type` used for the command is
    /// `"engine.script::{assembly_id}"` so that edits can be round-tripped
    /// through the scene's component store.
    fn edit_script_value(
        ui: &mut EditorUi,
        label: &str,
        value: &ScriptValue,
        entity_id: &str,
        assembly_id: &str,
        field_name: &str,
        commands: &mut Vec<Box<dyn Command>>,
    ) {
        let comp_type = format!("engine.script::{assembly_id}");

        match value {
            ScriptValue::Null => {
                ui.text_field(label, "(null)");
            }
            ScriptValue::Bool(b) => {
                let new_val = ui.checkbox(label, *b);
                if new_val != *b {
                    commands.push(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.clone(),
                        field_name.to_string(),
                        Value::Bool(new_val),
                    )));
                }
            }
            ScriptValue::Int(i) => {
                let current = i.to_string();
                if let Some(edited) = ui.text_field(label, &current) {
                    if let Ok(parsed) = edited.parse::<i64>() {
                        commands.push(Box::new(SetComponentField::new(
                            entity_id.to_string(),
                            comp_type.clone(),
                            field_name.to_string(),
                            Value::Int(parsed),
                        )));
                    }
                }
            }
            ScriptValue::Float(f) => {
                let as_f32 = *f as f32;
                if let Some(new_f) = ui.slider_f32(label, as_f32, -10_000.0, 10_000.0) {
                    if (new_f - as_f32).abs() > f32::EPSILON {
                        commands.push(Box::new(SetComponentField::new(
                            entity_id.to_string(),
                            comp_type.clone(),
                            field_name.to_string(),
                            Value::Float64(new_f as f64),
                        )));
                    }
                }
            }
            ScriptValue::String(s) => {
                if let Some(edited) = ui.text_field(label, s) {
                    commands.push(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.clone(),
                        field_name.to_string(),
                        Value::Str(edited),
                    )));
                }
            }
            ScriptValue::Vec3(arr) => {
                if let Some(new_x) =
                    ui.slider_f32(&format!("{label}.x"), arr[0], -10_000.0, 10_000.0)
                {
                    let mut new_arr = *arr;
                    new_arr[0] = new_x;
                    commands.push(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.clone(),
                        field_name.to_string(),
                        Value::Vec3(new_arr),
                    )));
                }
                if let Some(new_y) =
                    ui.slider_f32(&format!("{label}.y"), arr[1], -10_000.0, 10_000.0)
                {
                    let mut new_arr = *arr;
                    new_arr[1] = new_y;
                    commands.push(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.clone(),
                        field_name.to_string(),
                        Value::Vec3(new_arr),
                    )));
                }
                if let Some(new_z) =
                    ui.slider_f32(&format!("{label}.z"), arr[2], -10_000.0, 10_000.0)
                {
                    let mut new_arr = *arr;
                    new_arr[2] = new_z;
                    commands.push(Box::new(SetComponentField::new(
                        entity_id.to_string(),
                        comp_type.clone(),
                        field_name.to_string(),
                        Value::Vec3(new_arr),
                    )));
                }
            }
            ScriptValue::Vec4(arr) => {
                // Render as read-only since Value has no Vec4 variant
                ui.text_field(
                    label,
                    &format!("[{}, {}, {}, {}]", arr[0], arr[1], arr[2], arr[3]),
                );
            }
            ScriptValue::EntityId(eid) => {
                ui.text_field(label, eid);
            }
            ScriptValue::AssetIdWrapper(s) => {
                let display = format!("[asset] {s}");
                ui.text_field(label, &display);
                // Asset assignment picker would go here in a full implementation
            }
            ScriptValue::Array(items) => {
                let open = ui.collapsing_header(label, false);
                if open {
                    for (i, item) in items.iter().enumerate() {
                        let item_label = format!("{label}[{i}]");
                        Self::edit_script_value(
                            ui,
                            &item_label,
                            item,
                            entity_id,
                            assembly_id,
                            field_name,
                            commands,
                        );
                    }
                }
            }
            ScriptValue::Map(map) => {
                let open = ui.collapsing_header(label, false);
                if open {
                    for (key, val) in map {
                        let entry_label = format!("{label}.{key}");
                        Self::edit_script_value(
                            ui,
                            &entry_label,
                            val,
                            entity_id,
                            assembly_id,
                            field_name,
                            commands,
                        );
                    }
                }
            }
        }
    }
}

impl Default for ScriptInspector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EditorUi;

    #[test]
    fn script_inspector_new() {
        let inspector = ScriptInspector::new();
        assert!(inspector.expanded_scripts.is_empty());
    }

    #[test]
    fn script_inspector_default() {
        let inspector = ScriptInspector::default();
        assert!(inspector.expanded_scripts.is_empty());
    }

    #[test]
    fn script_inspector_ui_empty_scripts() {
        let mut inspector = ScriptInspector::new();
        let mut ui = EditorUi::new();
        let cmds = inspector.ui(&mut ui, "entity-001", &[]);
        assert!(cmds.is_empty());
    }

    #[test]
    fn script_inspector_ui_with_scripts() {
        let mut inspector = ScriptInspector::new();
        let mut ui = EditorUi::new();

        let scripts = vec![ScriptComponent::new("asm-01", "MyScript")
            .with_field("speed", ScriptValue::Float(100.0))
            .with_field("enabled", ScriptValue::Bool(true))];

        let cmds = inspector.ui(&mut ui, "entity-001", &scripts);
        // Scaffolding editor returns no edits, so commands should be empty
        assert!(cmds.is_empty());
    }

    #[test]
    fn script_inspector_ui_multiple_scripts() {
        let mut inspector = ScriptInspector::new();
        let mut ui = EditorUi::new();

        let scripts = vec![
            ScriptComponent::new("asm-a", "PlayerController"),
            ScriptComponent::new("asm-a", "HealthComponent"),
        ];

        let cmds = inspector.ui(&mut ui, "entity-001", &scripts);
        assert!(cmds.is_empty());
    }
}
