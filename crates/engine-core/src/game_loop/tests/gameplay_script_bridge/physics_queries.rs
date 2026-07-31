struct PhysicsQueryInstance {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    commands: Vec<GameplayCommand>,
    issued: bool,
}

impl ScriptInstance for PhysicsQueryInstance {
    fn call(&mut self, function: &str, _args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        if function == engine_script::ON_UPDATE && !self.issued {
            self.issued = true;
            let raycast = |query_id, direction| GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::Raycast {
                    query_id,
                    origin: [0.0, 5.0, 0.0],
                    direction,
                    max_distance: 10.0,
                    filter: None,
                },
            };
            // Downward ray hits the owning cube's top face at y = 0.5;
            // the upward ray misses every collider.
            self.commands.push(raycast(11, [0.0, -1.0, 0.0]));
            self.commands.push(raycast(12, [0.0, 1.0, 0.0]));
            self.commands.push(GameplayCommand::PhysicsQuery {
                query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                    query_id: 13,
                    center: [0.0, 0.0, 0.0],
                    radius: 1.0,
                    filter: None,
                },
            });
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
        self.contexts.lock().unwrap().push(context.clone());
        Ok(())
    }

    fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
        Ok(std::mem::take(&mut self.commands))
    }
}

struct PhysicsQueryHost {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
}

impl ScriptHost for PhysicsQueryHost {
    fn name(&self) -> &str {
        "physics-query-test"
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
        Ok(Box::new(PhysicsQueryInstance {
            contexts: Arc::clone(&self.contexts),
            commands: Vec::new(),
            issued: false,
        }))
    }

    fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
        Ok(())
    }
}

fn physics_query_game_loop(contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>) -> GameLoop {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(PhysicsQueryHost {
            contexts: Arc::clone(contexts),
        }));
    game_loop.runtime.set_script_host_name("physics-query-test");
    game_loop
        .runtime
        .load_script_assembly("game", "physics-query-test", b"test")
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
            fields: BTreeMap::new(),
        },
    );
    target.components.insert(
        "engine.physics.rigid_body".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([("body_type".into(), Value::Enum("Static".into()))]),
        },
    );
    target.components.insert(
        "engine.physics.collider".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::new(),
        },
    );
    target.components.insert(
        "engine.interactable".into(),
        engine_scene::ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("prompt".into(), Value::Str("Pick up cube".into())),
                ("action".into(), Value::Str("pickup".into())),
                ("max_distance".into(), Value::Float32(5.0)),
                ("grabbable".into(), Value::Bool(true)),
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
                ("class_name".into(), Value::Str("Probe".into())),
            ]),
        },
    );
    game_loop.load_scene(scene).unwrap();
    game_loop
}

#[test]
fn physics_queries_report_persistent_ids_in_the_next_frame_snapshot() {
    use engine_script::GameplayPhysicsQueryResult;

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = physics_query_game_loop(&contexts);

    // Frame 1: the script issues its queries; no results yet.
    game_loop.update(1.0 / 60.0);
    let first = contexts.lock().unwrap().last().unwrap().clone();
    assert!(first.physics_query_results.is_empty());

    // Frame 2: results arrive keyed by the script-chosen query ids.
    game_loop.update(1.0 / 60.0);
    let second = contexts.lock().unwrap().last().unwrap().clone();
    assert_eq!(second.entity_id, "cube-01");
    assert_eq!(second.physics_query_results.len(), 3);

    let hit = second
        .physics_query_results
        .iter()
        .find_map(|result| match result {
            GameplayPhysicsQueryResult::RaycastHit {
                query_id: 11,
                entity_id,
                point,
                normal,
                distance,
                interaction,
            } => Some((
                entity_id.clone(),
                *point,
                *normal,
                *distance,
                interaction.clone(),
            )),
            _ => None,
        })
        .expect("raycast hit result for query 11");
    assert_eq!(hit.0, "cube-01");
    assert!((hit.1[1] - 0.5).abs() < 1.0e-4, "hit point: {:?}", hit.1);
    let interaction = hit.4.expect("enabled in-range interactable metadata");
    assert_eq!(interaction.prompt, "Pick up cube");
    assert_eq!(interaction.action, "pickup");
    assert!(interaction.grabbable);
    assert!(
        hit.1[0].abs() < 1.0e-4 && hit.1[2].abs() < 1.0e-4,
        "hit point: {:?}",
        hit.1
    );
    assert!(
        (hit.2[1] - 1.0).abs() < 1.0e-4 && hit.2[0].abs() < 1.0e-4 && hit.2[2].abs() < 1.0e-4,
        "hit normal: {:?}",
        hit.2
    );
    assert!((hit.3 - 4.5).abs() < 1.0e-4, "hit distance: {}", hit.3);

    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::RaycastMiss { query_id: 12 }
    )));
    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::OverlapSphere { query_id: 13, entity_ids }
            if entity_ids == &vec!["cube-01".to_string()]
    )));

    // Frame 3: results are frame-local and expire with the next snapshot.
    game_loop.update(1.0 / 60.0);
    let third = contexts.lock().unwrap().last().unwrap().clone();
    assert!(third.physics_query_results.is_empty());
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}

