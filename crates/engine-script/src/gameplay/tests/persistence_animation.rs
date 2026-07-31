use super::*;

#[test]
fn checkpoint_and_logic_asset_commands_are_bounded_and_path_safe() {
    let save = GameplayCommand::SaveCheckpoint {
        slot: "ironman-01".into(),
        state_json: r#"{"round":4,"selected":"unit-2"}"#.into(),
    };
    assert!(save.validate().is_ok());
    assert_eq!(
        serde_json::to_string(&save).unwrap(),
        r#"{"type":"save_checkpoint","slot":"ironman-01","state_json":"{\"round\":4,\"selected\":\"unit-2\"}"}"#
    );
    assert!(GameplayCommand::LoadCheckpoint {
        slot: "../outside".into()
    }
    .validate()
    .is_err());
    assert!(GameplayCommand::SaveCheckpoint {
        slot: "slot".into(),
        state_json: "{broken".into(),
    }
    .validate()
    .is_err());

    let query = GameplayCommand::QueryLogicAsset {
        query_id: 9,
        asset_id: "soldier-abilities".into(),
    };
    assert!(query.validate().is_ok());
    assert_eq!(
        serde_json::to_string(&query).unwrap(),
        r#"{"type":"query_logic_asset","query_id":9,"asset_id":"soldier-abilities"}"#
    );
}

#[test]
fn animation_commands_have_a_typed_bounded_contract() {
    let play = GameplayCommand::PlayAnimation {
        entity_id: "hero".into(),
        clip_asset: "battle.attack".into(),
        looping: false,
        speed: 1.25,
        restart: true,
    };
    assert!(play.validate().is_ok());
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&play).unwrap()).unwrap(),
        play
    );
    assert!(GameplayCommand::PlayAnimation {
        entity_id: "hero".into(),
        clip_asset: "../attack".into(),
        looping: false,
        speed: 1.0,
        restart: true,
    }
    .validate()
    .is_err());
    assert!(GameplayCommand::PlayAnimation {
        entity_id: "hero".into(),
        clip_asset: "attack".into(),
        looping: false,
        speed: f32::NAN,
        restart: true,
    }
    .validate()
    .is_err());

    let parameter = GameplayCommand::SetAnimationParameter {
        entity_id: "hero".into(),
        name: "battle.phase".into(),
        value: GameplayAnimationParameterValue::Int(2),
    };
    assert!(parameter.validate().is_ok());
    assert_eq!(
        serde_json::to_string(&parameter).unwrap(),
        r#"{"type":"set_animation_parameter","entity_id":"hero","name":"battle.phase","value":{"type":"int","value":2}}"#
    );
    assert!(GameplayCommand::SetAnimationParameter {
        entity_id: "hero".into(),
        name: "speed".into(),
        value: GameplayAnimationParameterValue::Float(f32::INFINITY),
    }
    .validate()
    .is_err());

    let morph = GameplayCommand::SetMorphWeights {
        entity_id: "hero".into(),
        weights: vec![0.0, 0.5, 1.0],
    };
    assert!(morph.validate().is_ok());
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&morph).unwrap()).unwrap(),
        morph
    );
    assert!(GameplayCommand::SetMorphWeights {
        entity_id: "hero".into(),
        weights: vec![2.0],
    }
    .validate()
    .is_err());
}
