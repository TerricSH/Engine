use std::collections::BTreeMap;

use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_script::ScriptComponent;
use engine_serialize::{PersistentId, SchemaVersion, Value};

use crate::commands::{
    AddComponent, Command, RemoveComponent, SequencedCommand, SetComponentEnabled,
    SetComponentField, SetEntityEnabled,
};
use crate::editor_ui::{EditorUi, UiInteractionPhase, UiInteractionStamp};
use crate::hierarchy::{
    renderable_component, transform_component, RENDERABLE_COMPONENT_TYPE, TRANSFORM_COMPONENT_TYPE,
};

const CAMERA_COMPONENT_TYPE: &str = "engine.camera";
const LIGHT_COMPONENT_TYPE: &str = "engine.light";
const RIGID_BODY_COMPONENT_TYPE: &str = "engine.physics.rigid_body";
const COLLIDER_COMPONENT_TYPE: &str = "engine.physics.collider";
const CHARACTER_CONTROLLER_COMPONENT_TYPE: &str = "engine.character_controller";
const SCRIPT_COMPONENT_TYPE: &str = "engine.script";

fn sequenced_command(ui: &mut EditorUi, command: Box<dyn Command>) -> SequencedCommand {
    let stamp = ui
        .take_last_interaction_stamp()
        .unwrap_or(UiInteractionStamp {
            sequence: u64::MAX,
            phase: UiInteractionPhase::AfterRawPointer,
        });
    SequencedCommand::new(stamp, command)
}

/// Project-specific information needed for authoring components whose default
/// record cannot be inferred from the scene alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InspectorContext {
    /// Assembly identifier accepted by the project script loader, normally the
    /// configured DLL's file stem (for example `GameScripts`).
    pub script_assembly_id: Option<String>,
    /// Suggested fully qualified class for a newly attached script.
    pub default_script_class: Option<String>,
}

impl InspectorContext {
    pub fn with_script_assembly(assembly_id: impl Into<String>) -> Self {
        let assembly_id = assembly_id.into();
        Self {
            default_script_class: Some(format!("{assembly_id}.Main")),
            script_assembly_id: Some(assembly_id),
        }
    }
}

#[derive(Clone, Copy)]
struct ComponentDescriptor {
    type_id: &'static str,
    display_name: &'static str,
    make_default: fn(&EntityRecord) -> ComponentRecord,
}

