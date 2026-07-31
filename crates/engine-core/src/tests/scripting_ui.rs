#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
#[test]
fn managed_ui_commands_create_and_mutate_retained_canvas_components() {
    let _guard = serial_ffi_world_test();
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_world(World::from_scene(&engine_scene::sample_scene()));
    let layout = engine_script::GameplayUiLayout {
        anchor_min: [0.0, 0.0],
        anchor_max: [0.0, 0.0],
        offset_min: [24.0, 24.0],
        offset_max: [344.0, 56.0],
    };
    let commands = vec![
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::CreateCanvas {
                    canvas_id: "hud".into(),
                    width: 1280.0,
                    height: 720.0,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetCanvasScaleMode {
                    canvas_id: "hud".into(),
                    scale_mode: engine_script::GameplayUiScaleMode::FitWidth,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 1,
                    element: engine_script::GameplayUiElement::Panel {
                        layout,
                        color: engine_script::GameplayUiColor {
                            r: 20,
                            g: 20,
                            b: 20,
                            a: 210,
                        },
                        z_order: 10,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 2,
                    element: engine_script::GameplayUiElement::Text {
                        layout,
                        text: "Score: 0".into(),
                        font_size: 24.0,
                        color: engine_script::GameplayUiColor {
                            r: 255,
                            g: 255,
                            b: 255,
                            a: 255,
                        },
                        z_order: 11,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 3,
                    element: engine_script::GameplayUiElement::Toggle {
                        layout,
                        label: "Music".into(),
                        is_on: false,
                        color_on: engine_script::GameplayUiColor {
                            r: 0,
                            g: 200,
                            b: 80,
                            a: 255,
                        },
                        color_off: engine_script::GameplayUiColor {
                            r: 80,
                            g: 80,
                            b: 80,
                            a: 255,
                        },
                        callback_id: Some("music".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 4,
                    element: engine_script::GameplayUiElement::Checkbox {
                        layout,
                        label: "Hints".into(),
                        checked: false,
                        color: engine_script::GameplayUiColor {
                            r: 200,
                            g: 200,
                            b: 200,
                            a: 255,
                        },
                        callback_id: Some("hints".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::AddElement {
                    canvas_id: "hud".into(),
                    element_id: 5,
                    element: engine_script::GameplayUiElement::Slider {
                        layout,
                        label: "Volume".into(),
                        value: 0.2,
                        min: 0.0,
                        max: 1.0,
                        callback_id: Some("volume".into()),
                        z_order: 12,
                    },
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetText {
                    canvas_id: "hud".into(),
                    element_id: 2,
                    text: "Score: 10".into(),
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetElementEnabled {
                    canvas_id: "hud".into(),
                    element_id: 1,
                    enabled: false,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetToggleValue {
                    canvas_id: "hud".into(),
                    element_id: 3,
                    is_on: true,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetCheckboxValue {
                    canvas_id: "hud".into(),
                    element_id: 4,
                    checked: true,
                },
            },
        },
        engine_script::OwnedGameplayCommand {
            entity_id: "cube-01".into(),
            command: GameplayCommand::Ui {
                command: engine_script::GameplayUiCommand::SetSliderValue {
                    canvas_id: "hud".into(),
                    element_id: 5,
                    value: 0.8,
                },
            },
        },
    ];

    let diagnostics = runtime.apply_script_gameplay_commands(commands);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    runtime
            .with_world(|world| {
                let hud = world.entity_by_persistent_id("hud").expect("HUD entity");
                let canvas = world
                    .get::<engine_ui::Canvas>(hud)
                    .expect("Canvas component");
                assert_eq!((canvas.width, canvas.height), (1280.0, 720.0));
                assert_eq!(canvas.scale_mode, engine_ui::ScaleMode::FitWidth);
                assert!(!canvas.get_element(engine_ui::ElementId(1)).unwrap().enabled);
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(2)).unwrap().kind,
                    engine_ui::UiElementKind::Text { content, .. } if content == "Score: 10"
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(3)).unwrap().kind,
                    engine_ui::UiElementKind::Toggle { is_on: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(4)).unwrap().kind,
                    engine_ui::UiElementKind::Checkbox { checked: true, .. }
                ));
                assert!(matches!(
                    &canvas.get_element(engine_ui::ElementId(5)).unwrap().kind,
                    engine_ui::UiElementKind::Slider { value, .. } if (*value - 0.8).abs() < f32::EPSILON
                ));
            })
            .expect("runtime must keep an active World");
}
