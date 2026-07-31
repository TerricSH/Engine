use serde::{Deserialize, Serialize};

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
    JointBroken,
}

/// Entity-relative physics event exposed through the gameplay snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayPhysicsEvent {
    pub kind: GameplayPhysicsEventKind,
    pub other_entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torque: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayDamageKind {
    #[default]
    Generic,
    Impact,
    Bullet,
    Blast,
    Fire,
}

/// Frame-local result of one accepted damage request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayDamageEvent {
    pub target_entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_entity_id: Option<String>,
    pub damage_kind: GameplayDamageKind,
    pub raw_damage: f32,
    pub applied_damage: f32,
    pub remaining_health: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_position: Option<[f32; 3]>,
    pub impulse: [f32; 3],
    pub broke: bool,
    #[serde(default)]
    pub spawned_entity_ids: Vec<String>,
}

pub const MAX_DAMAGE_AMOUNT: f32 = 1_000_000.0;
pub const MAX_PENDING_DAMAGE_REQUESTS: usize = 256;
pub const MAX_RAGDOLL_RECOVERY_SECONDS: f32 = 30.0;
pub const MAX_PENDING_RAGDOLL_REQUESTS: usize = 64;
/// Maximum movement speed a script may request from a character controller.
pub const MAX_CHARACTER_CONTROL_SPEED: f32 = 100.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayRagdollEvent {
    pub entity_id: String,
    pub active: bool,
    pub recovering: bool,
    #[serde(default)]
    pub body_entity_ids: Vec<String>,
}
