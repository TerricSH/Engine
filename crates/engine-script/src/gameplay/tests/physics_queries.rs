use super::*;

#[test]
fn physics_query_filters_have_a_stable_validated_contract() {
    let filter = GameplayPhysicsQueryFilter {
        layer_mask: Some(0b0100),
        include_sensors: true,
        exclude_entity: Some("player-01".into()),
    };
    assert_eq!(
        serde_json::to_string(&filter).unwrap(),
        r#"{"layer_mask":4,"include_sensors":true,"exclude_entity":"player-01"}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQueryFilter>(
            r#"{"layer_mask":4,"include_sensors":true,"exclude_entity":"player-01"}"#
        )
        .unwrap(),
        filter
    );
    // Older hosts send no filter at all; a partial filter defaults the
    // remaining fields.
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQueryFilter>(r#"{"layer_mask":4}"#).unwrap(),
        GameplayPhysicsQueryFilter {
            layer_mask: Some(4),
            include_sensors: false,
            exclude_entity: None,
        }
    );

    let filtered_raycast = GameplayPhysicsQuery::Raycast {
        query_id: 3,
        origin: [0.0; 3],
        direction: [0.0, -1.0, 0.0],
        max_distance: 10.0,
        filter: Some(filter.clone()),
    };
    assert_eq!(
        serde_json::to_string(&filtered_raycast).unwrap(),
        r#"{"kind":"raycast","query_id":3,"origin":[0.0,0.0,0.0],"direction":[0.0,-1.0,0.0],"max_distance":10.0,"filter":{"layer_mask":4,"include_sensors":true,"exclude_entity":"player-01"}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQuery>(
            &serde_json::to_string(&filtered_raycast).unwrap()
        )
        .unwrap(),
        filtered_raycast
    );
    assert_eq!(filtered_raycast.filter(), Some(&filter));
    assert!(filtered_raycast.validate().is_ok());
    // A filter on a pre-0.6.0 query simply deserialises to `None`.
    assert_eq!(
            serde_json::from_str::<GameplayPhysicsQuery>(
                r#"{"kind":"raycast","query_id":3,"origin":[0,0,0],"direction":[0,-1,0],"max_distance":10}"#
            )
            .unwrap()
            .filter(),
            None
        );
}

#[test]
fn untrusted_physics_query_filters_reject_degenerate_values() {
    let zero_mask = GameplayPhysicsQuery::OverlapSphere {
        query_id: 1,
        center: [0.0; 3],
        radius: 1.0,
        filter: Some(GameplayPhysicsQueryFilter {
            layer_mask: Some(0),
            include_sensors: false,
            exclude_entity: None,
        }),
    };
    assert!(zero_mask.validate().unwrap_err().contains("non-zero"));

    let bad_exclude = GameplayPhysicsQuery::SphereCast {
        query_id: 1,
        origin: [0.0; 3],
        radius: 0.5,
        direction: [0.0, -1.0, 0.0],
        max_distance: 10.0,
        filter: Some(GameplayPhysicsQueryFilter {
            layer_mask: None,
            include_sensors: false,
            exclude_entity: Some("not/an id".into()),
        }),
    };
    assert!(bad_exclude
        .validate()
        .unwrap_err()
        .contains("invalid entity id"));
}

