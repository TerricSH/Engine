/// A static box entity at `translation` with a collider on
/// `collision_group`, optionally a sensor.
fn physics_box_entity(
    persistent_id: &str,
    translation: [f32; 3],
    collision_group: u32,
    is_trigger: bool,
) -> engine_scene::EntityRecord {
    engine_scene::EntityRecord {
        persistent_id: persistent_id.into(),
        parent: None,
        name: None,
        enabled: true,
        components: BTreeMap::from([
            (
                "engine.transform".into(),
                component_record(BTreeMap::from([(
                    "translation".into(),
                    Value::Vec3(translation),
                )])),
            ),
            (
                "engine.physics.rigid_body".into(),
                component_record(BTreeMap::from([(
                    "body_type".into(),
                    Value::Enum("Static".into()),
                )])),
            ),
            (
                "engine.physics.collider".into(),
                component_record(BTreeMap::from([
                    (
                        "collision_group".into(),
                        Value::UInt(u64::from(collision_group)),
                    ),
                    ("is_trigger".into(), Value::Bool(is_trigger)),
                ])),
            ),
        ]),
    }
}

/// A game loop whose script entity owns a layer-1 collider, alongside a
/// layer-2 box (`cube-02` at y = -4) and a sensor (`sensor-01` at
/// y = -8). `commands` are issued by the script on the first frame.
fn filtered_query_game_loop(
    contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>,
    commands: Vec<GameplayCommand>,
) -> GameLoop {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .register_script_host(Box::new(ScriptedQueriesHost {
            contexts: Arc::clone(contexts),
            commands,
        }));
    game_loop.runtime.set_script_host_name("scripted-queries");
    game_loop
        .runtime
        .load_script_assembly("game", "scripted-queries", b"test")
        .unwrap();

    let mut scene = engine_scene::sample_scene();
    let target = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap();
    target
        .components
        .insert("engine.transform".into(), component_record(BTreeMap::new()));
    target.components.insert(
        "engine.physics.rigid_body".into(),
        component_record(BTreeMap::from([(
            "body_type".into(),
            Value::Enum("Static".into()),
        )])),
    );
    target.components.insert(
        "engine.physics.collider".into(),
        component_record(BTreeMap::from([("collision_group".into(), Value::UInt(1))])),
    );
    target.components.insert(
        "engine.script".into(),
        component_record(BTreeMap::from([
            ("assembly_id".into(), Value::Str("game".into())),
            ("class_name".into(), Value::Str("Probe".into())),
        ])),
    );
    scene
        .entities
        .push(physics_box_entity("cube-02", [0.0, -4.0, 0.0], 2, false));
    scene.entities.push(physics_box_entity(
        "sensor-01",
        [0.0, -8.0, 0.0],
        0xFFFF_FFFF,
        true,
    ));
    game_loop.load_scene(scene).unwrap();
    game_loop
}

/// Run two frames and return the contexts from each.
fn two_frames(
    game_loop: &mut GameLoop,
    contexts: &Arc<std::sync::Mutex<Vec<GameplayContext>>>,
) -> (GameplayContext, GameplayContext) {
    game_loop.update(1.0 / 60.0);
    let first = contexts.lock().unwrap().last().unwrap().clone();
    game_loop.update(1.0 / 60.0);
    let second = contexts.lock().unwrap().last().unwrap().clone();
    (first, second)
}

#[test]
fn sphere_cast_reports_hit_miss_and_normal_in_the_next_frame() {
    use engine_script::GameplayPhysicsQueryResult;

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sphere_cast = |query_id, direction| GameplayCommand::PhysicsQuery {
        query: engine_script::GameplayPhysicsQuery::SphereCast {
            query_id,
            origin: [0.0, 5.0, 0.0],
            radius: 0.5,
            direction,
            max_distance: 10.0,
            filter: None,
        },
    };
    let mut game_loop = filtered_query_game_loop(
        &contexts,
        vec![
            sphere_cast(21, [0.0, -1.0, 0.0]),
            sphere_cast(22, [0.0, 1.0, 0.0]),
        ],
    );

    let (first, second) = two_frames(&mut game_loop, &contexts);
    assert!(first.physics_query_results.is_empty());
    assert_eq!(second.physics_query_results.len(), 2);

    let hit = second
        .physics_query_results
        .iter()
        .find_map(|result| match result {
            GameplayPhysicsQueryResult::SphereCastHit {
                query_id: 21,
                entity_id,
                point,
                normal,
                distance,
                ..
            } => Some((entity_id.clone(), *point, *normal, *distance)),
            _ => None,
        })
        .expect("sphere cast hit result for query 21");
    assert_eq!(hit.0, "cube-01");
    // The sphere surface touches the cube's top face once its centre
    // reaches y = 1.0: 4.0 units of travel from y = 5.
    assert!((hit.3 - 4.0).abs() < 1.0e-4, "hit distance: {}", hit.3);
    assert!(
        (hit.1[1] - 0.5).abs() < 5.0e-3,
        "contact point should sit on the top face (GJK/EPA tolerance): {:?}",
        hit.1
    );
    assert!(
        (hit.2[1] - 1.0).abs() < 1.0e-4 && hit.2[0].abs() < 1.0e-4 && hit.2[2].abs() < 1.0e-4,
        "hit normal: {:?}",
        hit.2
    );

    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::SphereCastMiss { query_id: 22 }
    )));
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}

