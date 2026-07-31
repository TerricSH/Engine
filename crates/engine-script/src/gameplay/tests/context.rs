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
        world_origin: [8000.0, 0.0, -4000.0],
        input_actions: BTreeMap::from([
            ("jump".into(), GameplayInputValue::Bool(true)),
            ("move".into(), GameplayInputValue::Vec2([0.25, -0.5])),
        ]),
        input_transitions: GameplayInputTransitions {
            pressed: BTreeSet::from(["jump".into()]),
            released: BTreeSet::new(),
        },
        pointer: GameplayPointerSnapshot {
            position: [640.0, 360.0],
            viewport: [1280.0, 720.0],
            focused: true,
            inside_viewport: true,
            ray_origin: Some([0.0, 10.0, 0.0]),
            ray_direction: Some([0.0, -1.0, 0.0]),
            ..GameplayPointerSnapshot::default()
        },
        camera: Some(GameplayCameraSnapshot {
            entity_id: Some("camera".into()),
            perspective: true,
            position: [0.0, 10.0, 0.0],
            forward: [0.0, -1.0, 0.0],
            viewport: [0.0, 0.0, 1280.0, 720.0],
            ..GameplayCameraSnapshot::default()
        }),
        save_events: vec![GameplaySaveEvent {
            slot: "autosave".into(),
            kind: GameplaySaveEventKind::Saved,
            state_json: None,
            error: None,
        }],
        logic_asset_results: vec![GameplayLogicAssetResult {
            query_id: 11,
            asset_id: "soldier-abilities".into(),
            json: Some("{\"nodes\":[]}".into()),
            error: None,
        }],
        physics_events: vec![GameplayPhysicsEvent {
            kind: GameplayPhysicsEventKind::CollisionEntered,
            other_entity_id: "floor".into(),
            joint_id: None,
            force: None,
            torque: None,
        }],
        damage_events: vec![GameplayDamageEvent {
            target_entity_id: "crate".into(),
            source_entity_id: Some("player".into()),
            damage_kind: GameplayDamageKind::Impact,
            raw_damage: 10.0,
            applied_damage: 10.0,
            remaining_health: 90.0,
            hit_position: Some([1.0, 2.0, 3.0]),
            impulse: [4.0, 0.0, 0.0],
            broke: false,
            spawned_entity_ids: Vec::new(),
        }],
        ragdoll_events: vec![GameplayRagdollEvent {
            entity_id: "npc-01".into(),
            active: true,
            recovering: false,
            body_entity_ids: vec!["npc-01.__ragdoll.body.0".into()],
        }],
        physics_query_results: vec![
            GameplayPhysicsQueryResult::RaycastHit {
                query_id: 7,
                entity_id: "floor".into(),
                point: [1.0, 0.5, 0.0],
                normal: [0.0, 1.0, 0.0],
                distance: 4.5,
                interaction: None,
            },
            GameplayPhysicsQueryResult::RaycastMiss { query_id: 8 },
            GameplayPhysicsQueryResult::OverlapSphere {
                query_id: 9,
                entity_ids: vec!["floor".into()],
            },
        ],
        component_query_results: vec![
            GameplayComponentQueryResult::Snapshot {
                query_id: 3,
                entity_id: "speaker-01".into(),
                component_type: "engine.audio_source".into(),
                fields: BTreeMap::from([
                    ("volume".into(), GameplayComponentValue::Float(0.75)),
                    ("playing".into(), GameplayComponentValue::Bool(true)),
                ]),
            },
            GameplayComponentQueryResult::Missing {
                query_id: 4,
                entity_id: "cube-01".into(),
                component_type: "engine.light".into(),
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
    assert!(json.contains(r#""world_origin":[8000.0,0.0,-4000.0]"#));
    assert!(json.contains(r#""jump":{"type":"Bool","value":true}"#));
    assert!(json.contains(r#""pressed":["jump"]"#));
    assert!(json.contains(r#""kind":"collision_entered""#));
    assert!(json.contains(
            r#""physics_query_results":[{"kind":"raycast_hit","query_id":7,"entity_id":"floor","point":[1.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.5},{"kind":"raycast_miss","query_id":8},{"kind":"overlap_sphere","query_id":9,"entity_ids":["floor"]}]"#
        ));
    assert!(json.contains(
            r#""component_query_results":[{"kind":"snapshot","query_id":3,"entity_id":"speaker-01","component_type":"engine.audio_source","fields":{"playing":{"type":"bool","value":true},"volume":{"type":"float","value":0.75}}},{"kind":"missing","query_id":4,"entity_id":"cube-01","component_type":"engine.light"}]"#
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
    // Older runtimes predate origin shifting: scripts see a zero origin.
    assert_eq!(context.world_origin, [0.0; 3]);
    assert!(context.physics_events.is_empty());
    assert!(context.damage_events.is_empty());
    assert!(context.ragdoll_events.is_empty());
    assert!(context.physics_query_results.is_empty());
    assert!(context.component_query_results.is_empty());
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