fn component_record(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn default_transform(_: &EntityRecord) -> ComponentRecord {
    transform_component()
}

fn default_renderable(_: &EntityRecord) -> ComponentRecord {
    renderable_component()
}

fn default_camera(_: &EntityRecord) -> ComponentRecord {
    component_record(BTreeMap::from([
        ("projection".into(), Value::Enum("Perspective".into())),
        ("near".into(), Value::Float32(0.1)),
        ("far".into(), Value::Float32(1000.0)),
        ("fov_y".into(), Value::Float32(std::f32::consts::FRAC_PI_4)),
        ("ortho_half_height".into(), Value::Float32(5.0)),
        ("render_layer_mask".into(), Value::UInt(u32::MAX as u64)),
        ("clear_flags".into(), Value::UInt(3)),
        ("clear_color".into(), Value::Color([0.02, 0.02, 0.06, 1.0])),
        ("priority".into(), Value::Int(0)),
        ("msaa_samples".into(), Value::UInt(1)),
        ("hdr_output".into(), Value::Bool(false)),
        ("aperture".into(), Value::Float32(16.0)),
        ("shutter_speed".into(), Value::Float32(1.0 / 60.0)),
        ("iso".into(), Value::Float32(100.0)),
        ("ev_compensation".into(), Value::Float32(0.0)),
    ]))
}

fn default_light(_: &EntityRecord) -> ComponentRecord {
    component_record(BTreeMap::from([
        ("kind".into(), Value::Enum("Directional".into())),
        ("color".into(), Value::Vec3([1.0, 1.0, 1.0])),
        ("intensity".into(), Value::Float32(1.0)),
        ("range".into(), Value::Float32(10.0)),
        ("shadow_mode".into(), Value::UInt(0)),
        ("direction".into(), Value::Vec3([0.0, -1.0, 0.0])),
    ]))
}

fn default_rigid_body(_: &EntityRecord) -> ComponentRecord {
    component_record(BTreeMap::from([
        ("body_type".into(), Value::Enum("Dynamic".into())),
        ("mass".into(), Value::Float32(1.0)),
        ("linear_damping".into(), Value::Float32(0.0)),
        ("angular_damping".into(), Value::Float32(0.0)),
        ("enabled".into(), Value::Bool(true)),
        ("gravity_scale".into(), Value::Float32(1.0)),
        ("can_sleep".into(), Value::Bool(true)),
        ("ccd_enabled".into(), Value::Bool(false)),
    ]))
}

fn default_collider(_: &EntityRecord) -> ComponentRecord {
    let shape = Value::Map(BTreeMap::from([
        ("kind".into(), Value::Enum("Cuboid".into())),
        (
            "params".into(),
            Value::Map(BTreeMap::from([
                ("hx".into(), Value::Float32(0.5)),
                ("hy".into(), Value::Float32(0.5)),
                ("hz".into(), Value::Float32(0.5)),
            ])),
        ),
    ]));
    component_record(BTreeMap::from([
        ("shape".into(), shape),
        ("density".into(), Value::Float32(1.0)),
        ("friction".into(), Value::Float32(0.5)),
        ("restitution".into(), Value::Float32(0.0)),
        ("is_trigger".into(), Value::Bool(false)),
        ("collision_group".into(), Value::UInt(u32::MAX as u64)),
        ("collision_mask".into(), Value::UInt(u32::MAX as u64)),
    ]))
}

fn default_character_controller(entity: &EntityRecord) -> ComponentRecord {
    let position = entity
        .components
        .get(TRANSFORM_COMPONENT_TYPE)
        .and_then(|transform| transform.fields.get("translation"))
        .and_then(|value| match value {
            Value::Vec3(position) => Some(*position),
            _ => None,
        })
        .unwrap_or([0.0; 3]);
    component_record(BTreeMap::from([
        ("height".into(), Value::Float32(1.8)),
        ("radius".into(), Value::Float32(0.3)),
        ("move_speed".into(), Value::Float32(5.0)),
        ("acceleration".into(), Value::Float32(20.0)),
        ("deceleration".into(), Value::Float32(15.0)),
        ("air_acceleration".into(), Value::Float32(5.0)),
        ("air_deceleration".into(), Value::Float32(2.0)),
        ("jump_velocity".into(), Value::Float32(5.0)),
        ("gravity_scale".into(), Value::Float32(1.0)),
        ("max_fall_speed".into(), Value::Float32(20.0)),
        ("step_height".into(), Value::Float32(0.3)),
        ("slope_limit".into(), Value::Float32(45.0)),
        ("skin_offset".into(), Value::Float32(0.01)),
        ("state".into(), Value::Enum("Falling".into())),
        ("position".into(), Value::Vec3(position)),
        ("velocity".into(), Value::Vec3([0.0; 3])),
        ("foot_ik_enabled".into(), Value::Bool(true)),
    ]))
}

const COMPONENT_DESCRIPTORS: [ComponentDescriptor; 7] = [
    ComponentDescriptor {
        type_id: TRANSFORM_COMPONENT_TYPE,
        display_name: "Transform",
        make_default: default_transform,
    },
    ComponentDescriptor {
        type_id: RENDERABLE_COMPONENT_TYPE,
        display_name: "Renderable",
        make_default: default_renderable,
    },
    ComponentDescriptor {
        type_id: CAMERA_COMPONENT_TYPE,
        display_name: "Camera",
        make_default: default_camera,
    },
    ComponentDescriptor {
        type_id: LIGHT_COMPONENT_TYPE,
        display_name: "Light",
        make_default: default_light,
    },
    ComponentDescriptor {
        type_id: RIGID_BODY_COMPONENT_TYPE,
        display_name: "Rigid Body",
        make_default: default_rigid_body,
    },
    ComponentDescriptor {
        type_id: COLLIDER_COMPONENT_TYPE,
        display_name: "Collider",
        make_default: default_collider,
    },
    ComponentDescriptor {
        type_id: CHARACTER_CONTROLLER_COMPONENT_TYPE,
        display_name: "Character Controller",
        make_default: default_character_controller,
    },
];

fn component_display_name(type_id: &str) -> &str {
    COMPONENT_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.type_id == type_id)
        .map_or_else(
            || {
                if type_id == SCRIPT_COMPONENT_TYPE {
                    "Script"
                } else {
                    type_id
                }
            },
            |descriptor| descriptor.display_name,
        )
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::UInt(value) => value.to_string(),
        Value::Float32(value) => format!("{value:.3}"),
        Value::Float64(value) => format!("{value:.3}"),
        Value::Str(value) | Value::Enum(value) | Value::Entity(value) => value.clone(),
        Value::Vec3(value) => format!("[{:.3}, {:.3}, {:.3}]", value[0], value[1], value[2]),
        Value::Quat(value) | Value::Color(value) => format!(
            "[{:.3}, {:.3}, {:.3}, {:.3}]",
            value[0], value[1], value[2], value[3]
        ),
        Value::Asset(value) => value.id.clone(),
        Value::List(value) => format!("{} items", value.len()),
        Value::Map(value) => format!("{} fields", value.len()),
    }
}

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
    script_class_name: String,
    script_class_assembly: Option<String>,
}

