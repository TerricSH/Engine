use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayXrSnapshot {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<GameplayXrFrame>,
    #[serde(default)]
    pub actions: BTreeMap<String, GameplayXrActionValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayXrFrame {
    pub predicted_display_time_nanoseconds: i64,
    pub should_render: bool,
    pub views: [GameplayXrView; 2],
    pub head: GameplayXrPose,
    pub left_hand: GameplayXrPose,
    pub right_hand: GameplayXrPose,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayXrPose {
    pub orientation: [f32; 4],
    pub position: [f32; 3],
    pub orientation_valid: bool,
    pub position_valid: bool,
    pub tracked: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayXrFieldOfView {
    pub angle_left: f32,
    pub angle_right: f32,
    pub angle_up: f32,
    pub angle_down: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameplayXrView {
    pub pose: GameplayXrPose,
    pub fov: GameplayXrFieldOfView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum GameplayXrActionValue {
    Boolean(bool),
    Float(f32),
    Vector2([f32; 2]),
    Pose(GameplayXrPose),
}