#[test]
fn invalid_physics_queries_report_script_diagnostics_and_never_execute() {
    use engine_script::GameplayPhysicsQueryResult;

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(ContextRecordingHost {
            contexts: Arc::clone(&contexts),
        }));
    game_loop.runtime.set_script_host_name("context-recording");
    game_loop
        .runtime
        .load_script_assembly("game", "context-recording", b"test")
        .unwrap();
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();

    // A typed host can bypass the JSON decoder, so the runtime
    // re-validates before staging anything for the physics world.
    let invalid = engine_script::OwnedGameplayCommand {
        entity_id: "cube-01".into(),
        command: GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id: 1,
                origin: [f32::NAN, 0.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
                filter: None,
            },
        },
    };
    let diagnostics = game_loop
        .runtime
        .apply_script_gameplay_commands(vec![invalid]);
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PHYSICS_QUERY_INVALID"));
    assert!(game_loop.runtime.take_pending_physics_queries().is_empty());

    // The invalid query never reaches a script snapshot.
    game_loop.update(1.0 / 60.0);
    assert!(contexts.lock().unwrap().iter().all(|context| {
        context.physics_query_results.iter().all(|result| {
            !matches!(
                result,
                GameplayPhysicsQueryResult::RaycastHit { query_id: 1, .. }
            )
        })
    }));
}

#[test]
fn physics_queries_with_unknown_exclude_entity_are_rejected() {
    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(ContextRecordingHost {
            contexts: Arc::clone(&contexts),
        }));
    game_loop.runtime.set_script_host_name("context-recording");
    game_loop
        .runtime
        .load_script_assembly("game", "context-recording", b"test")
        .unwrap();
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();

    let unknown_exclusion = |query_id| engine_script::OwnedGameplayCommand {
        entity_id: "cube-01".into(),
        command: GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id,
                origin: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
                filter: Some(engine_script::GameplayPhysicsQueryFilter {
                    layer_mask: None,
                    include_sensors: false,
                    exclude_entity: Some("ghost-entity".into()),
                }),
            },
        },
    };
    let diagnostics = game_loop
        .runtime
        .apply_script_gameplay_commands(vec![unknown_exclusion(1)]);
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "SCRIPT_PHYSICS_QUERY_INVALID"
                && diagnostic.message.contains("ghost-entity")
        }),
        "unknown exclude_entity id should be a validation error: {diagnostics:?}"
    );
    assert!(game_loop.runtime.take_pending_physics_queries().is_empty());

    // A known exclusion target passes validation and is queued.
    let self_exclusion = engine_script::OwnedGameplayCommand {
        entity_id: "cube-01".into(),
        command: GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id: 2,
                origin: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
                filter: Some(engine_script::GameplayPhysicsQueryFilter {
                    layer_mask: None,
                    include_sensors: false,
                    exclude_entity: Some("cube-01".into()),
                }),
            },
        },
    };
    let diagnostics = game_loop
        .runtime
        .apply_script_gameplay_commands(vec![self_exclusion]);
    assert!(
        diagnostics.is_empty(),
        "known exclude_entity id should validate: {diagnostics:?}"
    );
    assert_eq!(game_loop.runtime.take_pending_physics_queries().len(), 1);
}