impl InspectorPanel {
    /// Create a new inspector panel.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            script_class_name: String::new(),
            script_class_assembly: None,
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
        self.ui_with_context(ui, scene, selected, &InspectorContext::default())
    }

    /// Render the selected entity with project-aware component authoring.
    pub fn ui_with_context(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        selected: Option<&PersistentId>,
        context: &InspectorContext,
    ) -> Vec<Box<dyn Command>> {
        self.ui_with_context_ordered(ui, scene, selected, context)
            .into_iter()
            .map(SequencedCommand::into_command)
            .collect()
    }

    /// Render the inspector and retain the exact platform order for every
    /// command it emits.
    pub fn ui_with_context_ordered(
        &mut self,
        ui: &mut EditorUi,
        scene: &Scene,
        selected: Option<&PersistentId>,
        context: &InspectorContext,
    ) -> Vec<SequencedCommand> {
        let mut commands = Vec::new();

        ui.collapsing_header("Inspector", true);

        let entity =
            match selected.and_then(|id| scene.entities.iter().find(|e| e.persistent_id == *id)) {
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
            let new_name = if edited.is_empty() {
                None
            } else {
                Some(edited)
            };
            commands.push(sequenced_command(
                ui,
                Box::new(crate::commands::SetEntityName::new(
                    entity.persistent_id.clone(),
                    new_name,
                )),
            ));
        }

        let entity_enabled = ui.checkbox("Entity Enabled", entity.enabled);
        if entity_enabled != entity.enabled {
            commands.push(sequenced_command(
                ui,
                Box::new(SetEntityEnabled::new(
                    entity.persistent_id.clone(),
                    entity_enabled,
                )),
            ));
        }

        ui.separator();

        // ── Components ────────────────────────────────────────────
        let expanded = ui.collapsing_header("Components", true);
        if expanded {
            for descriptor in COMPONENT_DESCRIPTORS {
                if entity.components.contains_key(descriptor.type_id) {
                    continue;
                }
                if ui.button(&format!("Add {}", descriptor.display_name)) {
                    commands.push(sequenced_command(
                        ui,
                        Box::new(AddComponent::new(
                            entity.persistent_id.clone(),
                            descriptor.type_id.to_string(),
                            (descriptor.make_default)(entity),
                        )),
                    ));
                }
            }

            if self.script_class_assembly != context.script_assembly_id {
                self.script_class_assembly = context.script_assembly_id.clone();
                self.script_class_name =
                    context.default_script_class.clone().unwrap_or_else(|| {
                        context
                            .script_assembly_id
                            .as_ref()
                            .map_or_else(String::new, |assembly| format!("{assembly}.Main"))
                    });
            }
            if !entity.components.contains_key(SCRIPT_COMPONENT_TYPE) {
                if let Some(assembly_id) = context.script_assembly_id.as_deref() {
                    if let Some(edited) = ui.text_field("New Script Class", &self.script_class_name)
                    {
                        self.script_class_name = edited;
                    }
                    if self.script_class_name.trim().is_empty() {
                        ui.label_value("Add Script", "Enter a non-empty class name.");
                    } else if ui.button("Add Script") {
                        commands.push(sequenced_command(
                            ui,
                            Box::new(AddComponent::new(
                                entity.persistent_id.clone(),
                                SCRIPT_COMPONENT_TYPE.to_string(),
                                component_record(BTreeMap::from([
                                    ("assembly_id".into(), Value::Str(assembly_id.to_string())),
                                    (
                                        "class_name".into(),
                                        Value::Str(self.script_class_name.trim().to_string()),
                                    ),
                                ])),
                            )),
                        ));
                    }
                } else {
                    ui.label_value("Add Script", "Project has no script assembly.");
                }
            }

            for (comp_type, comp_record) in &entity.components {
                let comp_header = format!(
                    "{comp_type} [{}]",
                    if comp_record.enabled { "x" } else { " " }
                );
                let comp_open = ui.collapsing_header(&comp_header, false);
                if comp_open {
                    let component_enabled =
                        ui.checkbox(&format!("{comp_type}/Enabled"), comp_record.enabled);
                    if component_enabled != comp_record.enabled {
                        commands.push(sequenced_command(
                            ui,
                            Box::new(SetComponentEnabled::new(
                                entity.persistent_id.clone(),
                                comp_type.clone(),
                                component_enabled,
                            )),
                        ));
                    }

                    let display_name = component_display_name(comp_type);
                    if ui.button(&format!("Remove {display_name}")) {
                        commands.push(sequenced_command(
                            ui,
                            Box::new(RemoveComponent::new(
                                entity.persistent_id.clone(),
                                comp_type.clone(),
                            )),
                        ));
                        continue;
                    }

                    if comp_type == COLLIDER_COMPONENT_TYPE
                        && !entity.components.contains_key(RIGID_BODY_COMPONENT_TYPE)
                    {
                        ui.label_value("Collider", "Requires a Rigid Body on this entity.");
                    }

                    for (field_name, value) in &comp_record.fields {
                        if comp_type == CHARACTER_CONTROLLER_COMPONENT_TYPE
                            && matches!(field_name.as_str(), "state" | "position" | "velocity")
                        {
                            ui.label_value(
                                &format!("{comp_type}/{field_name}"),
                                &format_value(value),
                            );
                            continue;
                        }
                        let label = format!("{comp_type}/{field_name}");
                        if let Some(cmd) = edit_value(
                            ui,
                            &label,
                            value,
                            &entity.persistent_id,
                            comp_type,
                            field_name,
                        ) {
                            commands.push(sequenced_command(ui, cmd));
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
                // Use the selected entity ID if available
                let entity_id = selected.unwrap_or(&sc.class_name);
                if let Some(cmd) =
                    edit_script_value(ui, &label, sv, entity_id, SCRIPT_COMPONENT_TYPE, field_name)
                {
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
                    if valid_uint_field(comp_type, field_name, parsed) {
                        return Some(Box::new(SetComponentField::new(
                            entity_id.clone(),
                            comp_type.to_string(),
                            field_name.to_string(),
                            Value::UInt(parsed),
                        )));
                    }
                }
            }
        }
        Value::Float32(f) => {
            if let Some(new_f) = ui.slider_f32(label, *f, -10_000.0, 10_000.0) {
                if (new_f - *f).abs() > f32::EPSILON
                    && valid_float_field(comp_type, field_name, new_f as f64)
                {
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
                if (new_f - as_f32).abs() > f32::EPSILON
                    && valid_float_field(comp_type, field_name, new_f as f64)
                {
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
                if valid_string_field(comp_type, field_name, &edited) {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Str(edited),
                    )));
                }
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
            if let Some(edited) = ui.text_field(label, &asset_id.id) {
                if !edited.trim().is_empty() {
                    let mut new_asset = asset_id.clone();
                    new_asset.id = edited.trim().to_string();
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Asset(new_asset),
                    )));
                }
            }
        }
        Value::Entity(eid) => {
            if let Some(edited) = ui.text_field(label, eid) {
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Entity(edited),
                )));
            }
        }
        Value::Enum(s) => {
            let edited = if let Some(options) = enum_options(comp_type, field_name) {
                if ui.button(&format!("{label}: {s}")) {
                    let current = options.iter().position(|option| *option == s).unwrap_or(0);
                    Some(options[(current + 1) % options.len()].to_string())
                } else {
                    None
                }
            } else {
                ui.text_field(label, s)
            };
            if let Some(edited) = edited {
                return Some(Box::new(SetComponentField::new(
                    entity_id.clone(),
                    comp_type.to_string(),
                    field_name.to_string(),
                    Value::Enum(edited),
                )));
            }
        }
        Value::List(items) => {
            let open = ui.collapsing_header(label, false);
            if open {
                let mut edited_items = items.clone();
                let mut changed = false;
                for (i, item) in items.iter().enumerate() {
                    let item_label = format!("{label}[{i}]");
                    if let Some(edited) = edit_nested_value(ui, &item_label, item) {
                        edited_items[i] = edited;
                        changed = true;
                    }
                }
                if changed {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::List(edited_items),
                    )));
                }
            }
        }
        Value::Map(map) => {
            if comp_type == COLLIDER_COMPONENT_TYPE && field_name == "shape" {
                if let Some(edited) = edit_collider_shape(ui, label, map) {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        edited,
                    )));
                }
                return None;
            }
            let open = ui.collapsing_header(label, false);
            if open {
                let mut edited_map = map.clone();
                let mut changed = false;
                for (key, val) in map {
                    let entry_label = format!("{label}.{key}");
                    if let Some(edited) = edit_nested_value(ui, &entry_label, val) {
                        edited_map.insert(key.clone(), edited);
                        changed = true;
                    }
                }
                if changed {
                    return Some(Box::new(SetComponentField::new(
                        entity_id.clone(),
                        comp_type.to_string(),
                        field_name.to_string(),
                        Value::Map(edited_map),
                    )));
                }
            }
        }
    }

    None
}

