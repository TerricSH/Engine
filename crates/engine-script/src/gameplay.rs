//! Data-only gameplay bridge shared by script hosts and the engine runtime.
//!
//! Process-based hosts cannot call the engine's in-process FFI directly. The
//! runtime therefore sends each script a frame snapshot and applies the
//! commands returned after the lifecycle call completes.

use std::collections::{BTreeMap, BTreeSet};

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

/// One gameplay-facing click emitted by the runtime UI for the current frame.
///
/// The event identifies both the source canvas and element even when no
/// callback id was authored. Runtime UI currently reports click events only;
/// Toggle, Checkbox, and Slider values are not changed automatically through
/// this bridge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayUiEvent {
    pub canvas_id: String,
    pub element_id: u32,
    pub callback_id: Option<String>,
}

/// Frame-local data made available to one script instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayContext {
    pub entity_id: String,
    pub transform: Option<ScriptTransform>,
    pub input_actions: BTreeMap<String, GameplayInputValue>,
    #[serde(default)]
    pub input_transitions: GameplayInputTransitions,
    /// Collision and trigger events involving the owning entity this frame.
    #[serde(default)]
    pub physics_events: Vec<GameplayPhysicsEvent>,
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
        }
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

/// A validated command paired with the entity that owns the script instance.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedGameplayCommand {
    pub entity_id: String,
    pub command: GameplayCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_json_contract_roundtrips_typed_input() {
        let context = GameplayContext {
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
            ui_events: vec![GameplayUiEvent {
                canvas_id: "main-menu".into(),
                element_id: 17,
                callback_id: Some("start-game".into()),
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
        assert!(json.contains(r#""jump":{"type":"Bool","value":true}"#));
        assert!(json.contains(r#""pressed":["jump"]"#));
        assert!(json.contains(r#""kind":"collision_entered""#));
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
        assert!(context.physics_events.is_empty());
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
        };
        assert_eq!(
            serde_json::to_string(&without_callback).unwrap(),
            r#"{"canvas_id":"hud","element_id":43,"callback_id":null}"#
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
}