#[test]
fn physics_queries_respect_layer_masks() {
    use engine_script::GameplayPhysicsQueryResult;

    let layer_filter = |mask| {
        Some(engine_script::GameplayPhysicsQueryFilter {
            layer_mask: Some(mask),
            include_sensors: false,
            exclude_entity: None,
        })
    };
    let raycast = |query_id, mask| GameplayCommand::PhysicsQuery {
        query: engine_script::GameplayPhysicsQuery::Raycast {
            query_id,
            origin: [0.0, 5.0, 0.0],
            direction: [0.0, -1.0, 0.0],
            max_distance: 20.0,
            filter: layer_filter(mask),
        },
    };
    let overlap = |query_id, mask| GameplayCommand::PhysicsQuery {
        query: engine_script::GameplayPhysicsQuery::OverlapSphere {
            query_id,
            center: [0.0, -4.0, 0.0],
            radius: 1.0,
            filter: layer_filter(mask),
        },
    };

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = filtered_query_game_loop(
        &contexts,
        vec![
            raycast(31, 1),
            raycast(32, 2),
            overlap(33, 1),
            overlap(34, 2),
        ],
    );
    let (_, second) = two_frames(&mut game_loop, &contexts);

    let hit_entity = |query_id| {
        second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::RaycastHit {
                    query_id: id,
                    entity_id,
                    ..
                } if *id == query_id => Some(entity_id.clone()),
                _ => None,
            })
    };
    // cube-01 sits on layer bit 1, cube-02 on layer bit 2.
    assert_eq!(hit_entity(31).as_deref(), Some("cube-01"));
    assert_eq!(hit_entity(32).as_deref(), Some("cube-02"));

    let overlap_ids = |query_id| {
        second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::OverlapSphere {
                    query_id: id,
                    entity_ids,
                } if *id == query_id => Some(entity_ids.clone()),
                _ => None,
            })
            .expect("overlap result")
    };
    assert!(overlap_ids(33).is_empty());
    assert_eq!(overlap_ids(34), vec!["cube-02".to_string()]);
}

#[test]
fn physics_queries_exclude_sensors_unless_opted_in() {
    use engine_script::GameplayPhysicsQueryResult;

    let overlap = |query_id, include_sensors| GameplayCommand::PhysicsQuery {
        query: engine_script::GameplayPhysicsQuery::OverlapSphere {
            query_id,
            center: [0.0, -8.0, 0.0],
            radius: 1.0,
            filter: Some(engine_script::GameplayPhysicsQueryFilter {
                layer_mask: None,
                include_sensors,
                exclude_entity: None,
            }),
        },
    };
    let raycast = |query_id, include_sensors| GameplayCommand::PhysicsQuery {
        query: engine_script::GameplayPhysicsQuery::Raycast {
            query_id,
            origin: [0.0, -6.0, 0.0],
            direction: [0.0, -1.0, 0.0],
            max_distance: 4.0,
            filter: Some(engine_script::GameplayPhysicsQueryFilter {
                layer_mask: None,
                include_sensors,
                exclude_entity: None,
            }),
        },
    };

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = filtered_query_game_loop(
        &contexts,
        vec![
            overlap(41, false),
            overlap(42, true),
            raycast(43, false),
            raycast(44, true),
        ],
    );
    let (_, second) = two_frames(&mut game_loop, &contexts);

    let overlap_ids = |query_id| {
        second
            .physics_query_results
            .iter()
            .find_map(|result| match result {
                GameplayPhysicsQueryResult::OverlapSphere {
                    query_id: id,
                    entity_ids,
                } if *id == query_id => Some(entity_ids.clone()),
                _ => None,
            })
            .expect("overlap result")
    };
    assert!(
        overlap_ids(41).is_empty(),
        "sensors stay invisible by default"
    );
    assert_eq!(overlap_ids(42), vec!["sensor-01".to_string()]);

    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::RaycastMiss { query_id: 43 }
    )));
    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::RaycastHit { query_id: 44, entity_id, .. }
            if entity_id == "sensor-01"
    )));
}

#[test]
fn physics_queries_respect_exclude_entity() {
    use engine_script::GameplayPhysicsQueryResult;

    let self_filter = || {
        Some(engine_script::GameplayPhysicsQueryFilter {
            layer_mask: None,
            include_sensors: false,
            exclude_entity: Some("cube-01".into()),
        })
    };
    let commands = vec![
        GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::Raycast {
                query_id: 51,
                origin: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                max_distance: 20.0,
                filter: self_filter(),
            },
        },
        GameplayCommand::PhysicsQuery {
            query: engine_script::GameplayPhysicsQuery::OverlapSphere {
                query_id: 52,
                center: [0.0, 0.0, 0.0],
                radius: 6.0,
                filter: self_filter(),
            },
        },
    ];

    let contexts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = filtered_query_game_loop(&contexts, commands);
    let (_, second) = two_frames(&mut game_loop, &contexts);

    // With cube-01 excluded, the ray passes through to cube-02 (top face
    // at y = -3.5, 8.5 units below the origin).
    let hit = second
        .physics_query_results
        .iter()
        .find_map(|result| match result {
            GameplayPhysicsQueryResult::RaycastHit {
                query_id: 51,
                entity_id,
                distance,
                ..
            } => Some((entity_id.clone(), *distance)),
            _ => None,
        })
        .expect("raycast hit result for query 51");
    assert_eq!(hit.0, "cube-02");
    assert!((hit.1 - 8.5).abs() < 1.0e-4, "hit distance: {}", hit.1);

    assert!(second.physics_query_results.iter().any(|result| matches!(
        result,
        GameplayPhysicsQueryResult::OverlapSphere { query_id: 52, entity_ids }
            if entity_ids == &vec!["cube-02".to_string()]
    )));
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .script_diagnostics
        .is_empty());
}