fn enum_options(comp_type: &str, field_name: &str) -> Option<&'static [&'static str]> {
    match (comp_type, field_name) {
        (CAMERA_COMPONENT_TYPE, "projection") => Some(&["Perspective", "Orthographic"]),
        (LIGHT_COMPONENT_TYPE, "kind") => Some(&["Directional", "Point", "Spot"]),
        (RIGID_BODY_COMPONENT_TYPE, "body_type") => Some(&["Dynamic", "Static", "Kinematic"]),
        (CHARACTER_CONTROLLER_COMPONENT_TYPE, "state") => {
            Some(&["Grounded", "Jumping", "Falling", "Landing", "Free"])
        }
        _ => None,
    }
}

fn valid_uint_field(comp_type: &str, field_name: &str, value: u64) -> bool {
    match (comp_type, field_name) {
        (CAMERA_COMPONENT_TYPE, "msaa_samples") => matches!(value, 1 | 2 | 4 | 8),
        (LIGHT_COMPONENT_TYPE, "shadow_mode") => value <= 2,
        _ => true,
    }
}

fn valid_float_field(comp_type: &str, field_name: &str, value: f64) -> bool {
    if !value.is_finite() {
        return false;
    }
    match (comp_type, field_name) {
        (
            CAMERA_COMPONENT_TYPE,
            "near" | "far" | "fov_y" | "ortho_half_height" | "aperture" | "shutter_speed" | "iso",
        ) => value > 0.0,
        (LIGHT_COMPONENT_TYPE, "intensity" | "range") => value >= 0.0,
        (RIGID_BODY_COMPONENT_TYPE, "mass") => value > 0.0,
        (RIGID_BODY_COMPONENT_TYPE, "linear_damping" | "angular_damping") => value >= 0.0,
        (COLLIDER_COMPONENT_TYPE, "density" | "friction") => value >= 0.0,
        (COLLIDER_COMPONENT_TYPE, "restitution") => (0.0..=1.0).contains(&value),
        (CHARACTER_CONTROLLER_COMPONENT_TYPE, "slope_limit") => (0.0..=90.0).contains(&value),
        (
            CHARACTER_CONTROLLER_COMPONENT_TYPE,
            "height" | "radius" | "move_speed" | "acceleration" | "deceleration"
            | "air_acceleration" | "air_deceleration" | "max_fall_speed" | "step_height"
            | "skin_offset",
        ) => value >= 0.0,
        _ => true,
    }
}

