//! Data-only gameplay bridge shared by script hosts and the engine runtime.
//!
//! Process-based hosts cannot call the engine's in-process FFI directly. The
//! runtime therefore sends each script a frame snapshot and applies the
//! commands returned after the lifecycle call completes.

use std::collections::{BTreeMap, BTreeSet};

use engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA;
use serde::{Deserialize, Serialize};

fn default_gameplay_script_api_schema() -> String {
    GAMEPLAY_SCRIPT_API_SCHEMA.to_owned()
}

/// Transform data exposed to a script for its owning entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// Read-only frame snapshot for one persistent ECS entity.
///
/// More component snapshots can be added compatibly in later protocol
/// revisions. A missing Transform is represented explicitly instead of
/// omitting the entity, so managed code can distinguish an entity without a
/// Transform from an entity that does not exist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayEntitySnapshot {
    pub transform: Option<ScriptTransform>,
}

/// Resolved value of a project input action for the current frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum GameplayInputValue {
    Bool(bool),
    Float(f32),
    Vec2([f32; 2]),
}

/// Edge transitions for resolved project input actions in one frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayInputTransitions {
    #[serde(default)]
    pub pressed: BTreeSet<String>,
    #[serde(default)]
    pub released: BTreeSet<String>,
}

impl GameplayInputTransitions {
    pub fn was_pressed(&self, action: &str) -> bool {
        self.pressed.contains(action)
    }

    pub fn was_released(&self, action: &str) -> bool {
        self.released.contains(action)
    }
}

/// Kind of physics interaction reported to a script for the current frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayPhysicsEventKind {
    CollisionEntered,
    CollisionStayed,
    CollisionExited,
    TriggerEntered,
    TriggerStayed,
    TriggerExited,
}

/// Entity-relative physics event exposed through the gameplay snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayPhysicsEvent {
    pub kind: GameplayPhysicsEventKind,
    pub other_entity_id: String,
}

/// Upper bound applied to script physics query distances and radii.
///
/// Script-provided distances are clamped to this value before touching the
/// physics backend so a misbehaving script cannot force unbounded native
/// work through the gameplay bridge.
pub const MAX_PHYSICS_QUERY_DISTANCE: f32 = 10_000.0;

/// Maximum persistent entity ids returned by a single overlap query.
pub const MAX_PHYSICS_OVERLAP_RESULTS: usize = 64;

/// Maximum script physics queries the runtime buffers from one command
/// drain. Queries beyond the cap are rejected with a script diagnostic.
pub const MAX_PENDING_PHYSICS_QUERIES: usize = 256;

/// Active physics query requested by a script through the gameplay bridge.
///
/// Queries travel as deferred gameplay commands: the engine validates and
/// executes them against the physics world at the frame boundary and
/// delivers the matching [`GameplayPhysicsQueryResult`] with the next
/// frame's snapshot. Scripts correlate requests and results through the
/// caller-chosen `query_id`; scripts never receive raw ECS handles or
/// backend objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayPhysicsQuery {
    /// Cast a ray and report the closest collider hit, if any.
    Raycast {
        /// Script-chosen correlator echoed back with the result.
        query_id: u32,
        /// World-space ray origin.
        origin: [f32; 3],
        /// Ray direction. Does not need to be normalised; the engine
        /// normalises before querying. Must not be zero length.
        direction: [f32; 3],
        /// Maximum travel distance, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        max_distance: f32,
    },
    /// Find every collider overlapping a world-space sphere.
    OverlapSphere {
        /// Script-chosen correlator echoed back with the result.
        query_id: u32,
        /// World-space sphere centre.
        center: [f32; 3],
        /// Sphere radius, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        radius: f32,
    },
}

impl GameplayPhysicsQuery {
    /// The script-chosen correlator carried by this query.
    pub fn query_id(&self) -> u32 {
        match self {
            Self::Raycast { query_id, .. } | Self::OverlapSphere { query_id, .. } => *query_id,
        }
    }

    /// Validate untrusted query data received from a script host.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Raycast {
                origin,
                direction,
                max_distance,
                ..
            } => {
                if !origin.iter().all(|value| value.is_finite()) {
                    return Err("raycast origin must contain only finite values".into());
                }
                if !direction.iter().all(|value| value.is_finite()) {
                    return Err("raycast direction must contain only finite values".into());
                }
                if direction.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON {
                    return Err("raycast direction must not be zero length".into());
                }
                if !max_distance.is_finite() || *max_distance <= 0.0 {
                    return Err("raycast max_distance must be finite and greater than zero".into());
                }
                Ok(())
            }
            Self::OverlapSphere { center, radius, .. } => {
                if !center.iter().all(|value| value.is_finite()) {
                    return Err("overlap sphere center must contain only finite values".into());
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err("overlap sphere radius must be finite and greater than zero".into());
                }
                Ok(())
            }
        }
    }
}

