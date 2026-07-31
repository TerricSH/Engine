// ── Sweep / filter fixtures ─────────────────────────────────────────

/// Script instance that issues a fixed, pre-built command batch once.
struct ScriptedQueriesInstance {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    commands: Vec<GameplayCommand>,
}

impl ScriptInstance for ScriptedQueriesInstance {
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

    fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
        Ok(std::mem::take(&mut self.commands))
    }
}

struct ScriptedQueriesHost {
    contexts: Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    commands: Vec<GameplayCommand>,
}

impl ScriptHost for ScriptedQueriesHost {
    fn name(&self) -> &str {
        "scripted-queries"
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
        Ok(Box::new(ScriptedQueriesInstance {
            contexts: Arc::clone(&self.contexts),
            commands: self.commands.clone(),
        }))
    }

    fn unload(&mut self, _handle: &ScriptHandle) -> Result<(), ScriptError> {
        Ok(())
    }
}

fn component_record(fields: BTreeMap<String, Value>) -> engine_scene::ComponentRecord {
    engine_scene::ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn register_damage_test_prefab(game_loop: &mut GameLoop, prefab_id: &str) {
    let mut prefab = engine_scene::Prefab::new(engine_serialize::AssetId::new(prefab_id));
    prefab.add_entity(engine_scene::EntityRecord {
        persistent_id: "root".into(),
        parent: None,
        name: Some("Fracture piece".into()),
        enabled: true,
        components: BTreeMap::from([
            ("engine.transform".into(), component_record(BTreeMap::new())),
            (
                "engine.physics.rigid_body".into(),
                component_record(BTreeMap::new()),
            ),
            (
                "engine.physics.collider".into(),
                component_record(BTreeMap::new()),
            ),
        ]),
    });
    let asset_id = engine_serialize::AssetId::new(prefab_id);
    game_loop
        .runtime
        .asset_registry_mut()
        .insert_typed(asset_id.clone(), prefab);
    game_loop
        .runtime
        .loaded_extension_asset_ids
        .entry("prefab".into())
        .or_default()
        .insert(asset_id);
}

fn damage_command(
    owner: &str,
    target: &str,
    amount: f32,
    impulse: [f32; 3],
) -> engine_script::OwnedGameplayCommand {
    engine_script::OwnedGameplayCommand {
        entity_id: owner.into(),
        command: GameplayCommand::ApplyDamage {
            entity_id: target.into(),
            amount,
            damage_kind: engine_script::GameplayDamageKind::Impact,
            hit_position: Some([3.0, 4.0, 5.0]),
            impulse,
        },
    }
}

#[test]
fn script_damage_breaks_once_replaces_prefab_and_inherits_physics_state() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    let target = game_loop
        .runtime
        .with_world_mut(|world| {
            let target = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(
                target,
                engine_scene::components::Transform {
                    translation: Vec3::new(3.0, 4.0, 5.0),
                    ..Default::default()
                },
            );
            world.add_component(target, engine_physics::RigidBody::default());
            world.add_component(target, engine_physics::Collider::default());
            world.add_component(
                target,
                engine_physics::Destructible {
                    max_health: 10.0,
                    health: 10.0,
                    replacement_prefab: Some(engine_serialize::AssetId::new("crate-fracture")),
                    fracture_impulse_scale: 0.5,
                    ..Default::default()
                },
            );
            target
        })
        .unwrap();
    game_loop.resync_physics_from_world();
    let source_state = engine_physics::RigidBodyRuntimeState {
        position: [3.0, 4.0, 5.0],
        rotation: glam::Quat::IDENTITY.to_array(),
        linear_velocity: [2.0, 3.0, 4.0],
        angular_velocity: [0.0, 1.5, 0.0],
        sleeping: false,
    };
    assert!(game_loop
        .physics
        .as_mut()
        .unwrap()
        .restore_runtime_body_state(target, &source_state));
    register_damage_test_prefab(&mut game_loop, "crate-fracture");

    let diagnostics = game_loop
        .runtime
        .apply_script_gameplay_commands(vec![damage_command(
            "camera-main",
            "cube-01",
            10.0,
            [6.0, 0.0, 0.0],
        )]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    game_loop.process_script_damage_requests();

    let fragment = game_loop
        .runtime
        .with_world(|world| {
            assert!(world.entity_by_persistent_id("cube-01").is_none());
            let fragment = world
                .entity_by_persistent_id("crate-fracture")
                .expect("replacement prefab root");
            assert_eq!(
                world
                    .get::<engine_scene::components::Transform>(fragment)
                    .unwrap()
                    .translation,
                Vec3::new(3.0, 4.0, 5.0)
            );
            fragment
        })
        .unwrap();
    let delivered = &game_loop.runtime.scripting.damage_events["camera-main"];
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].broke);
    assert_eq!(
        delivered[0].spawned_entity_ids,
        vec!["crate-fracture".to_string()]
    );

    {
        let physics = game_loop.physics.as_mut().unwrap();
        game_loop
            .runtime
            .with_world_mut(|world| physics.step(0.0, world))
            .unwrap();
    }
    let fragment_state = game_loop
        .physics
        .as_ref()
        .unwrap()
        .runtime_body_states()
        .into_iter()
        .find_map(|(entity, state)| (entity == fragment).then_some(state))
        .expect("fracture piece body state");
    assert!(fragment_state.linear_velocity[0] > source_state.linear_velocity[0]);
    assert_eq!(
        fragment_state.linear_velocity[1],
        source_state.linear_velocity[1]
    );
    assert_eq!(
        fragment_state.linear_velocity[2],
        source_state.linear_velocity[2]
    );
    assert_eq!(
        fragment_state.angular_velocity,
        source_state.angular_velocity
    );
}

#[test]
fn failed_fracture_prefab_does_not_delete_the_broken_source() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            let target = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(
                target,
                engine_physics::Destructible {
                    max_health: 1.0,
                    health: 1.0,
                    replacement_prefab: Some(engine_serialize::AssetId::new("missing-fracture")),
                    ..Default::default()
                },
            );
        })
        .unwrap();

    assert!(game_loop
        .runtime
        .apply_script_gameplay_commands(vec![damage_command(
            "camera-main",
            "cube-01",
            1.0,
            [0.0; 3],
        )])
        .is_empty());
    game_loop.process_script_damage_requests();

    game_loop
        .runtime
        .with_world(|world| {
            let target = world
                .entity_by_persistent_id("cube-01")
                .expect("failed replacement keeps source entity");
            let destructible = world.get::<engine_physics::Destructible>(target).unwrap();
            assert!(destructible.broken);
            assert_eq!(destructible.health, 0.0);
        })
        .unwrap();
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "SCRIPT_PREFAB_UNKNOWN"));
}
