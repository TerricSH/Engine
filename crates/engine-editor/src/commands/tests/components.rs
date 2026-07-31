use super::*;

#[test]
fn add_component_rejects_duplicates_without_overwriting_or_history() {
    let mut scene = engine_scene::sample_scene();
    let entity = scene
        .entities
        .iter()
        .find(|entity| !entity.components.is_empty())
        .unwrap();
    let entity_id = entity.persistent_id.clone();
    let (component_type, original) = entity.components.iter().next().unwrap();
    let component_type = component_type.clone();
    let original = original.clone();
    let mut replacement = original.clone();
    replacement.enabled = !replacement.enabled;
    let mut history = CommandHistory::new();

    let result = history.push(
        Box::new(AddComponent::new(
            entity_id.clone(),
            component_type.clone(),
            replacement,
        )),
        &mut scene,
    );
    assert!(matches!(
        result,
        Err(EditorError::ComponentAlreadyExists(type_id)) if type_id == component_type
    ));
    assert_eq!(
        scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == entity_id)
            .unwrap()
            .components[&component_type],
        original
    );
    assert!(!history.can_undo());
}

#[test]
fn camera_component_commands_keep_active_camera_valid_through_undo_redo() {
    let mut scene = engine_scene::sample_scene();
    let entity_id = scene.entities[0].persistent_id.clone();
    scene.entities[0].components.remove("engine.camera");
    scene.scene_settings.active_camera = None;
    let camera = ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: std::collections::BTreeMap::new(),
    };
    let mut history = CommandHistory::new();

    history
        .push(
            Box::new(AddComponent::new(
                entity_id.clone(),
                "engine.camera".into(),
                camera,
            )),
            &mut scene,
        )
        .unwrap();
    assert_eq!(
        scene.scene_settings.active_camera.as_deref(),
        Some(entity_id.as_str())
    );

    history.undo(&mut scene).unwrap();
    assert!(scene.scene_settings.active_camera.is_none());
    assert!(!scene.entities[0].components.contains_key("engine.camera"));

    history.redo(&mut scene).unwrap();
    assert_eq!(
        scene.scene_settings.active_camera.as_deref(),
        Some(entity_id.as_str())
    );
    history
        .push(
            Box::new(RemoveComponent::new(
                entity_id.clone(),
                "engine.camera".into(),
            )),
            &mut scene,
        )
        .unwrap();
    assert!(scene.scene_settings.active_camera.is_none());

    history.undo(&mut scene).unwrap();
    assert_eq!(
        scene.scene_settings.active_camera.as_deref(),
        Some(entity_id.as_str())
    );
    assert!(scene.entities[0].components.contains_key("engine.camera"));
}

#[test]
fn parent_command_is_undoable_and_rejects_cycles() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();
    history
        .push(
            Box::new(SetEntityParent::new(
                "cube-01".to_string(),
                Some("camera-main".to_string()),
            )),
            &mut scene,
        )
        .unwrap();
    assert_eq!(
        scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "cube-01")
            .unwrap()
            .parent
            .as_deref(),
        Some("camera-main")
    );
    assert!(history
        .push(
            Box::new(SetEntityParent::new(
                "camera-main".to_string(),
                Some("cube-01".to_string()),
            )),
            &mut scene,
        )
        .is_err());
    history.undo(&mut scene).unwrap();
    assert!(scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .unwrap()
        .parent
        .is_none());
}

fn component_record(schema: (u16, u16, u16), enabled: bool, label: &str) -> ComponentRecord {
    ComponentRecord {
        schema_version: engine_serialize::SchemaVersion::new(schema.0, schema.1, schema.2),
        enabled,
        fields: BTreeMap::from([
            ("label".into(), Value::Str(label.into())),
            (
                "nested".into(),
                Value::Map(BTreeMap::from([(
                    "items".into(),
                    Value::List(vec![Value::Bool(enabled), Value::Float32(3.5)]),
                )])),
            ),
        ]),
    }
}

fn component_clipboard_scene() -> Scene {
    let mut source = entity("source", None);
    source.components.insert(
        "test.clipboard_values".into(),
        component_record((2, 3, 4), false, "copied"),
    );
    let mut target = entity("target", None);
    target.components.insert(
        "test.clipboard_values".into(),
        component_record((0, 1, 0), true, "original"),
    );
    target.components.insert(
        "test.other_component".into(),
        component_record((0, 2, 0), true, "camera"),
    );
    let mut scene = engine_scene::sample_scene();
    scene.scene_settings.active_camera = None;
    scene.entities = vec![source, target];
    scene
}

fn component<'a>(scene: &'a Scene, entity_id: &str, component_type: &str) -> &'a ComponentRecord {
    &scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == entity_id)
        .unwrap()
        .components[component_type]
}

fn component_at_mut<'a>(
    scene: &'a mut Scene,
    entity_id: &str,
    component_type: &str,
) -> &'a mut ComponentRecord {
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == entity_id)
        .unwrap()
        .components
        .get_mut(component_type)
        .unwrap()
}

#[test]
fn component_clipboard_ron_round_trip_preserves_the_complete_record() {
    let scene = component_clipboard_scene();
    let clipboard =
        ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
            .unwrap();
    assert_eq!(clipboard.type_id(), "test.clipboard_values");
    assert_eq!(
        clipboard.component(),
        component(&scene, "source", "test.clipboard_values")
    );

    let serialized = clipboard.to_ron().unwrap();
    let decoded = ComponentClipboard::from_ron(&serialized).unwrap();
    assert_eq!(decoded, clipboard);
    assert_eq!(
        decoded.component().schema_version,
        engine_serialize::SchemaVersion::new(2, 3, 4)
    );
    assert!(!decoded.component().enabled);
    assert_eq!(
        decoded.component().fields,
        component(&scene, "source", "test.clipboard_values").fields
    );
}