/// Outcome of a script physics query, delivered with the next frame
/// snapshot following the frame that issued the query.
///
/// Results are frame-local: they appear in exactly one snapshot and are not
/// repeated. Every result echoes the issuing query's `query_id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayPhysicsQueryResult {
    /// A raycast found a collider attached to the given persistent entity.
    RaycastHit {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Persistent entity id of the closest hit — never a raw ECS handle.
        entity_id: String,
        /// World-space intersection point.
        point: [f32; 3],
        /// World-space surface normal at the intersection.
        normal: [f32; 3],
        /// Distance from the ray origin to the intersection.
        distance: f32,
    },
    /// A raycast found no collider within range.
    RaycastMiss {
        /// Correlator from the issuing query.
        query_id: u32,
    },
    /// Persistent entity ids overlapped by a sphere query, sorted and
    /// bounded to [`MAX_PHYSICS_OVERLAP_RESULTS`].
    OverlapSphere {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Overlapping persistent entity ids — never raw ECS handles.
        entity_ids: Vec<String>,
    },
}

/// Resulting value carried by a stateful runtime-UI event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum GameplayUiValue {
    Bool(bool),
    Float(f32),
}

/// One gameplay-facing click emitted by the runtime UI for the current frame.
///
/// The event identifies both the source canvas and element even when no
/// callback id was authored. Stateful controls also report their retained
/// value after the interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayUiEvent {
    pub canvas_id: String,
    pub element_id: u32,
    pub callback_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<GameplayUiValue>,
}

/// RGBA colour used by managed runtime-UI commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayUiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Anchor-and-offset layout sent by a managed gameplay script.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayUiLayout {
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub offset_min: [f32; 2],
    pub offset_max: [f32; 2],
}

/// Viewport scaling policy selected by managed Canvas code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayUiScaleMode {
    #[default]
    Fixed,
    FitWidth,
    FitHeight,
}

/// One retained UI element authored through the managed class API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayUiElement {
    Panel {
        layout: GameplayUiLayout,
        color: GameplayUiColor,
        z_order: i32,
    },
    Image {
        layout: GameplayUiLayout,
        texture_id: String,
        color: GameplayUiColor,
        z_order: i32,
    },
    Text {
        layout: GameplayUiLayout,
        text: String,
        font_size: f32,
        color: GameplayUiColor,
        z_order: i32,
    },
    Button {
        layout: GameplayUiLayout,
        label: String,
        normal_color: GameplayUiColor,
        hover_color: GameplayUiColor,
        pressed_color: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Toggle {
        layout: GameplayUiLayout,
        label: String,
        is_on: bool,
        color_on: GameplayUiColor,
        color_off: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Checkbox {
        layout: GameplayUiLayout,
        label: String,
        checked: bool,
        color: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Slider {
        layout: GameplayUiLayout,
        label: String,
        value: f32,
        min: f32,
        max: f32,
        callback_id: Option<String>,
        z_order: i32,
    },
    ScrollView {
        layout: GameplayUiLayout,
        content_width: f32,
        content_height: f32,
        color: GameplayUiColor,
        z_order: i32,
    },
}

/// Deferred UI mutations emitted by the managed `UICanvas`/`UIElement`
/// classes. The engine validates and applies these commands at the same frame
/// boundary as scene mutations, so process-host scripts never receive native
/// pointers or ECS handles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameplayUiCommand {
    CreateCanvas {
        canvas_id: String,
        width: f32,
        height: f32,
    },
    RemoveCanvas {
        canvas_id: String,
    },
    ResizeCanvas {
        canvas_id: String,
        width: f32,
        height: f32,
    },
    SetCanvasScaleMode {
        canvas_id: String,
        scale_mode: GameplayUiScaleMode,
    },
    ClearCanvas {
        canvas_id: String,
    },
    AddElement {
        canvas_id: String,
        element_id: u32,
        element: GameplayUiElement,
    },
    RemoveElement {
        canvas_id: String,
        element_id: u32,
    },
    SetElementEnabled {
        canvas_id: String,
        element_id: u32,
        enabled: bool,
    },
    SetText {
        canvas_id: String,
        element_id: u32,
        text: String,
    },
    SetToggleValue {
        canvas_id: String,
        element_id: u32,
        is_on: bool,
    },
    SetCheckboxValue {
        canvas_id: String,
        element_id: u32,
        checked: bool,
    },
    SetSliderValue {
        canvas_id: String,
        element_id: u32,
        value: f32,
    },
}

/// Frame-local data made available to one script instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayContext {
    /// Versioned boundary implemented by the engine-owned generated C# API.
    ///
    /// The default preserves compatibility with contexts recorded before the
    /// explicit handshake was added. New runtimes always serialize this field,
    /// and managed hosts reject a different schema before invoking game code.
    #[serde(default = "default_gameplay_script_api_schema")]
    pub script_api: String,
    pub entity_id: String,
    pub transform: Option<ScriptTransform>,
    pub input_actions: BTreeMap<String, GameplayInputValue>,
    #[serde(default)]
    pub input_transitions: GameplayInputTransitions,
    /// Collision and trigger events involving the owning entity this frame.
    #[serde(default)]
    pub physics_events: Vec<GameplayPhysicsEvent>,
    /// Results of physics queries issued by the owning script instance in a
    /// previous frame.
    ///
    /// Queries are deferred commands: the engine executes them at the frame
    /// boundary and answers with the next frame's snapshot. `default` keeps
    /// contexts produced before physics queries existed compatible with the
    /// current script hosts.
    #[serde(default)]
    pub physics_query_results: Vec<GameplayPhysicsQueryResult>,
    /// Runtime UI clicks delivered during this frame.
    ///
    /// `default` keeps contexts produced before gameplay UI events were added
    /// compatible with current script hosts.
    #[serde(default)]
    pub ui_events: Vec<GameplayUiEvent>,
    /// Snapshot of every persistent entity in the active World.
    ///
    /// `default` keeps contexts produced by older runtimes deserializable by
    /// the current API assembly.
    #[serde(default)]
    pub entities: BTreeMap<String, GameplayEntitySnapshot>,
}

