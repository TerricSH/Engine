use super::*;
use engine_scene::Component;

fn entity(id: &str, parent: Option<&str>) -> EntityRecord {
    EntityRecord {
        persistent_id: id.into(),
        parent: parent.map(str::to_owned),
        name: Some(id.into()),
        enabled: true,
        components: BTreeMap::new(),
    }
}

fn hierarchy_scene() -> Scene {
    let mut scene = engine_scene::sample_scene();
    scene.scene_settings.active_camera = None;
    scene.entities = vec![
        entity("external", None),
        entity("root", Some("external")),
        entity("other-root", None),
        entity("child", Some("root")),
        entity("child-copy", None),
        entity("grandchild", Some("child")),
        entity("tail", None),
    ];
    let component = ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: BTreeMap::from([
            ("internal".into(), Value::Entity("child".into())),
            (
                "nested".into(),
                Value::Map(BTreeMap::from([(
                    "target".into(),
                    Value::List(vec![Value::Entity("grandchild".into())]),
                )])),
            ),
            ("external".into(), Value::Entity("external".into())),
        ]),
    };
    scene.entities[1]
        .components
        .insert("test.references".into(), component);
    scene
}

struct UndoFails;

impl Command for UndoFails {
    fn name(&self) -> &str {
        "Undo Fails"
    }

    fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Ok(())
    }

    fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Err(EditorError::InitFailed("undo rejected".into()))
    }
}

struct RedoFails {
    executions: usize,
}

impl Command for RedoFails {
    fn name(&self) -> &str {
        "Redo Fails"
    }

    fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        self.executions += 1;
        if self.executions > 1 {
            Err(EditorError::InitFailed("redo rejected".into()))
        } else {
            Ok(())
        }
    }

    fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Ok(())
    }
}

struct MutatesThenFails;

impl Command for MutatesThenFails {
    fn name(&self) -> &str {
        "Mutates Then Fails"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene.entities.push(entity("partial", None));
        Err(EditorError::InitFailed("forward rejected".into()))
    }

    fn undo(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Ok(())
    }
}

struct UndoMutatesThenFails;

impl Command for UndoMutatesThenFails {
    fn name(&self) -> &str {
        "Undo Mutates Then Fails"
    }

    fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene.entities.clear();
        Err(EditorError::InitFailed("undo rejected".into()))
    }
}

struct UndoProducesInvalidScene;

impl Command for UndoProducesInvalidScene {
    fn name(&self) -> &str {
        "Undo Produces Invalid Scene"
    }

    fn execute(&mut self, _scene: &mut Scene) -> Result<(), EditorError> {
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene.entities[1]
            .components
            .get_mut("engine.renderable")
            .unwrap()
            .fields
            .insert("visible".into(), Value::Str("not-a-bool".into()));
        Ok(())
    }
}

struct RedoProducesInvalidScene {
    executions: usize,
}

impl Command for RedoProducesInvalidScene {
    fn name(&self) -> &str {
        "Redo Produces Invalid Scene"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.executions += 1;
        if self.executions > 1 {
            scene.entities[1]
                .components
                .get_mut("engine.renderable")
                .unwrap()
                .fields
                .insert("visible".into(), Value::Str("not-a-bool".into()));
        }
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        scene.entities[1]
            .components
            .get_mut("engine.renderable")
            .unwrap()
            .fields
            .insert("visible".into(), Value::Bool(true));
        Ok(())
    }
}

struct TestExternal {
    _value: u64,
}

impl engine_scene::Component for TestExternal {
    const TYPE_ID: &'static str = "test.validated_external";
}

struct WrongExternal;

impl engine_scene::Component for WrongExternal {
    const TYPE_ID: &'static str = "test.wrong_external";
}

fn test_external_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
    Box::new(engine_scene::SparseSet::<TestExternal>::new())
}

fn deserialize_test_external(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    match fields.get("value") {
        Some(Value::UInt(value)) => Box::new(TestExternal { _value: *value }),
        _ => Box::new(WrongExternal),
    }
}

fn strict_test_registry() -> Arc<ComponentRegistry> {
    let mut registry = ComponentRegistry::new();
    registry.register_core();
    registry
        .register(engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: TestExternal::TYPE_ID,
                display_name: "Validated External",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: engine_scene::ScriptAccess::None,
            },
            storage_factory: test_external_storage,
            serialize: None,
            deserialize: Some(deserialize_test_external),
        })
        .unwrap();
    Arc::new(registry)
}

#[path = "tests/architecture.rs"]
mod architecture;
#[path = "tests/components.rs"]
mod components;
#[path = "tests/entities.rs"]
mod entities;
#[path = "tests/history.rs"]
mod history;