fn valid_string_field(comp_type: &str, field_name: &str, value: &str) -> bool {
    if comp_type != SCRIPT_COMPONENT_TYPE {
        return true;
    }
    match field_name {
        "assembly_id" => !value.is_empty() && !value.chars().any(char::is_whitespace),
        "class_name" => !value.trim().is_empty(),
        _ => true,
    }
}

fn edit_nested_value(ui: &mut EditorUi, label: &str, value: &Value) -> Option<Value> {
    match value {
        Value::Bool(value) => {
            let edited = ui.checkbox(label, *value);
            (edited != *value).then_some(Value::Bool(edited))
        }
        Value::Int(value) => ui
            .text_field(label, &value.to_string())
            .and_then(|edited| edited.parse::<i64>().ok())
            .map(Value::Int),
        Value::UInt(value) => ui
            .text_field(label, &value.to_string())
            .and_then(|edited| edited.parse::<u64>().ok())
            .map(Value::UInt),
        Value::Float32(value) => ui
            .slider_f32(label, *value, -10_000.0, 10_000.0)
            .filter(|edited| edited.is_finite() && (*edited - *value).abs() > f32::EPSILON)
            .map(Value::Float32),
        Value::Float64(value) => ui
            .slider_f32(label, *value as f32, -10_000.0, 10_000.0)
            .filter(|edited| edited.is_finite() && (*edited as f64 - *value).abs() > f64::EPSILON)
            .map(|edited| Value::Float64(edited as f64)),
        Value::Str(value) => ui.text_field(label, value).map(Value::Str),
        Value::Vec3(value) => {
            let mut edited = *value;
            for (index, suffix) in ["x", "y", "z"].into_iter().enumerate() {
                if let Some(component) = ui.slider_f32(
                    &format!("{label}.{suffix}"),
                    value[index],
                    -10_000.0,
                    10_000.0,
                ) {
                    edited[index] = component;
                }
            }
            (edited != *value).then_some(Value::Vec3(edited))
        }
        Value::Quat(value) => {
            let mut edited = *value;
            for (index, suffix) in ["x", "y", "z", "w"].into_iter().enumerate() {
                if let Some(component) =
                    ui.slider_f32(&format!("{label}.{suffix}"), value[index], -1.0, 1.0)
                {
                    edited[index] = component;
                }
            }
            (edited != *value).then_some(Value::Quat(edited))
        }
        Value::Color(value) => ui.color_edit(label, *value).map(Value::Color),
        Value::Asset(value) => ui.text_field(label, &value.id).and_then(|edited| {
            (!edited.trim().is_empty()).then(|| {
                let mut asset = value.clone();
                asset.id = edited.trim().to_string();
                Value::Asset(asset)
            })
        }),
        Value::Entity(value) => ui.text_field(label, value).map(Value::Entity),
        Value::Enum(value) => ui.text_field(label, value).map(Value::Enum),
        Value::List(values) => {
            if !ui.collapsing_header(label, false) {
                return None;
            }
            let mut edited = values.clone();
            let mut changed = false;
            for (index, value) in values.iter().enumerate() {
                if let Some(new_value) = edit_nested_value(ui, &format!("{label}[{index}]"), value)
                {
                    edited[index] = new_value;
                    changed = true;
                }
            }
            changed.then_some(Value::List(edited))
        }
        Value::Map(values) => {
            if !ui.collapsing_header(label, false) {
                return None;
            }
            let mut edited = values.clone();
            let mut changed = false;
            for (key, value) in values {
                if let Some(new_value) = edit_nested_value(ui, &format!("{label}.{key}"), value) {
                    edited.insert(key.clone(), new_value);
                    changed = true;
                }
            }
            changed.then_some(Value::Map(edited))
        }
    }
}