/// Mutations a script may request after running a lifecycle method.
///
/// Every command is still bound to the instance's owning entity by the Rust
/// manager. Commands with an explicit target carry a *persistent entity id*,
/// which the runtime validates and resolves against the current World at the
/// frame boundary; a script can never forge an ECS handle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameplayCommand {
    SetTransform {
        transform: ScriptTransform,
    },
    /// Replace another persistent entity's Transform at the frame boundary.
    SetEntityTransform {
        entity_id: String,
        transform: ScriptTransform,
    },
    /// Create a persistent entity with a Transform at the frame boundary.
    CreateEntity {
        entity_id: String,
        transform: ScriptTransform,
    },
    /// Destroy the script's owning entity at the frame boundary.
    DestroySelf,
    /// Destroy another persistent entity at the frame boundary.
    DestroyEntity {
        entity_id: String,
    },
    /// Request a scene change after the current script update finishes.
    ///
    /// The runtime resolves `scene_id` against the project's named scene
    /// catalog. Process hosts validate this value before exposing the command
    /// to the engine, but other hosts can call [`Self::validate`] explicitly.
    LoadScene {
        scene_id: String,
    },
    /// Instantiate a cooked prefab asset at the frame boundary.
    ///
    /// The runtime resolves `prefab_id` against the prefab assets loaded from
    /// the project's cooked asset batch. The spawned instance root receives
    /// the first free persistent id from `prefab_id`, `prefab_id-2`, and so
    /// on; every other prefab entity receives `<rootId>.<prefab-local id>`
    /// (with the same `-N` conflict suffix). `translation`, when present,
    /// overrides the root entity's translation while keeping the prefab's
    /// rotation and scale.
    SpawnPrefab {
        prefab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        translation: Option<[f32; 3]>,
    },
    /// Mutate retained runtime UI through the managed class API.
    Ui {
        command: GameplayUiCommand,
    },
    /// Request an active physics query against the current physics world.
    ///
    /// The query is validated and executed at the frame boundary; the result
    /// arrives in the next frame's [`GameplayContext::physics_query_results`].
    PhysicsQuery {
        query: GameplayPhysicsQuery,
    },
}

impl GameplayCommand {
    /// Validate untrusted command data received from a script host.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::SetTransform { transform } => validate_script_transform(transform),
            Self::SetEntityTransform {
                entity_id,
                transform,
            }
            | Self::CreateEntity {
                entity_id,
                transform,
            } => {
                validate_entity_id(entity_id)?;
                validate_script_transform(transform)
            }
            Self::DestroySelf => Ok(()),
            Self::DestroyEntity { entity_id } => validate_entity_id(entity_id),
            Self::LoadScene { scene_id } => validate_scene_id(scene_id),
            Self::SpawnPrefab {
                prefab_id,
                translation,
            } => {
                validate_prefab_id(prefab_id)?;
                if let Some(translation) = translation {
                    if !translation.iter().all(|value| value.is_finite()) {
                        return Err("spawn translation must contain only finite values".into());
                    }
                }
                Ok(())
            }
            Self::Ui { command } => command.validate(),
            Self::PhysicsQuery { query } => query.validate(),
        }
    }
}

impl GameplayUiCommand {
    /// Validate UI data received from an untrusted process-host script.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::CreateCanvas {
                canvas_id,
                width,
                height,
            }
            | Self::ResizeCanvas {
                canvas_id,
                width,
                height,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_canvas_size(*width, *height)
            }
            Self::RemoveCanvas { canvas_id } | Self::ClearCanvas { canvas_id } => {
                validate_canvas_id(canvas_id)
            }
            Self::SetCanvasScaleMode { canvas_id, .. } => validate_canvas_id(canvas_id),
            Self::AddElement {
                canvas_id,
                element_id,
                element,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                element.validate()
            }
            Self::RemoveElement {
                canvas_id,
                element_id,
            }
            | Self::SetElementEnabled {
                canvas_id,
                element_id,
                ..
            }
            | Self::SetToggleValue {
                canvas_id,
                element_id,
                ..
            }
            | Self::SetCheckboxValue {
                canvas_id,
                element_id,
                ..
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)
            }
            Self::SetText {
                canvas_id,
                element_id,
                text,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                validate_ui_text(text, "text")
            }
            Self::SetSliderValue {
                canvas_id,
                element_id,
                value,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                if value.is_finite() {
                    Ok(())
                } else {
                    Err("UI slider value must be finite".into())
                }
            }
        }
    }
}

