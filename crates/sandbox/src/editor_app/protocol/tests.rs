    use super::*;

    fn request(method: &str, params: JsonValue) -> BridgeRequest {
        BridgeRequest {
            id: "request-1".into(),
            protocol: Some(EDITOR_PROTOCOL.into()),
            session_id: Some("session-1".into()),
            base_revision: Some(4),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn unknown_methods_are_rejected_instead_of_using_a_generic_escape_hatch() {
        let error = EditorRequest::decode(&request(
            "editor.execute",
            serde_json::json!({"command": "anything"}),
        ))
        .unwrap_err();
        assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
    }

    #[test]
    fn retired_command_aliases_cannot_reopen_a_second_protocol_path() {
        for method in [
            "editor.saveScene",
            "editor.undo",
            "editor.redo",
            "scene.reparent",
            "scene.duplicateSelection",
            "scene.deleteSelection",
        ] {
            let error = EditorRequest::decode(&request(method, serde_json::json!({})))
                .expect_err("retired command aliases must stay unavailable");
            assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
        }
    }

    #[test]
    fn retired_parameter_aliases_and_fake_gizmo_modes_are_rejected() {
        for (method, params) in [
            (
                "scene.select",
                serde_json::json!({"activeEntityId": "cube"}),
            ),
            (
                "scene.createEntity",
                serde_json::json!({"archetype": "cube"}),
            ),
            (
                "scene.setEntityParent",
                serde_json::json!({"entityId": "cube", "parentId": "root"}),
            ),
            (
                "scene.setComponentField",
                serde_json::json!({
                    "entityId": "cube",
                    "componentType": "engine.transform",
                    "fieldPath": "translation",
                    "value": {"Vec3": [1.0, 2.0, 3.0]}
                }),
            ),
            ("viewport.setTool", serde_json::json!({"tool": "move"})),
            ("viewport.setTool", serde_json::json!({"mode": "hand"})),
            ("viewport.setTool", serde_json::json!({"mode": "rect"})),
            ("viewport.setTool", serde_json::json!({"mode": "combined"})),
        ] {
            let error = EditorRequest::decode(&request(method, params))
                .expect_err("retired protocol shapes must stay unavailable");
            assert!(matches!(error.code, EditorErrorCode::InvalidRequest));
        }
    }

    #[test]
    fn component_field_request_decodes_a_typed_engine_value() {
        let decoded = EditorRequest::decode(&request(
            "scene.setComponentField",
            serde_json::json!({
                "entityId": "cube",
                "componentType": "engine.transform",
                "fieldName": "translation",
                "value": {"Vec3": [1.0, 2.0, 3.0]}
            }),
        ))
        .unwrap();
        let EditorRequest::SetComponentField(params) = decoded else {
            panic!("wrong request variant");
        };
        assert_eq!(params.entity_id, "cube");
        assert_eq!(params.value, Value::Vec3([1.0, 2.0, 3.0]));
    }

    #[test]
    fn viewport_input_is_explicit_and_uses_css_local_coordinates() {
        let decoded = EditorRequest::decode(&request(
            "viewport.input",
            serde_json::json!({
                "viewport": "scene",
                "event": {
                    "type": "pointerDown",
                    "pointerId": 7,
                    "x": 14.5,
                    "y": 20.0,
                    "button": 0,
                    "buttons": 1,
                    "modifiers": {"alt": false, "control": false, "meta": false, "shift": true}
                }
            }),
        ))
        .unwrap();
        assert!(matches!(decoded, EditorRequest::ViewportInput(_)));
    }

    #[test]
    fn viewport_camera_and_gizmo_visibility_have_typed_commands() {
        let decoded = EditorRequest::decode(&request(
            "viewport.setCamera",
            serde_json::json!({
                "pitch": 12.0,
                "yaw": 36.0,
                "distance": 8.0,
                "target": [1.0, 2.0, 3.0],
                "orthographic": true,
                "speed": 6.0
            }),
        ))
        .unwrap();
        let EditorRequest::SetCamera(camera) = decoded else {
            panic!("wrong camera request variant");
        };
        assert_eq!(camera.target, [1.0, 2.0, 3.0]);
        assert!(camera.orthographic);

        let decoded = EditorRequest::decode(&request(
            "viewport.setGizmos",
            serde_json::json!({"visible": false}),
        ))
        .unwrap();
        let EditorRequest::SetGizmos(gizmos) = decoded else {
            panic!("wrong gizmo request variant");
        };
        assert!(!gizmos.visible);
    }

    #[test]
    fn ui_open_panel_event_has_a_versioned_typed_wire_shape() {
        let event = BridgeEvent {
            protocol: EDITOR_PROTOCOL,
            session_id: "session-1".to_string(),
            sequence: 4,
            revision: 9,
            event: UI_OPEN_PANEL_EVENT,
            params: UiOpenPanelParams {
                panel: UiPanel::Material,
                preferred_zone: UiDockZone::Bottom,
            },
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "protocol": EDITOR_PROTOCOL,
                "sessionId": "session-1",
                "sequence": 4,
                "revision": 9,
                "event": "ui.openPanel",
                "params": {"panel": "material", "preferredZone": "bottom"}
            })
        );
    }

    #[test]
    fn terrain_debug_commands_preserve_full_u64_seed_text() {
        let decoded = EditorRequest::decode(&request(
            "terrain.replaySeed",
            serde_json::json!({"seed": "18446744073709551615"}),
        ))
        .unwrap();
        let EditorRequest::ReplayTerrainSeed(params) = decoded else {
            panic!("wrong terrain request variant");
        };
        assert_eq!(params.seed, "18446744073709551615");
        assert!(matches!(
            EditorRequest::decode(&request("terrain.regenerate", serde_json::json!({}))).unwrap(),
            EditorRequest::RegenerateTerrain
        ));
    }
