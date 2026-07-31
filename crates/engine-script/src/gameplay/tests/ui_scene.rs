use super::*;

#[test]
fn load_scene_command_has_a_stable_json_contract() {
    let command = GameplayCommand::LoadScene {
        scene_id: "level_two".into(),
    };
    assert_eq!(
        serde_json::to_string(&command).unwrap(),
        r#"{"type":"load_scene","scene_id":"level_two"}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(r#"{"type":"load_scene","scene_id":"level_two"}"#)
            .unwrap(),
        command
    );
    assert!(command.validate().is_ok());
}

#[test]
fn managed_ui_commands_have_a_stable_validated_contract() {
    let create = GameplayCommand::Ui {
        command: GameplayUiCommand::CreateCanvas {
            canvas_id: "hud".into(),
            width: 1280.0,
            height: 720.0,
        },
    };
    assert_eq!(
        serde_json::to_string(&create).unwrap(),
        r#"{"type":"ui","command":{"type":"create_canvas","canvas_id":"hud","width":1280.0,"height":720.0}}"#
    );
    assert!(create.validate().is_ok());

    let add = GameplayCommand::Ui {
        command: GameplayUiCommand::AddElement {
            canvas_id: "hud".into(),
            element_id: 1,
            element: GameplayUiElement::Panel {
                layout: GameplayUiLayout {
                    anchor_min: [0.0, 0.0],
                    anchor_max: [0.0, 0.0],
                    offset_min: [24.0, 24.0],
                    offset_max: [344.0, 56.0],
                },
                color: GameplayUiColor {
                    r: 20,
                    g: 20,
                    b: 20,
                    a: 210,
                },
                z_order: 10,
            },
        },
    };
    let json = serde_json::to_string(&add).unwrap();
    assert!(json.contains(r#""kind":"panel""#));
    assert!(json.contains(r#""element_id":1"#));
    assert_eq!(serde_json::from_str::<GameplayCommand>(&json).unwrap(), add);
    assert!(add.validate().is_ok());

    let set_slider = GameplayCommand::Ui {
        command: GameplayUiCommand::SetSliderValue {
            canvas_id: "hud".into(),
            element_id: 3,
            value: 0.75,
        },
    };
    assert_eq!(
        serde_json::to_string(&set_slider).unwrap(),
        r#"{"type":"ui","command":{"type":"set_slider_value","canvas_id":"hud","element_id":3,"value":0.75}}"#
    );
    assert!(set_slider.validate().is_ok());
}

#[test]
fn managed_ui_commands_reject_invalid_ids_and_geometry() {
    for command in [
        GameplayUiCommand::CreateCanvas {
            canvas_id: "../hud".into(),
            width: 1280.0,
            height: 720.0,
        },
        GameplayUiCommand::CreateCanvas {
            canvas_id: "hud".into(),
            width: f32::NAN,
            height: 720.0,
        },
        GameplayUiCommand::RemoveElement {
            canvas_id: "hud".into(),
            element_id: 0,
        },
        GameplayUiCommand::SetSliderValue {
            canvas_id: "hud".into(),
            element_id: 1,
            value: f32::INFINITY,
        },
    ] {
        assert!(command.validate().is_err());
    }
}

#[test]
fn scene_id_validation_matches_the_project_catalog_contract() {
    for valid in ["main", "level-two", "level_two", "chapter.2", "A1"] {
        assert!(validate_scene_id(valid).is_ok(), "{valid}");
    }
    for invalid in ["", ".", "..", "level/two", "level two", "关卡"] {
        let error = validate_scene_id(invalid).unwrap_err();
        assert!(error.contains("game.project.json `scenes`"), "{error}");
    }
    assert!(validate_scene_id(&"a".repeat(129)).is_err());
}
