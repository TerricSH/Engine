use super::*;

#[test]
fn physics_query_command_has_a_stable_validated_contract() {
    let raycast = GameplayCommand::PhysicsQuery {
        query: GameplayPhysicsQuery::Raycast {
            query_id: 7,
            origin: [0.0, 5.0, 0.0],
            direction: [0.0, -1.0, 0.0],
            max_distance: 10.0,
            filter: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&raycast).unwrap(),
        r#"{"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0.0,5.0,0.0],"direction":[0.0,-1.0,0.0],"max_distance":10.0}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&raycast).unwrap()).unwrap(),
        raycast
    );
    assert!(raycast.validate().is_ok());

    let overlap = GameplayCommand::PhysicsQuery {
        query: GameplayPhysicsQuery::OverlapSphere {
            query_id: 8,
            center: [1.0, 2.0, 3.0],
            radius: 2.5,
            filter: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&overlap).unwrap(),
        r#"{"type":"physics_query","query":{"kind":"overlap_sphere","query_id":8,"center":[1.0,2.0,3.0],"radius":2.5}}"#
    );
    assert!(overlap.validate().is_ok());

    let sphere_cast = GameplayCommand::PhysicsQuery {
        query: GameplayPhysicsQuery::SphereCast {
            query_id: 9,
            origin: [0.0, 5.0, 0.0],
            radius: 0.5,
            direction: [0.0, -1.0, 0.0],
            max_distance: 10.0,
            filter: None,
        },
    };
    assert_eq!(
        serde_json::to_string(&sphere_cast).unwrap(),
        r#"{"type":"physics_query","query":{"kind":"sphere_cast","query_id":9,"origin":[0.0,5.0,0.0],"radius":0.5,"direction":[0.0,-1.0,0.0],"max_distance":10.0}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&sphere_cast).unwrap())
            .unwrap(),
        sphere_cast
    );
    assert!(sphere_cast.validate().is_ok());

    let GameplayCommand::PhysicsQuery { query } = &raycast else {
        panic!("expected physics query command");
    };
    assert_eq!(query.query_id(), 7);
    assert_eq!(query.filter(), None);
}

#[test]
fn physics_mutation_command_has_a_stable_bounded_contract() {
    let impulse = GameplayCommand::PhysicsMutation {
        mutation: GameplayPhysicsMutation::ApplyImpulse {
            entity_id: "crate-01".into(),
            impulse: [10.0, 2.0, -4.0],
        },
    };
    assert_eq!(
        serde_json::to_string(&impulse).unwrap(),
        r#"{"type":"physics_mutation","mutation":{"kind":"apply_impulse","entity_id":"crate-01","impulse":[10.0,2.0,-4.0]}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&impulse).unwrap()).unwrap(),
        impulse
    );
    assert!(impulse.validate().is_ok());

    for invalid in [
        GameplayPhysicsMutation::ApplyForce {
            entity_id: "../crate".into(),
            force: [1.0, 0.0, 0.0],
        },
        GameplayPhysicsMutation::ApplyForce {
            entity_id: "crate".into(),
            force: [f32::NAN, 0.0, 0.0],
        },
        GameplayPhysicsMutation::ApplyImpulse {
            entity_id: "crate".into(),
            impulse: [MAX_PHYSICS_MUTATION_COMPONENT + 1.0, 0.0, 0.0],
        },
    ] {
        assert!(invalid.validate().is_err());
    }

    let hinge = GameplayCommand::PhysicsMutation {
        mutation: GameplayPhysicsMutation::CreateJoint {
            joint_id: "door-hinge".into(),
            body_a: "door-frame".into(),
            body_b: "door".into(),
            joint_type: GameplayJointType::Revolute,
            anchor_a: [0.0, 1.0, 0.0],
            anchor_b: [-0.5, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            limits: Some(GameplayJointLimits {
                min: -1.5,
                max: 1.5,
                stiffness: 20.0,
                damping: 2.0,
            }),
            motor: Some(GameplayJointMotor {
                target_vel: 1.0,
                target_pos: 0.25,
                stiffness: 10.0,
                damping: 1.0,
            }),
            break_force: 5000.0,
            break_torque: 1000.0,
        },
    };
    assert!(hinge.validate().is_ok());
    let json = serde_json::to_string(&hinge).unwrap();
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&json).unwrap(),
        hinge
    );
    assert!(json.contains(r#""kind":"create_joint""#));
    assert!(json.contains(r#""joint_type":"revolute""#));

    let same_body = GameplayPhysicsMutation::CreateJoint {
        joint_id: "bad-joint".into(),
        body_a: "crate".into(),
        body_b: "crate".into(),
        joint_type: GameplayJointType::Fixed,
        anchor_a: [0.0; 3],
        anchor_b: [0.0; 3],
        axis: [1.0, 0.0, 0.0],
        limits: None,
        motor: None,
        break_force: 0.0,
        break_torque: 0.0,
    };
    assert!(same_body.validate().is_err());
}

#[test]
fn joint_break_event_extends_the_legacy_physics_event_contract() {
    let legacy: GameplayPhysicsEvent =
        serde_json::from_str(r#"{"kind":"collision_entered","other_entity_id":"floor"}"#).unwrap();
    assert_eq!(legacy.joint_id, None);
    assert_eq!(legacy.force, None);
    assert_eq!(legacy.torque, None);

    let broken = GameplayPhysicsEvent {
        kind: GameplayPhysicsEventKind::JointBroken,
        other_entity_id: "door".into(),
        joint_id: Some("door-hinge".into()),
        force: Some(4200.0),
        torque: Some(750.0),
    };
    assert_eq!(
        serde_json::from_str::<GameplayPhysicsEvent>(&serde_json::to_string(&broken).unwrap())
            .unwrap(),
        broken
    );
}