#[test]
fn physics_query_results_have_a_stable_json_contract() {
    let hit = GameplayPhysicsQueryResult::RaycastHit {
        query_id: 3,
        entity_id: "cube-01".into(),
        point: [0.0, 0.5, 0.0],
        normal: [0.0, 1.0, 0.0],
        distance: 4.5,
        interaction: None,
    };
    assert_eq!(
        serde_json::to_string(&hit).unwrap(),
        r#"{"kind":"raycast_hit","query_id":3,"entity_id":"cube-01","point":[0.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.5}"#
    );
    assert_eq!(
            serde_json::from_str::<GameplayPhysicsQueryResult>(
                r#"{"kind":"raycast_hit","query_id":3,"entity_id":"cube-01","point":[0,0.5,0],"normal":[0,1,0],"distance":4.5}"#
            )
            .unwrap(),
            hit
        );

    let miss = GameplayPhysicsQueryResult::RaycastMiss { query_id: 4 };
    assert_eq!(
        serde_json::to_string(&miss).unwrap(),
        r#"{"kind":"raycast_miss","query_id":4}"#
    );

    let sphere_hit = GameplayPhysicsQueryResult::SphereCastHit {
        query_id: 6,
        entity_id: "cube-01".into(),
        point: [0.0, 0.5, 0.0],
        normal: [0.0, 1.0, 0.0],
        distance: 4.0,
        interaction: Some(GameplayInteractionSnapshot {
            prompt: "Pick up".into(),
            action: "pickup".into(),
            max_distance: 3.0,
            grabbable: true,
        }),
    };
    assert_eq!(
        serde_json::to_string(&sphere_hit).unwrap(),
        r#"{"kind":"sphere_cast_hit","query_id":6,"entity_id":"cube-01","point":[0.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.0,"interaction":{"prompt":"Pick up","action":"pickup","max_distance":3.0,"grabbable":true}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQueryResult>(
            &serde_json::to_string(&sphere_hit).unwrap()
        )
        .unwrap(),
        sphere_hit
    );

    let sphere_miss = GameplayPhysicsQueryResult::SphereCastMiss { query_id: 7 };
    assert_eq!(
        serde_json::to_string(&sphere_miss).unwrap(),
        r#"{"kind":"sphere_cast_miss","query_id":7}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQueryResult>(
            r#"{"kind":"sphere_cast_miss","query_id":7}"#
        )
        .unwrap(),
        sphere_miss
    );

    let overlap = GameplayPhysicsQueryResult::OverlapSphere {
        query_id: 5,
        entity_ids: vec!["cube-01".into(), "physics-peer".into()],
    };
    assert_eq!(
        serde_json::to_string(&overlap).unwrap(),
        r#"{"kind":"overlap_sphere","query_id":5,"entity_ids":["cube-01","physics-peer"]}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsQueryResult>(
            &serde_json::to_string(&overlap).unwrap()
        )
        .unwrap(),
        overlap
    );
}

#[test]
fn untrusted_physics_queries_reject_non_finite_and_degenerate_values() {
    let nan_origin = GameplayPhysicsQuery::Raycast {
        query_id: 1,
        origin: [f32::NAN, 0.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        max_distance: 10.0,
        filter: None,
    };
    assert!(nan_origin.validate().unwrap_err().contains("finite"));

    let infinite_direction = GameplayPhysicsQuery::Raycast {
        query_id: 1,
        origin: [0.0; 3],
        direction: [f32::INFINITY, 0.0, 0.0],
        max_distance: 10.0,
        filter: None,
    };
    assert!(infinite_direction
        .validate()
        .unwrap_err()
        .contains("finite"));

    let zero_direction = GameplayPhysicsQuery::Raycast {
        query_id: 1,
        origin: [0.0; 3],
        direction: [0.0; 3],
        max_distance: 10.0,
        filter: None,
    };
    assert!(zero_direction
        .validate()
        .unwrap_err()
        .contains("zero length"));

    let zero_cast_direction = GameplayPhysicsQuery::SphereCast {
        query_id: 1,
        origin: [0.0; 3],
        radius: 0.5,
        direction: [0.0; 3],
        max_distance: 10.0,
        filter: None,
    };
    assert!(zero_cast_direction
        .validate()
        .unwrap_err()
        .contains("zero length"));

    for invalid_command in [
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::Raycast {
                query_id: 1,
                origin: [0.0; 3],
                direction: [0.0, -1.0, 0.0],
                max_distance: 0.0,
                filter: None,
            },
        },
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::Raycast {
                query_id: 1,
                origin: [0.0; 3],
                direction: [0.0, -1.0, 0.0],
                max_distance: f32::INFINITY,
                filter: None,
            },
        },
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::SphereCast {
                query_id: 1,
                origin: [0.0; 3],
                radius: 0.0,
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
                filter: None,
            },
        },
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::SphereCast {
                query_id: 1,
                origin: [0.0, f32::NAN, 0.0],
                radius: 0.5,
                direction: [0.0, -1.0, 0.0],
                max_distance: 10.0,
                filter: None,
            },
        },
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::OverlapSphere {
                query_id: 2,
                center: [0.0, f32::NAN, 0.0],
                radius: 1.0,
                filter: None,
            },
        },
        GameplayCommand::PhysicsQuery {
            query: GameplayPhysicsQuery::OverlapSphere {
                query_id: 2,
                center: [0.0; 3],
                radius: -1.0,
                filter: None,
            },
        },
    ] {
        assert!(invalid_command.validate().is_err());
    }
}
