use super::*;

#[test]
fn browser_key_codes_map_to_the_existing_platform_contract() {
    assert_eq!(web_key_code("KeyW"), Some(platform::KeyCode::W));
    assert_eq!(web_key_code("ArrowLeft"), Some(platform::KeyCode::Left));
    assert_eq!(web_key_code("BrowserBack"), None);
}

#[test]
fn viewport_traffic_does_not_require_scene_revision_round_trips() {
    assert!(!request_requires_revision(
        &EditorRequest::SetViewportBounds(ViewportBoundsParams {
            viewport: "scene".into(),
            rect: ScreenRect::default(),
            visible: true,
        })
    ));
    assert!(request_requires_revision(&EditorRequest::Undo));
}

#[test]
fn entity_id_params_deserialize_the_canonical_batch_ids() {
    let params: EntityIdParams = serde_json::from_value(json!({
        "entityIds": ["root", "child"]
    }))
    .unwrap();

    assert!(params.entity_id.is_empty());
    assert_eq!(params.entity_ids, ["root", "child"]);

    for method in ["scene.duplicateEntity", "scene.deleteEntity"] {
        let request = BridgeRequest {
            id: "batch-entity-request".into(),
            protocol: Some(EDITOR_PROTOCOL.into()),
            session_id: Some("session".into()),
            base_revision: Some(0),
            method: method.into(),
            params: json!({ "entityIds": ["root", "child"] }),
        };
        let decoded = EditorRequest::decode(&request).unwrap();
        let decoded_params = match decoded {
            EditorRequest::DuplicateEntity(params) | EditorRequest::DeleteEntity(params) => params,
            _ => panic!("{method} decoded to the wrong request variant"),
        };
        assert!(decoded_params.entity_id.is_empty());
        assert_eq!(decoded_params.entity_ids, ["root", "child"]);
    }
}

#[test]
fn css_viewport_coordinates_scale_to_physical_pixels_on_hidpi_surfaces() {
    assert_eq!(
        css_pointer_to_physical(120.0, 80.0, 1.5),
        Vec2::new(180.0, 120.0)
    );
    assert_eq!(
        css_pointer_to_physical(120.0, 80.0, 2.0),
        Vec2::new(240.0, 160.0)
    );
}

#[test]
fn scene_camera_command_rejects_non_finite_state() {
    let valid = CameraParams {
        pitch: 20.0,
        yaw: 45.0,
        distance: 10.0,
        target: [0.0, 1.0, 2.0],
        orthographic: false,
        speed: 5.0,
    };
    assert!(camera_params_are_finite(&valid));
    assert!(!camera_params_are_finite(&CameraParams {
        target: [f32::NAN, 0.0, 0.0],
        ..valid
    }));
}

#[test]
fn persisted_react_layout_requires_the_canonical_dock_shape() {
    assert!(validate_react_layout(DEFAULT_REACT_LAYOUT).is_ok());
    assert!(validate_react_layout(r#"{"zones":{}}"#).is_err());
    assert!(validate_react_layout("not json").is_err());
}