#[test]
fn malformed_component_clipboard_ron_is_rejected() {
    let scene = component_clipboard_scene();
    let clipboard =
        ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
            .unwrap();
    let serialized = clipboard.to_ron().unwrap();

    let unsupported = serialized.replacen("format_version: 1", "format_version: 99", 1);
    assert!(matches!(
        ComponentClipboard::from_ron(&unsupported),
        Err(EditorError::InvalidComponentClipboard(_))
    ));
    let empty_type = serialized.replacen("\"test.clipboard_values\"", "\"\"", 1);
    assert!(matches!(
        ComponentClipboard::from_ron(&empty_type),
        Err(EditorError::InvalidComponentClipboard(_))
    ));
    let unknown_field = serialized.replacen('(', "(unknown: true,", 1);
    assert!(matches!(
        ComponentClipboard::from_ron(&unknown_field),
        Err(EditorError::ComponentClipboardSerialization(_))
    ));
    assert!(matches!(
        ComponentClipboard::from_ron("this is not RON"),
        Err(EditorError::ComponentClipboardSerialization(_))
    ));
}

#[test]
fn component_values_paste_across_entities_has_exact_undo_and_redo() {
    let mut scene = component_clipboard_scene();
    let original = component(&scene, "target", "test.clipboard_values").clone();
    let copied = component(&scene, "source", "test.clipboard_values").clone();
    let original_component_count = scene.entities[1].components.len();
    let clipboard =
        ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
            .unwrap();
    let command = ReplaceComponent::prepare(
        &scene,
        "target".into(),
        "test.clipboard_values".into(),
        &clipboard,
    )
    .unwrap();
    assert_eq!(command.replacement(), &copied);

    let mut history = CommandHistory::new();
    history.push(Box::new(command), &mut scene).unwrap();
    assert_eq!(
        component(&scene, "target", "test.clipboard_values"),
        &copied
    );
    assert_eq!(scene.entities[1].components.len(), original_component_count);
    assert!(scene.entities[1]
        .components
        .contains_key("test.clipboard_values"));
    assert_eq!(
        component(&scene, "source", "test.clipboard_values"),
        &copied
    );

    history.undo(&mut scene).unwrap();
    assert_eq!(
        component(&scene, "target", "test.clipboard_values"),
        &original
    );
    history.redo(&mut scene).unwrap();
    assert_eq!(
        component(&scene, "target", "test.clipboard_values"),
        &copied
    );
}

#[test]
fn component_clipboard_cannot_replace_a_different_type() {
    let scene = component_clipboard_scene();
    let clipboard =
        ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
            .unwrap();
    assert!(matches!(
        ReplaceComponent::prepare(
            &scene,
            "target".into(),
            "test.other_component".into(),
            &clipboard,
        ),
        Err(EditorError::InvalidComponentClipboard(_))
    ));
}

#[test]
fn deferred_component_reset_is_an_undoable_same_key_replacement() {
    let mut scene = component_clipboard_scene();
    let original = component(&scene, "target", "test.clipboard_values").clone();
    let reset = component_record((5, 0, 1), true, "reset");
    let mut history = CommandHistory::new();
    history
        .push(
            Box::new(ReplaceComponent::new(
                "target".into(),
                "test.clipboard_values".into(),
                reset.clone(),
            )),
            &mut scene,
        )
        .unwrap();
    assert_eq!(component(&scene, "target", "test.clipboard_values"), &reset);
    history.undo(&mut scene).unwrap();
    assert_eq!(
        component(&scene, "target", "test.clipboard_values"),
        &original
    );
    history.redo(&mut scene).unwrap();
    assert_eq!(component(&scene, "target", "test.clipboard_values"), &reset);
}

#[test]
fn stale_component_replace_and_undo_fail_without_partial_mutation() {
    let mut scene = component_clipboard_scene();
    let clipboard =
        ComponentClipboard::capture(&scene, &"source".into(), &"test.clipboard_values".into())
            .unwrap();
    let command = ReplaceComponent::prepare(
        &scene,
        "target".into(),
        "test.clipboard_values".into(),
        &clipboard,
    )
    .unwrap();
    component_at_mut(&mut scene, "target", "test.clipboard_values")
        .fields
        .insert("external-edit".into(), Value::Bool(true));
    let before_failed_execute = scene.clone();
    let mut history = CommandHistory::new();
    assert!(history.push(Box::new(command), &mut scene).is_err());
    assert_eq!(scene, before_failed_execute);
    assert!(!history.can_undo());

    let command = ReplaceComponent::prepare(
        &scene,
        "target".into(),
        "test.clipboard_values".into(),
        &clipboard,
    )
    .unwrap();
    history.push(Box::new(command), &mut scene).unwrap();
    component_at_mut(&mut scene, "target", "test.clipboard_values")
        .fields
        .insert("post-paste-edit".into(), Value::Bool(true));
    let before_failed_undo = scene.clone();
    assert!(history.undo(&mut scene).is_err());
    assert_eq!(scene, before_failed_undo);
    assert!(history.can_undo());
}
