use super::*;

#[test]
fn apply_damage_command_has_a_stable_bounded_contract() {
    let command = GameplayCommand::ApplyDamage {
        entity_id: "crate-01".into(),
        amount: 25.0,
        damage_kind: GameplayDamageKind::Blast,
        hit_position: Some([1.0, 2.0, 3.0]),
        impulse: [4.0, 0.0, -2.0],
    };
    assert_eq!(
        serde_json::to_string(&command).unwrap(),
        r#"{"type":"apply_damage","entity_id":"crate-01","amount":25.0,"damage_kind":"blast","hit_position":[1.0,2.0,3.0],"impulse":[4.0,0.0,-2.0]}"#
    );
    assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"apply_damage","entity_id":"crate-01","amount":25,"damage_kind":"blast","hit_position":[1,2,3],"impulse":[4,0,-2]}"#
            )
            .unwrap(),
            command
        );
    assert!(command.validate().is_ok());

    for invalid in [
        GameplayCommand::ApplyDamage {
            entity_id: "../crate".into(),
            amount: 1.0,
            damage_kind: GameplayDamageKind::Generic,
            hit_position: None,
            impulse: [0.0; 3],
        },
        GameplayCommand::ApplyDamage {
            entity_id: "crate-01".into(),
            amount: 0.0,
            damage_kind: GameplayDamageKind::Generic,
            hit_position: None,
            impulse: [0.0; 3],
        },
        GameplayCommand::ApplyDamage {
            entity_id: "crate-01".into(),
            amount: MAX_DAMAGE_AMOUNT + 1.0,
            damage_kind: GameplayDamageKind::Generic,
            hit_position: None,
            impulse: [0.0; 3],
        },
        GameplayCommand::ApplyDamage {
            entity_id: "crate-01".into(),
            amount: 1.0,
            damage_kind: GameplayDamageKind::Generic,
            hit_position: Some([f32::NAN, 0.0, 0.0]),
            impulse: [0.0; 3],
        },
        GameplayCommand::ApplyDamage {
            entity_id: "crate-01".into(),
            amount: 1.0,
            damage_kind: GameplayDamageKind::Generic,
            hit_position: None,
            impulse: [MAX_PHYSICS_MUTATION_COMPONENT + 1.0, 0.0, 0.0],
        },
    ] {
        assert!(invalid.validate().is_err(), "{invalid:?}");
    }
}

#[test]
fn set_ragdoll_command_has_a_stable_bounded_contract() {
    let command = GameplayCommand::SetRagdoll {
        entity_id: "npc-01".into(),
        active: true,
        recovery_duration: 0.35,
        impulse: [10.0, 2.0, 0.0],
    };
    assert_eq!(
        serde_json::to_string(&command).unwrap(),
        r#"{"type":"set_ragdoll","entity_id":"npc-01","active":true,"recovery_duration":0.35,"impulse":[10.0,2.0,0.0]}"#
    );
    assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"set_ragdoll","entity_id":"npc-01","active":true,"recovery_duration":0.35,"impulse":[10,2,0]}"#
            )
            .unwrap(),
            command
        );
    assert!(command.validate().is_ok());

    for invalid in [
        GameplayCommand::SetRagdoll {
            entity_id: "../npc".into(),
            active: true,
            recovery_duration: 0.0,
            impulse: [0.0; 3],
        },
        GameplayCommand::SetRagdoll {
            entity_id: "npc-01".into(),
            active: false,
            recovery_duration: MAX_RAGDOLL_RECOVERY_SECONDS + 1.0,
            impulse: [0.0; 3],
        },
        GameplayCommand::SetRagdoll {
            entity_id: "npc-01".into(),
            active: true,
            recovery_duration: 0.0,
            impulse: [f32::NAN, 0.0, 0.0],
        },
    ] {
        assert!(invalid.validate().is_err(), "{invalid:?}");
    }
}

#[test]
fn character_control_has_a_stable_bounded_contract() {
    let command = GameplayCommand::CharacterControl {
        entity_id: "npc-01".into(),
        direction: [0.6, 0.0, -0.8],
        jump: true,
        speed: Some(7.5),
    };
    assert_eq!(
        serde_json::to_string(&command).unwrap(),
        r#"{"type":"character_control","entity_id":"npc-01","direction":[0.6,0.0,-0.8],"jump":true,"speed":7.5}"#
    );
    assert_eq!(
            serde_json::from_str::<GameplayCommand>(
                r#"{"type":"character_control","entity_id":"npc-01","direction":[0.6,0,-0.8],"jump":true,"speed":7.5}"#
            )
            .unwrap(),
            command
        );
    assert!(command.validate().is_ok());

    for invalid in [
        GameplayCommand::CharacterControl {
            entity_id: "../npc".into(),
            direction: [0.0; 3],
            jump: false,
            speed: None,
        },
        GameplayCommand::CharacterControl {
            entity_id: "npc-01".into(),
            direction: [1.0, 0.0, 1.0],
            jump: false,
            speed: None,
        },
        GameplayCommand::CharacterControl {
            entity_id: "npc-01".into(),
            direction: [0.0, 0.1, 0.0],
            jump: false,
            speed: None,
        },
        GameplayCommand::CharacterControl {
            entity_id: "npc-01".into(),
            direction: [0.0; 3],
            jump: false,
            speed: Some(MAX_CHARACTER_CONTROL_SPEED + 1.0),
        },
    ] {
        assert!(invalid.validate().is_err(), "{invalid:?}");
    }
}