impl GameplayUiElement {
    fn validate(&self) -> Result<(), String> {
        let layout = match self {
            Self::Panel { layout, .. }
            | Self::Image { layout, .. }
            | Self::Text { layout, .. }
            | Self::Button { layout, .. }
            | Self::Toggle { layout, .. }
            | Self::Checkbox { layout, .. }
            | Self::Slider { layout, .. }
            | Self::ScrollView { layout, .. } => layout,
        };
        validate_ui_layout(layout)?;

        match self {
            Self::Panel { .. } => Ok(()),
            Self::Image { texture_id, .. } => validate_ui_asset_id(texture_id),
            Self::Text {
                text, font_size, ..
            } => {
                validate_ui_text(text, "text")?;
                if font_size.is_finite() && *font_size > 0.0 && *font_size <= 512.0 {
                    Ok(())
                } else {
                    Err("UI font_size must be finite and in the range (0, 512]".into())
                }
            }
            Self::Button {
                label, callback_id, ..
            }
            | Self::Toggle {
                label, callback_id, ..
            }
            | Self::Checkbox {
                label, callback_id, ..
            } => {
                validate_ui_text(label, "label")?;
                validate_ui_callback_id(callback_id.as_deref())
            }
            Self::Slider {
                label,
                value,
                min,
                max,
                callback_id,
                ..
            } => {
                validate_ui_text(label, "label")?;
                validate_ui_callback_id(callback_id.as_deref())?;
                if !value.is_finite() || !min.is_finite() || !max.is_finite() || min > max {
                    return Err(
                        "UI slider value/min/max must be finite and min must not exceed max".into(),
                    );
                }
                if *value < *min || *value > *max {
                    return Err("UI slider value must be between min and max".into());
                }
                Ok(())
            }
            Self::ScrollView {
                content_width,
                content_height,
                ..
            } => {
                if content_width.is_finite()
                    && content_height.is_finite()
                    && *content_width >= 0.0
                    && *content_height >= 0.0
                {
                    Ok(())
                } else {
                    Err("UI scroll-view content dimensions must be finite and non-negative".into())
                }
            }
        }
    }
}

fn validate_canvas_id(canvas_id: &str) -> Result<(), String> {
    validate_entity_id(canvas_id).map_err(|reason| format!("invalid canvas id: {reason}"))
}

fn validate_canvas_size(width: f32, height: f32) -> Result<(), String> {
    if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
        Ok(())
    } else {
        Err("UI canvas width and height must be finite and greater than zero".into())
    }
}

fn validate_ui_element_id(element_id: u32) -> Result<(), String> {
    if element_id > 0 && element_id != u32::MAX {
        Ok(())
    } else {
        Err("UI element_id must be between 1 and 4294967294".into())
    }
}

fn validate_ui_layout(layout: &GameplayUiLayout) -> Result<(), String> {
    if !layout
        .anchor_min
        .iter()
        .chain(layout.anchor_max.iter())
        .chain(layout.offset_min.iter())
        .chain(layout.offset_max.iter())
        .all(|value| value.is_finite())
    {
        return Err("UI layout anchors and offsets must contain only finite values".into());
    }
    if layout
        .anchor_min
        .iter()
        .chain(layout.anchor_max.iter())
        .any(|value| !(0.0..=1.0).contains(value))
    {
        return Err("UI layout anchors must be in the range [0, 1]".into());
    }
    Ok(())
}

fn validate_ui_text(value: &str, field: &str) -> Result<(), String> {
    if value.len() <= 16_384
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        Ok(())
    } else {
        Err(format!(
            "UI {field} must contain at most 16384 bytes and no control characters"
        ))
    }
}

fn validate_ui_asset_id(asset_id: &str) -> Result<(), String> {
    if !asset_id.is_empty() && asset_id.len() <= 256 && !asset_id.chars().any(char::is_control) {
        Ok(())
    } else {
        Err("UI texture_id must contain 1 to 256 bytes and no control characters".into())
    }
}

fn validate_ui_callback_id(callback_id: Option<&str>) -> Result<(), String> {
    let Some(callback_id) = callback_id else {
        return Ok(());
    };
    if !callback_id.is_empty()
        && callback_id.len() <= 128
        && !callback_id.chars().any(char::is_control)
    {
        Ok(())
    } else {
        Err("UI callback_id must contain 1 to 128 bytes and no control characters".into())
    }
}

/// Validate a persistent entity id received from untrusted script code.
///
/// Entity ids are identifiers, never paths. Keeping the wire-safe subset
/// deliberately small prevents traversal-like strings and control characters
/// from crossing into diagnostics, lookup tables, or future persistence APIs.
pub fn validate_entity_id(entity_id: &str) -> Result<(), String> {
    let valid = !entity_id.is_empty()
        && entity_id.len() <= 128
        && entity_id != "."
        && entity_id != ".."
        && entity_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid entity id {entity_id:?}: expected 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..'); entity ids are not file paths"
        ))
    }
}

