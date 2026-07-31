use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

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

/// Frame-local pointing-device state for selection and tactical targeting.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayPointerSnapshot {
    pub position: [f32; 2],
    pub delta: [f32; 2],
    pub scroll: [f32; 2],
    pub viewport: [f32; 2],
    pub primary_down: bool,
    pub primary_pressed: bool,
    pub primary_released: bool,
    pub secondary_down: bool,
    pub middle_down: bool,
    pub focused: bool,
    pub inside_viewport: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ray_origin: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ray_direction: Option<[f32; 3]>,
}

/// Active renderer camera snapshot paired with [`GameplayPointerSnapshot`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayCameraSnapshot {
    pub entity_id: Option<String>,
    pub perspective: bool,
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub viewport: [f32; 4],
    pub view_projection: [f32; 16],
    pub inverse_view_projection: [f32; 16],
}

pub const MAX_SCRIPT_SAVE_STATE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PENDING_SAVE_REQUESTS: usize = 8;
pub const MAX_PENDING_LOGIC_ASSET_QUERIES: usize = 256;
pub const MAX_SCRIPT_LOGIC_ASSET_JSON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ANIMATION_SPEED: f32 = 16.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum GameplayAnimationParameterValue {
    Float(f32),
    Int(i32),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplaySaveEventKind {
    Saved,
    Loaded,
    Failed,
}

/// Completion of a deferred save-slot request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplaySaveEvent {
    pub slot: String,
    pub kind: GameplaySaveEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayLogicAssetResult {
    pub query_id: u32,
    pub asset_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GameplayInputTransitions {
    pub fn was_pressed(&self, action: &str) -> bool {
        self.pressed.contains(action)
    }

    pub fn was_released(&self, action: &str) -> bool {
        self.released.contains(action)
    }
}