fn collider_shape_value(kind: &str) -> Value {
    let params = match kind {
        "Ball" => BTreeMap::from([("radius".into(), Value::Float32(0.5))]),
        "Capsule" => BTreeMap::from([
            ("half_height".into(), Value::Float32(0.5)),
            ("radius".into(), Value::Float32(0.5)),
        ]),
        _ => BTreeMap::from([
            ("hx".into(), Value::Float32(0.5)),
            ("hy".into(), Value::Float32(0.5)),
            ("hz".into(), Value::Float32(0.5)),
        ]),
    };
    Value::Map(BTreeMap::from([
        ("kind".into(), Value::Enum(kind.to_string())),
        ("params".into(), Value::Map(params)),
    ]))
}

fn edit_collider_shape(
    ui: &mut EditorUi,
    label: &str,
    shape: &BTreeMap<String, Value>,
) -> Option<Value> {
    if !ui.collapsing_header(label, false) {
        return None;
    }
    let current_kind = shape
        .get("kind")
        .and_then(|value| match value {
            Value::Enum(kind) => Some(kind.as_str()),
            _ => None,
        })
        .unwrap_or("Cuboid");
    let kinds = ["Cuboid", "Ball", "Capsule"];
    if ui.button(&format!("{label}/Kind: {current_kind}")) {
        let current = kinds
            .iter()
            .position(|kind| *kind == current_kind)
            .unwrap_or(0);
        return Some(collider_shape_value(kinds[(current + 1) % kinds.len()]));
    }

    let params = shape.get("params")?;
    let Value::Map(params) = params else {
        return None;
    };
    let mut edited_params = params.clone();
    let mut changed = false;
    for (name, value) in params {
        if let Some(edited) = edit_nested_value(ui, &format!("{label}/{name}"), value) {
            let valid = match &edited {
                Value::Float32(value) => value.is_finite() && *value > 0.0,
                Value::Float64(value) => value.is_finite() && *value > 0.0,
                _ => false,
            };
            if valid {
                edited_params.insert(name.clone(), edited);
                changed = true;
            }
        }
    }
    changed.then(|| {
        Value::Map(BTreeMap::from([
            ("kind".into(), Value::Enum(current_kind.to_string())),
            ("params".into(), Value::Map(edited_params)),
        ]))
    })
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
            ui.text_field(
                label,
                &format!("[{}, {}, {}, {}]", arr[0], arr[1], arr[2], arr[3]),
            );
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
                    let _ =
                        edit_script_value(ui, &item_label, item, entity_id, comp_type, field_name);
                }
            }
        }
        engine_script::ScriptValue::Map(map) => {
            let open = ui.collapsing_header(label, false);
            if open {
                for (key, val) in map {
                    let entry_label = format!("{label}.{key}");
                    let _ =
                        edit_script_value(ui, &entry_label, val, entity_id, comp_type, field_name);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use engine_scene::{ComponentRecord, EntityRecord};

    use super::*;
    use crate::editor_ui::UiEvent;
    use crate::{EditorScene, EditorUi};

    fn component<'a>(
        editor_scene: &'a EditorScene,
        entity_id: &str,
        component_type: &str,
    ) -> Option<&'a ComponentRecord> {
        editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == entity_id)
            .and_then(|entity| entity.components.get(component_type))
    }

    #[test]
    fn add_component_buttons_execute_undo_and_redo() {
        for descriptor in COMPONENT_DESCRIPTORS {
            let selected = "inspector-target".to_string();
            let target = EntityRecord {
                persistent_id: selected.clone(),
                parent: None,
                name: Some("Inspector Target".to_string()),
                enabled: true,
                components: BTreeMap::new(),
            };
            let expected = (descriptor.make_default)(&target);
            let mut scene = engine_scene::sample_scene();
            scene.entities.push(target);
            let mut editor_scene = EditorScene::new(scene);
            let mut panel = InspectorPanel::new("Inspector");
            let mut ui = EditorUi::new();
            let button_label = format!("Add {}", descriptor.display_name);
            ui.inject_event(UiEvent::ButtonClick(button_label.clone()));

            let commands = panel.ui(&mut ui, &editor_scene.scene, Some(&selected));
            assert_eq!(commands.len(), 1, "{button_label} must emit one command");
            editor_scene
                .execute(commands.into_iter().next().unwrap())
                .unwrap();
            assert_eq!(
                component(&editor_scene, &selected, descriptor.type_id),
                Some(&expected)
            );

            editor_scene.undo().unwrap();
            assert!(component(&editor_scene, &selected, descriptor.type_id).is_none());
            editor_scene.redo().unwrap();
            assert_eq!(
                component(&editor_scene, &selected, descriptor.type_id),
                Some(&expected)
            );
        }
    }

    #[test]
    fn remove_component_buttons_execute_undo_and_redo() {
        for descriptor in COMPONENT_DESCRIPTORS {
            let selected = "inspector-target".to_string();
            let mut scene = engine_scene::sample_scene();
            let mut target = EntityRecord {
                persistent_id: selected.clone(),
                parent: None,
                name: Some("Inspector Target".to_string()),
                enabled: true,
                components: BTreeMap::new(),
            };
            let expected = (descriptor.make_default)(&target);
            let mut components = BTreeMap::new();
            components.insert(descriptor.type_id.to_string(), expected.clone());
            target.components = components;
            scene.entities.push(target);

            let mut editor_scene = EditorScene::new(scene);
            let mut panel = InspectorPanel::new("Inspector");
            let mut ui = EditorUi::new();
            let button_label = format!("Remove {}", descriptor.display_name);
            ui.inject_event(UiEvent::ButtonClick(format!("{} [x]", descriptor.type_id)));
            ui.inject_event(UiEvent::ButtonClick(button_label.clone()));

            let commands = panel.ui(&mut ui, &editor_scene.scene, Some(&selected));
            assert_eq!(commands.len(), 1, "{button_label} must emit one command");
            editor_scene
                .execute(commands.into_iter().next().unwrap())
                .unwrap();
            assert!(component(&editor_scene, &selected, descriptor.type_id).is_none());

            editor_scene.undo().unwrap();
            assert_eq!(
                component(&editor_scene, &selected, descriptor.type_id),
                Some(&expected)
            );
            editor_scene.redo().unwrap();
            assert!(component(&editor_scene, &selected, descriptor.type_id).is_none());
        }
    }

    #[test]
    fn script_attachment_uses_the_canonical_component_type_and_project_assembly() {
        let selected = "script-target".to_string();
        let mut scene = engine_scene::sample_scene();
        scene.entities.push(EntityRecord {
            persistent_id: selected.clone(),
            parent: None,
            name: Some("Script Target".into()),
            enabled: true,
            components: BTreeMap::new(),
        });
        let mut editor_scene = EditorScene::new(scene);
        let mut panel = InspectorPanel::new("Inspector");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Add Script".into()));

        let commands = panel.ui_with_context(
            &mut ui,
            &editor_scene.scene,
            Some(&selected),
            &InspectorContext::with_script_assembly("GameScripts"),
        );
        assert_eq!(commands.len(), 1);
        editor_scene
            .execute(commands.into_iter().next().unwrap())
            .unwrap();

        let script = component(&editor_scene, &selected, SCRIPT_COMPONENT_TYPE).unwrap();
        assert_eq!(
            script.fields.get("assembly_id"),
            Some(&Value::Str("GameScripts".into()))
        );
        assert_eq!(
            script.fields.get("class_name"),
            Some(&Value::Str("GameScripts.Main".into()))
        );
        assert!(editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == selected)
            .unwrap()
            .components
            .keys()
            .all(|type_id| !type_id.starts_with("engine.script::")));

        editor_scene.undo().unwrap();
        assert!(component(&editor_scene, &selected, SCRIPT_COMPONENT_TYPE).is_none());
        editor_scene.redo().unwrap();
        assert!(component(&editor_scene, &selected, SCRIPT_COMPONENT_TYPE).is_some());
    }

    #[test]
    fn entity_and_component_enabled_state_are_undoable() {
        let selected = "enabled-target".to_string();
        let mut scene = engine_scene::sample_scene();
        scene.entities.push(EntityRecord {
            persistent_id: selected.clone(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::from([(
                RENDERABLE_COMPONENT_TYPE.to_string(),
                renderable_component(),
            )]),
        });
        let mut editor_scene = EditorScene::new(scene);
        let mut panel = InspectorPanel::new("Inspector");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::CheckboxToggle("Entity Enabled".into(), false));
        ui.inject_event(UiEvent::ButtonClick(format!(
            "{RENDERABLE_COMPONENT_TYPE} [x]"
        )));
        ui.inject_event(UiEvent::CheckboxToggle(
            format!("{RENDERABLE_COMPONENT_TYPE}/Enabled"),
            false,
        ));

        let commands = panel.ui(&mut ui, &editor_scene.scene, Some(&selected));
        assert_eq!(commands.len(), 2);
        for command in commands {
            editor_scene.execute(command).unwrap();
        }
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == selected)
            .unwrap();
        assert!(!entity.enabled);
        assert!(!entity.components[RENDERABLE_COMPONENT_TYPE].enabled);

        editor_scene.undo().unwrap();
        editor_scene.undo().unwrap();
        let entity = editor_scene
            .scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == selected)
            .unwrap();
        assert!(entity.enabled);
        assert!(entity.components[RENDERABLE_COMPONENT_TYPE].enabled);
    }

    #[test]
    fn collider_nested_shape_edits_replace_the_complete_top_level_value() {
        let selected = "collider-target".to_string();
        let target = EntityRecord {
            persistent_id: selected.clone(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::new(),
        };
        let collider = default_collider(&target);
        let mut scene = engine_scene::sample_scene();
        let mut target = target;
        target
            .components
            .insert(COLLIDER_COMPONENT_TYPE.into(), collider);
        scene.entities.push(target);
        let mut editor_scene = EditorScene::new(scene);
        let mut panel = InspectorPanel::new("Inspector");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick(format!(
            "{COLLIDER_COMPONENT_TYPE} [x]"
        )));
        ui.inject_event(UiEvent::ButtonClick(format!(
            "{COLLIDER_COMPONENT_TYPE}/shape"
        )));
        ui.inject_event(UiEvent::SliderDrag(
            format!("{COLLIDER_COMPONENT_TYPE}/shape/hx"),
            2.0,
        ));

        let commands = panel.ui(&mut ui, &editor_scene.scene, Some(&selected));
        assert_eq!(commands.len(), 1);
        editor_scene
            .execute(commands.into_iter().next().unwrap())
            .unwrap();
        let shape = &component(&editor_scene, &selected, COLLIDER_COMPONENT_TYPE)
            .unwrap()
            .fields["shape"];
        let Value::Map(shape) = shape else {
            panic!("shape map");
        };
        let Value::Map(params) = &shape["params"] else {
            panic!("params map");
        };
        assert_eq!(params["hx"], Value::Float32(2.0));
        assert_eq!(params["hy"], Value::Float32(0.5));
    }

    #[test]
    fn known_enum_fields_cycle_without_writing_unknown_values() {
        let selected = "camera-target".to_string();
        let target = EntityRecord {
            persistent_id: selected.clone(),
            parent: None,
            name: None,
            enabled: true,
            components: BTreeMap::new(),
        };
        let camera = default_camera(&target);
        let mut scene = engine_scene::sample_scene();
        let mut target = target;
        target
            .components
            .insert(CAMERA_COMPONENT_TYPE.into(), camera);
        scene.entities.push(target);
        let mut editor_scene = EditorScene::new(scene);
        let mut panel = InspectorPanel::new("Inspector");
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick(format!("{CAMERA_COMPONENT_TYPE} [x]")));
        ui.inject_event(UiEvent::ButtonClick(format!(
            "{CAMERA_COMPONENT_TYPE}/projection: Perspective"
        )));

        let commands = panel.ui(&mut ui, &editor_scene.scene, Some(&selected));
        assert_eq!(commands.len(), 1);
        editor_scene
            .execute(commands.into_iter().next().unwrap())
            .unwrap();
        assert_eq!(
            component(&editor_scene, &selected, CAMERA_COMPONENT_TYPE)
                .unwrap()
                .fields["projection"],
            Value::Enum("Orthographic".into())
        );
    }
}