/// Validate a Transform received from an untrusted script host.
pub fn validate_script_transform(transform: &ScriptTransform) -> Result<(), String> {
    if !transform
        .translation
        .iter()
        .chain(transform.rotation.iter())
        .chain(transform.scale.iter())
        .all(|value| value.is_finite())
    {
        return Err("translation, rotation, and scale must contain only finite values".into());
    }
    let rotation_length_squared = transform
        .rotation
        .iter()
        .map(|value| value * value)
        .sum::<f32>();
    if rotation_length_squared <= f32::EPSILON {
        return Err("rotation quaternion must not be zero length".into());
    }
    Ok(())
}

/// Validate a project scene catalog identifier.
///
/// This intentionally mirrors the portable scene-id contract used by project
/// manifests without making the script protocol depend on `engine-asset`.
pub fn validate_scene_id(scene_id: &str) -> Result<(), String> {
    let valid = !scene_id.is_empty()
        && scene_id.len() <= 128
        && scene_id != "."
        && scene_id != ".."
        && scene_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid scene id {scene_id:?}: use a key from game.project.json `scenes` containing 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..')"
        ))
    }
}

/// Validate a cooked prefab asset identifier used by `Scene.Spawn`.
///
/// The spawned instance root takes this id as the base of its persistent
/// entity id, so prefab ids share the wire-safe identifier contract of
/// persistent entity ids. Authors choose asset ids, so keeping the two
/// contracts aligned is always possible.
pub fn validate_prefab_id(prefab_id: &str) -> Result<(), String> {
    validate_entity_id(prefab_id).map_err(|_| {
        format!(
            "invalid prefab id {prefab_id:?}: expected a cooked prefab asset id containing 1 to 128 ASCII letters, digits, hyphens, underscores, or dots (but not '.' or '..'); prefab ids are not file paths"
        )
    })
}

/// A validated command paired with the entity that owns the script instance.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayCommand {
    pub entity_id: String,
    pub command: GameplayCommand,
}

