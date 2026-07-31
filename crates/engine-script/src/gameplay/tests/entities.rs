use super::*;

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

    let scale = GameplayCommand::Ui {
        command: GameplayUiCommand::SetCanvasScaleMode {
            canvas_id: "hud".into(),
            scale_mode: GameplayUiScaleMode::FitWidth,
        },
    };
    assert_eq!(
        serde_json::to_string(&scale).unwrap(),
        r#"{"type":"ui","command":{"type":"set_canvas_scale_mode","canvas_id":"hud","scale_mode":"fit_width"}}"#
    );
    assert!(scale.validate().is_ok());
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
fn spawn_prefab_command_has_a_stable_validated_contract() {
    let bare = GameplayCommand::SpawnPrefab {
        prefab_id: "prefab-enemy".into(),
        translation: None,
    };
    assert_eq!(
        serde_json::to_string(&bare).unwrap(),
        r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy"}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(
            r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy"}"#
        )
        .unwrap(),
        bare
    );
    assert!(bare.validate().is_ok());

    let placed = GameplayCommand::SpawnPrefab {
        prefab_id: "prefab-enemy".into(),
        translation: Some([1.0, 2.0, 3.0]),
    };
    assert_eq!(
        serde_json::to_string(&placed).unwrap(),
        r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy","translation":[1.0,2.0,3.0]}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(
            r#"{"type":"spawn_prefab","prefab_id":"prefab-enemy","translation":[1,2,3]}"#
        )
        .unwrap(),
        placed
    );
    assert!(placed.validate().is_ok());

    for command in [
        GameplayCommand::SpawnPrefab {
            prefab_id: "../enemy".into(),
            translation: None,
        },
        GameplayCommand::SpawnPrefab {
            prefab_id: "prefabs/enemy".into(),
            translation: None,
        },
        GameplayCommand::SpawnPrefab {
            prefab_id: String::new(),
            translation: None,
        },
        GameplayCommand::SpawnPrefab {
            prefab_id: "prefab-enemy".into(),
            translation: Some([f32::NAN, 0.0, 0.0]),
        },
    ] {
        assert!(command.validate().is_err(), "{command:?}");
    }
    assert!(validate_prefab_id("prefab-enemy").is_ok());
    assert!(validate_prefab_id("prefab.enemy_01").is_ok());
    let error = validate_prefab_id("prefabs/enemy").unwrap_err();
    assert!(error.contains("not file paths"), "{error}");
}
