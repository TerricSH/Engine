use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use engine_gameplay::{InputAction, InputValue, InputValueType};
use engine_script::{
    GameplayCommand, GameplayContext, ScriptError, ScriptHandle, ScriptHost, ScriptInstance,
    ScriptTransform, ScriptValue,
};
use engine_serialize::{SchemaVersion, Value};

use super::*;

struct InputDrivenInstance {
    context: Option<GameplayContext>,
    commands: Vec<GameplayCommand>,
    destroy_count: Arc<AtomicUsize>,
}

impl ScriptInstance for InputDrivenInstance {
    fn call(&mut self, function: &str, _args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        if function == engine_script::ON_DESTROY {
            self.destroy_count.fetch_add(1, Ordering::SeqCst);
        } else if function == engine_script::ON_UPDATE {
            let context = self
                .context
                .as_ref()
                .expect("gameplay context before update");
            if context.input_actions.get("jump")
                == Some(&engine_script::GameplayInputValue::Bool(true))
            {
                let mut transform = context.transform.clone().expect("owner Transform");
                transform.translation[0] += 2.0;
                self.commands
                    .push(GameplayCommand::SetTransform { transform });
            }
            if context.input_actions.get("load_level")
                == Some(&engine_script::GameplayInputValue::Bool(true))
            {
                self.commands.push(GameplayCommand::LoadScene {
                    scene_id: "level_two".into(),
                });
            }
            if context.input_actions.get("load_other")
                == Some(&engine_script::GameplayInputValue::Bool(true))
            {
                self.commands.push(GameplayCommand::LoadScene {
                    scene_id: "level_three".into(),
                });
            }
            if context.entity_id == "cube-01"
                && context.input_actions.get("move_camera")
                    == Some(&engine_script::GameplayInputValue::Bool(true))
            {
                let mut transform = context.entities["camera-main"]
                    .transform
                    .clone()
                    .expect("camera Transform snapshot");
                transform.translation = [7.0, 8.0, 9.0];
                self.commands.push(GameplayCommand::SetEntityTransform {
                    entity_id: "camera-main".into(),
                    transform,
                });
            }
            if context.entity_id == "cube-01"
                && context.input_actions.get("destroy_camera")
                    == Some(&engine_script::GameplayInputValue::Bool(true))
            {
                self.commands.push(GameplayCommand::DestroyEntity {
                    entity_id: "camera-main".into(),
                });
            }
        }
        Ok(ScriptValue::Null)
    }

    fn set_field(&mut self, _name: &str, _value: ScriptValue) -> Result<(), ScriptError> {
        Ok(())
    }

    fn get_field(&self, _name: &str) -> Option<ScriptValue> {
        None
    }

    fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
        self.context = Some(context.clone());
        Ok(())
    }

    fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
        Ok(std::mem::take(&mut self.commands))
    }
}

struct InputDrivenHost {
    destroy_count: Arc<AtomicUsize>,
}

impl InputDrivenHost {
    fn new(destroy_count: Arc<AtomicUsize>) -> Self {
        Self { destroy_count }
    }
}

impl ScriptHost for InputDrivenHost {
    fn name(&self) -> &str {
        "bridge-test"
    }

    fn load_assembly(
        &mut self,
        id: &str,
        _assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        Ok(ScriptHandle::new(id))
    }

    fn instantiate(
        &mut self,
        _handle: &ScriptHandle,
        _class_name: &str,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        Ok(Box::new(InputDrivenInstance {
            context: None,
            commands: Vec::new(),
            destroy_count: Arc::clone(&self.destroy_count),
        }))
    }

    fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
        Ok(())
    }
}

struct ContextRecordingInstance {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
}

impl ScriptInstance for ContextRecordingInstance {
    fn call(&mut self, _function: &str, _args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        Ok(ScriptValue::Null)
    }

    fn set_field(&mut self, _name: &str, _value: ScriptValue) -> Result<(), ScriptError> {
        Ok(())
    }

    fn get_field(&self, _name: &str) -> Option<ScriptValue> {
        None
    }

    fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
        self.contexts.lock().unwrap().push(context.clone());
        Ok(())
    }
}

struct ContextRecordingHost {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
}

impl ScriptHost for ContextRecordingHost {
    fn name(&self) -> &str {
        "context-recording"
    }

    fn load_assembly(
        &mut self,
        id: &str,
        _assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        Ok(ScriptHandle::new(id))
    }

    fn instantiate(
        &mut self,
        _handle: &ScriptHandle,
        _class_name: &str,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        Ok(Box::new(ContextRecordingInstance {
            contexts: Arc::clone(&self.contexts),
        }))
    }

    fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
        Ok(())
    }
}

#[test]
fn resolved_true_input_reaches_script_and_applies_owner_transform_command() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    let destroy_count = Arc::new(AtomicUsize::new(0));
    game_loop
        .runtime
        .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
    game_loop.runtime.set_script_host_name("bridge-test");
    game_loop
        .runtime
        .load_script_assembly("game", "bridge-test", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    let target = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    target.components.insert(
        "engine.transform".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("translation".into(), Value::Vec3([0.0; 3])),
                ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                ("scale".into(), Value::Vec3([1.0; 3])),
            ]),
        },
    );
    target.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("assembly_id".into(), Value::Str("game".into())),
                ("class_name".into(), Value::Str("Player".into())),
            ]),
        },
    );
    game_loop.load_scene(scene).unwrap();

    let mut jump = InputAction::new("jump", InputValueType::Digital);
    jump.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(jump);
    game_loop.update(1.0 / 60.0);

    let translation = game_loop
        .runtime
        .with_world(|world| {
            world
                .query_all::<engine_scene::components::Transform>()
                .find_map(|(entity, transform)| {
                    (world.persistent_id(entity) == Some("cube-01"))
                        .then_some(transform.translation)
                })
                .unwrap()
        })
        .unwrap();
    assert_eq!(translation, glam::Vec3::new(2.0, 0.0, 0.0));
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}

#[test]
fn entity_snapshot_can_drive_an_explicit_target_transform_command() {
    let destroy_count = Arc::new(AtomicUsize::new(0));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(InputDrivenHost::new(destroy_count)));
    game_loop.runtime.set_script_host_name("bridge-test");
    game_loop
        .runtime
        .load_script_assembly("game", "bridge-test", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "camera-main")
        .unwrap()
        .components
        .insert(
            "engine.transform".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("translation".into(), Value::Vec3([0.0; 3])),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0; 3])),
                ]),
            },
        );
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .components
        .insert(
            "engine.script".into(),
            engine_scene::ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([
                    ("assembly_id".into(), Value::Str("game".into())),
                    ("class_name".into(), Value::Str("Player".into())),
                ]),
            },
        );
    game_loop.load_scene(scene).unwrap();

    let mut move_camera = InputAction::new("move_camera", InputValueType::Digital);
    move_camera.current_value = InputValue::Bool(true);
    game_loop.input_map.add_action(move_camera);
    game_loop.update(1.0 / 60.0);

    let camera_translation = game_loop
        .runtime
        .with_world(|world| {
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            world
                .get::<engine_scene::components::Transform>(camera)
                .unwrap()
                .translation
        })
        .unwrap();
    assert_eq!(camera_translation, glam::Vec3::new(7.0, 8.0, 9.0));
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}