/// A validated physics query paired with the entity that owns the script
/// instance that issued it.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayPhysicsQuery {
    pub entity_id: String,
    pub query: GameplayPhysicsQuery,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_json_contract_roundtrips_typed_input() {
        let context = GameplayContext {
            script_api: GAMEPLAY_SCRIPT_API_SCHEMA.to_owned(),
            entity_id: "player".into(),
            transform: Some(ScriptTransform {
                translation: [1.0, 2.0, 3.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            }),
            input_actions: BTreeMap::from([
                ("jump".into(), GameplayInputValue::Bool(true)),
                ("move".into(), GameplayInputValue::Vec2([0.25, -0.5])),
            ]),
            input_transitions: GameplayInputTransitions {
                pressed: BTreeSet::from(["jump".into()]),
                released: BTreeSet::new(),
            },
            physics_events: vec![GameplayPhysicsEvent {
                kind: GameplayPhysicsEventKind::CollisionEntered,
                other_entity_id: "floor".into(),
            }],
            physics_query_results: vec![
                GameplayPhysicsQueryResult::RaycastHit {
                    query_id: 7,
                    entity_id: "floor".into(),
                    point: [1.0, 0.5, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    distance: 4.5,
                },
                GameplayPhysicsQueryResult::RaycastMiss { query_id: 8 },
                GameplayPhysicsQueryResult::OverlapSphere {
                    query_id: 9,
                    entity_ids: vec!["floor".into()],
                },
            ],
            ui_events: vec![GameplayUiEvent {
                canvas_id: "main-menu".into(),
                element_id: 17,
                callback_id: Some("start-game".into()),
                value: None,
            }],
            entities: BTreeMap::from([(
                "player".into(),
                GameplayEntitySnapshot {
                    transform: Some(ScriptTransform {
                        translation: [1.0, 2.0, 3.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0, 1.0, 1.0],
                    }),
                },
            )]),
        };

        let json = serde_json::to_string(&context).unwrap();
        assert!(json.contains(r#""script_api":"ScriptAPI-v0""#));
        assert!(json.contains(r#""jump":{"type":"Bool","value":true}"#));
        assert!(json.contains(r#""pressed":["jump"]"#));
        assert!(json.contains(r#""kind":"collision_entered""#));
        assert!(json.contains(
            r#""physics_query_results":[{"kind":"raycast_hit","query_id":7,"entity_id":"floor","point":[1.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.5},{"kind":"raycast_miss","query_id":8},{"kind":"overlap_sphere","query_id":9,"entity_ids":["floor"]}]"#
        ));
        assert!(json.contains(
            r#""ui_events":[{"canvas_id":"main-menu","element_id":17,"callback_id":"start-game"}]"#
        ));
        assert_eq!(
            serde_json::from_str::<GameplayContext>(&json).unwrap(),
            context
        );
    }

    #[test]
    fn context_from_older_runtime_defaults_the_entity_snapshot_map() {
        let context: GameplayContext =
            serde_json::from_str(r#"{"entity_id":"player","transform":null,"input_actions":{}}"#)
                .unwrap();
        assert!(context.entities.is_empty());
        assert_eq!(context.script_api, GAMEPLAY_SCRIPT_API_SCHEMA);
        assert!(context.physics_events.is_empty());
        assert!(context.physics_query_results.is_empty());
        assert!(context.ui_events.is_empty());
        assert_eq!(
            context.input_transitions,
            GameplayInputTransitions::default()
        );
    }

    #[test]
    fn gameplay_ui_event_has_a_stable_json_contract() {
        let with_callback = GameplayUiEvent {
            canvas_id: "hud".into(),
            element_id: 42,
            callback_id: Some("pause".into()),
            value: None,
        };
        assert_eq!(
            serde_json::to_string(&with_callback).unwrap(),
            r#"{"canvas_id":"hud","element_id":42,"callback_id":"pause"}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayUiEvent>(
                r#"{"canvas_id":"hud","element_id":42,"callback_id":"pause"}"#
            )
            .unwrap(),
            with_callback
        );

        let without_callback = GameplayUiEvent {
            canvas_id: "hud".into(),
            element_id: 43,
            callback_id: None,
            value: None,
        };
        assert_eq!(
            serde_json::to_string(&without_callback).unwrap(),
            r#"{"canvas_id":"hud","element_id":43,"callback_id":null}"#
        );

        let stateful = GameplayUiEvent {
            canvas_id: "hud".into(),
            element_id: 44,
            callback_id: Some("volume".into()),
            value: Some(GameplayUiValue::Float(0.75)),
        };
        assert_eq!(
            serde_json::to_string(&stateful).unwrap(),
            r#"{"canvas_id":"hud","element_id":44,"callback_id":"volume","value":{"type":"Float","value":0.75}}"#
        );
    }

    #[test]
    fn transform_command_cannot_forge_an_entity_id() {
        let command = GameplayCommand::SetTransform {
            transform: ScriptTransform {
                translation: [4.0, 5.0, 6.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [2.0, 2.0, 2.0],
            },
        };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(
            json,
            r#"{"type":"set_transform","transform":{"translation":[4.0,5.0,6.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[2.0,2.0,2.0]}}"#
        );
        assert!(!json.contains("entity_id"));
    }

    #[test]
    fn explicit_entity_commands_have_a_stable_validated_contract() {
        let transform = ScriptTransform {
            translation: [4.0, 5.0, 6.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [2.0, 2.0, 2.0],
        };
        let command = GameplayCommand::SetEntityTransform {
            entity_id: "enemy-01".into(),
            transform: transform.clone(),
        };
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"type":"set_entity_transform","entity_id":"enemy-01","transform":{"translation":[4.0,5.0,6.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[2.0,2.0,2.0]}}"#
        );
        assert!(command.validate().is_ok());
        let create = GameplayCommand::CreateEntity {
            entity_id: "spawned-01".into(),
            transform: transform.clone(),
        };
        assert_eq!(
            serde_json::to_string(&create).unwrap(),
            r#"{"type":"create_entity","entity_id":"spawned-01","transform":{"translation":[4.0,5.0,6.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[2.0,2.0,2.0]}}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"create_entity","entity_id":"spawned-01","transform":{"translation":[4,5,6],"rotation":[0,0,0,1],"scale":[2,2,2]}}"#
            )
            .unwrap(),
            create
        );
        assert!(create.validate().is_ok());

        let scale = GameplayCommand::Ui {
            command: GameplayUiCommand::SetCanvasScaleMode {
                canvas_id: "hud".into(),
                scale_mode: GameplayUiScaleMode::FitWidth,
            },
        };
        assert_eq!(
            serde_json::to_string(&scale).unwrap(),
            r#"{"type":"ui","command":{"type":"set_canvas_scale_mode","canvas_id":"hud","scale_mode":"fit_width"}}"#
        );
        assert!(scale.validate().is_ok());
        assert_eq!(
            serde_json::to_string(&GameplayCommand::DestroySelf).unwrap(),
            r#"{"type":"destroy_self"}"#
        );
        assert_eq!(
            serde_json::to_string(&GameplayCommand::DestroyEntity {
                entity_id: "enemy-01".into()
            })
            .unwrap(),
            r#"{"type":"destroy_entity","entity_id":"enemy-01"}"#
        );
    }

    #[test]
    fn untrusted_entity_commands_reject_paths_and_non_finite_transforms() {
        for invalid in ["", ".", "..", "../enemy", "enemy/child", "enemy child"] {
            assert!(validate_entity_id(invalid).is_err(), "{invalid:?}");
        }
        let command = GameplayCommand::SetEntityTransform {
            entity_id: "enemy".into(),
            transform: ScriptTransform {
                translation: [f32::INFINITY, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        };
        assert!(command.validate().unwrap_err().contains("finite"));

        for command in [
            GameplayCommand::CreateEntity {
                entity_id: "../spawn".into(),
                transform: ScriptTransform {
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
            GameplayCommand::CreateEntity {
                entity_id: "spawn".into(),
                transform: ScriptTransform {
                    translation: [f32::NAN, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            },
        ] {
            assert!(command.validate().is_err());
        }
    }

    #[test]
    fn spawn_prefab_command_has_a_stable_validated_contract() {
        let bare = GameplayCommand::SpawnPrefab {
            prefab_id: "prefab-enemy".into(),
            translation: None,
        };
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy"}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy"}"#
            )
            .unwrap(),
            bare
        );
        assert!(bare.validate().is_ok());

        let placed = GameplayCommand::SpawnPrefab {
            prefab_id: "prefab-enemy".into(),
            translation: Some([1.0, 2.0, 3.0]),
        };
        assert_eq!(
            serde_json::to_string(&placed).unwrap(),
            r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy","translation":[1.0,2.0,3.0]}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy","translation":[1,2,3]}"#
            )
            .unwrap(),
            placed
        );
        assert!(placed.validate().is_ok());

        for command in [
            GameplayCommand::SpawnPrefab {
                prefab_id: "../enemy".into(),
                translation: None,
            },
            GameplayCommand::SpawnPrefab {
                prefab_id: "prefabs/enemy".into(),
                translation: None,
            },
            GameplayCommand::SpawnPrefab {
                prefab_id: String::new(),
                translation: None,
            },
            GameplayCommand::SpawnPrefab {
                prefab_id: "prefab-enemy".into(),
                translation: Some([f32::NAN, 0.0, 0.0]),
            },
        ] {
            assert!(command.validate().is_err(), "{command:?}");
        }
        assert!(validate_prefab_id("prefab-enemy").is_ok());
        assert!(validate_prefab_id("prefab.enemy_01").is_ok());
        let error = validate_prefab_id("prefabs/enemy").unwrap_err();
        assert!(error.contains("not file paths"), "{error}");
    }

    #[test]
    fn load_scene_command_has_a_stable_json_contract() {
        let command = GameplayCommand::LoadScene {
            scene_id: "level_two".into(),
        };
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"type":"load_scene","scene_id":"level_two"}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"load_scene","scene_id":"level_two"}"#
            )
            .unwrap(),
            command
        );
        assert!(command.validate().is_ok());
    }

    #[test]
    fn managed_ui_commands_have_a_stable_validated_contract() {
        let create = GameplayCommand::Ui {
            command: GameplayUiCommand::CreateCanvas {
                canvas_id: "hud".into(),
                width: 1280.0,
                height: 720.0,
            },
        };
        assert_eq!(
            serde_json::to_string(&create).unwrap(),
            r#"{"type":"ui","command":{"type":"create_canvas","canvas_id":"hud","width":1280.0,"height":720.0}}"#
        );
        assert!(create.validate().is_ok());

        let add = GameplayCommand::Ui {
            command: GameplayUiCommand::AddElement {
                canvas_id: "hud".into(),
                element_id: 1,
                element: GameplayUiElement::Panel {
                    layout: GameplayUiLayout {
                        anchor_min: [0.0, 0.0],
                        anchor_max: [0.0, 0.0],
                        offset_min: [24.0, 24.0],
                        offset_max: [344.0, 56.0],
                    },
                    color: GameplayUiColor {
                        r: 20,
                        g: 20,
                        b: 20,
                        a: 210,
                    },
                    z_order: 10,
                },
            },
        };
        let json = serde_json::to_string(&add).unwrap();
        assert!(json.contains(r#""kind":"panel""#));
        assert!(json.contains(r#""element_id":1"#));
        assert_eq!(serde_json::from_str::<GameplayCommand>(&json).unwrap(), add);
        assert!(add.validate().is_ok());

        let set_slider = GameplayCommand::Ui {
            command: GameplayUiCommand::SetSliderValue {
                canvas_id: "hud".into(),
                element_id: 3,
                value: 0.75,
            },
        };
        assert_eq!(
            serde_json::to_string(&set_slider).unwrap(),
            r#"{"type":"ui","command":{"type":"set_slider_value","canvas_id":"hud","element_id":3,"value":0.75}}"#
        );
        assert!(set_slider.validate().is_ok());
    }

    #[test]
    fn managed_ui_commands_reject_invalid_ids_and_geometry() {
        for command in [
            GameplayUiCommand::CreateCanvas {
                canvas_id: "../hud".into(),
                width: 1280.0,
                height: 720.0,
            },
            GameplayUiCommand::CreateCanvas {
                canvas_id: "hud".into(),
                width: f32::NAN,
                height: 720.0,
            },
            GameplayUiCommand::RemoveElement {
                canvas_id: "hud".into(),
                element_id: 0,
            },
            GameplayUiCommand::SetSliderValue {
                canvas_id: "hud".into(),
                element_id: 1,
                value: f32::INFINITY,
            },
        ] {
            assert!(command.validate().is_err());
        }
    }

    #[test]
    fn scene_id_validation_matches_the_project_catalog_contract() {
        for valid in ["main", "level-two", "level_two", "chapter.2", "A1"] {
            assert!(validate_scene_id(valid).is_ok(), "{valid}");
        }
        for invalid in ["", ".", "..", "level/two", "level two", "关卡"] {
            let error = validate_scene_id(invalid).unwrap_err();
            assert!(error.contains("game.project.json `scenes`"), "{error}");
        }
        assert!(validate_scene_id(&"a".repeat(129)).is_err());
    }

    #[test]
    fn physics_query_command_has_a_stable_validated_contract() {
        let raycast = GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::Raycast {
                query_id: 7,
                origin: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
            },
        };
        assert_eq!(
            serde_json::to_string(&raycast).unwrap(),
            r#"{"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0.0,5.0,0.0],"direction":[0.0,-1.0,0.0],"max_distance":10.0}}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&raycast).unwrap())
                .unwrap(),
            raycast
        );
        assert!(raycast.validate().is_ok());

        let overlap = GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::OverlapSphere {
                query_id: 8,
                center: [1.0, 2.0, 3.0],
                radius: 2.5,
            },
        };
        assert_eq!(
            serde_json::to_string(&overlap).unwrap(),
            r#"{"type":"physics_query","query":{"kind":"overlap_sphere","query_id":8,"center":[1.0,2.0,3.0],"radius":2.5}}"#
        );
        assert!(overlap.validate().is_ok());

        let GameplayCommand::PhysicsQuery { query } = &raycast else {
            panic!("expected physics query command");
        };
        assert_eq!(query.query_id(), 7);
    }

    #[test]
    fn physics_query_results_have_a_stable_json_contract() {
        let hit = GameplayPhysicsQueryResult::RaycastHit {
            query_id: 3,
            entity_id: "cube-01".into(),
            point: [0.0, 0.5, 0.0],
            normal: [0.0, 1.0, 0.0],
            distance: 4.5,
        };
        assert_eq!(
            serde_json::to_string(&hit).unwrap(),
            r#"{"kind":"raycast_hit","query_id":3,"entity_id":"cube-01","point":[0.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.5}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayPhysicsQueryResult>(
                r#"{"kind":"raycast_hit","query_id":3,"entity_id":"cube-01","point":[0,0.5,0],"normal":[0,1,0],"distance":4.5}"#
            )
            .unwrap(),
            hit
        );

        let miss = GameplayPhysicsQueryResult::RaycastMiss { query_id: 4 };
        assert_eq!(
            serde_json::to_string(&miss).unwrap(),
            r#"{"kind":"raycast_miss","query_id":4}"#
        );

        let overlap = GameplayPhysicsQueryResult::OverlapSphere {
            query_id: 5,
            entity_ids: vec!["cube-01".into(), "physics-peer".into()],
        };
        assert_eq!(
            serde_json::to_string(&overlap).unwrap(),
            r#"{"kind":"overlap_sphere","query_id":5,"entity_ids":["cube-01","physics-peer"]}"#
        );
        assert_eq!(
            serde_json::from_str::<GameplayPhysicsQueryResult>(
                &serde_json::to_string(&overlap).unwrap()
            )
            .unwrap(),
            overlap
        );
    }

    #[test]
    fn untrusted_physics_queries_reject_non_finite_and_degenerate_values() {
        let nan_origin = GameplayPhysicsQuery::Raycast {
            query_id: 1,
            origin: [f32::NAN, 0.0, 0.0],
            direction: [0.0, -1.0, 0.0],
            max_distance: 10.0,
        };
        assert!(nan_origin.validate().unwrap_err().contains("finite"));

        let infinite_direction = GameplayPhysicsQuery::Raycast {
            query_id: 1,
            origin: [0.0; 3],
            direction: [f32::INFINITY, 0.0, 0.0],
            max_distance: 10.0,
        };
        assert!(infinite_direction
            .validate()
            .unwrap_err()
            .contains("finite"));

        let zero_direction = GameplayPhysicsQuery::Raycast {
            query_id: 1,
            origin: [0.0; 3],
            direction: [0.0; 3],
            max_distance: 10.0,
        };
        assert!(zero_direction
            .validate()
            .unwrap_err()
            .contains("zero length"));

        for invalid_command in [
            GameplayCommand::PhysicsQuery {
                query: GameplayPhysicsQuery::Raycast {
                    query_id: 1,
                    origin: [0.0; 3],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 0.0,
                },
            },
            GameplayCommand::PhysicsQuery {
                query: GameplayPhysicsQuery::Raycast {
                    query_id: 1,
                    origin: [0.0; 3],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: f32::INFINITY,
                },
            },
            GameplayCommand::PhysicsQuery {
                query: GameplayPhysicsQuery::OverlapSphere {
                    query_id: 2,
                    center: [0.0, f32::NAN, 0.0],
                    radius: 1.0,
                },
            },
            GameplayCommand::PhysicsQuery {
                query: GameplayPhysicsQuery::OverlapSphere {
                    query_id: 2,
                    center: [0.0; 3],
                    radius: -1.0,
                },
            },
        ] {
            assert!(invalid_command.validate().is_err());
        }
    }
}
