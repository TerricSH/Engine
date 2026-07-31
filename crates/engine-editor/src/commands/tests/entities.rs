use super::*;

#[test]
fn clipboard_round_trip_captures_each_selected_subtree_once() {
    let scene = hierarchy_scene();
    let clipboard = EntityClipboard::capture(
        &scene,
        &["grandchild".into(), "root".into(), "child".into()],
    )
    .unwrap();
    assert_eq!(clipboard.root_ids(), &["root"]);
    assert_eq!(
        clipboard
            .entities()
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<Vec<_>>(),
        ["root", "child", "grandchild"]
    );

    let serialized = clipboard.to_ron().unwrap();
    assert_eq!(EntityClipboard::from_ron(&serialized).unwrap(), clipboard);
    assert!(EntityClipboard::from_ron("(format_version: 99, root_ids: [], entities: [])").is_err());
}

#[test]
fn duplicate_subtree_remaps_hierarchy_and_nested_entity_references() {
    let mut scene = hierarchy_scene();
    let original = scene.clone();
    let command = DuplicateEntitySubtree::prepare(&scene, &"root".into()).unwrap();
    assert_eq!(command.duplicated_root_id(), "root-copy");
    assert_eq!(
        command
            .duplicated_records()
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<Vec<_>>(),
        ["root-copy", "child-copy-2", "grandchild-copy"]
    );
    let mut history = CommandHistory::new();
    history.push(Box::new(command), &mut scene).unwrap();
    let duplicated = scene.clone();

    let root = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "root-copy")
        .unwrap();
    assert_eq!(root.parent.as_deref(), Some("external"));
    assert_eq!(
        scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "child-copy-2")
            .unwrap()
            .parent
            .as_deref(),
        Some("root-copy")
    );
    assert_eq!(
        scene
            .entities
            .iter()
            .find(|entity| entity.persistent_id == "grandchild-copy")
            .unwrap()
            .parent
            .as_deref(),
        Some("child-copy-2")
    );
    let fields = &root.components["test.references"].fields;
    assert_eq!(fields["internal"], Value::Entity("child-copy-2".into()));
    assert_eq!(
        fields["nested"],
        Value::Map(BTreeMap::from([(
            "target".into(),
            Value::List(vec![Value::Entity("grandchild-copy".into())])
        )]))
    );
    assert_eq!(fields["external"], Value::Entity("external".into()));

    history.undo(&mut scene).unwrap();
    assert_eq!(scene, original);
    history.redo(&mut scene).unwrap();
    assert_eq!(scene, duplicated);
}

#[test]
fn paste_supports_explicit_parent_and_is_undoable() {
    let mut scene = hierarchy_scene();
    let clipboard = EntityClipboard::capture(&scene, &["root".into()]).unwrap();
    let command =
        PasteEntityRecords::prepare(&scene, &clipboard, EntityPasteParent::SceneRoot).unwrap();
    let pasted_root = command.pasted_root_ids()[0].clone();
    let original = scene.clone();
    let mut history = CommandHistory::new();
    history.push(Box::new(command), &mut scene).unwrap();
    let pasted = scene.clone();
    assert!(scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == pasted_root)
        .unwrap()
        .parent
        .is_none());

    history.undo(&mut scene).unwrap();
    assert_eq!(scene, original);
    history.redo(&mut scene).unwrap();
    assert_eq!(scene, pasted);
}

#[test]
fn stale_paste_plan_fails_without_scene_or_history_mutation() {
    let mut scene = hierarchy_scene();
    let clipboard = EntityClipboard::capture(&scene, &["root".into()]).unwrap();
    let command =
        PasteEntityRecords::prepare(&scene, &clipboard, EntityPasteParent::SceneRoot).unwrap();
    let conflict = command.pasted_records()[0].clone();
    scene.entities.push(conflict);
    let before = scene.clone();
    let mut history = CommandHistory::new();
    assert!(history.push(Box::new(command), &mut scene).is_err());
    assert_eq!(scene, before);
    assert!(!history.can_undo());
}

fn sibling_scene() -> Scene {
    let mut scene = engine_scene::sample_scene();
    scene.scene_settings.active_camera = None;
    scene.entities = vec![
        entity("a", None),
        entity("a-child", Some("a")),
        entity("b", None),
        entity("b-child", Some("b")),
        entity("c", None),
    ];
    scene
}

#[test]
fn every_sibling_move_has_exact_undo_and_redo() {
    for (entity_id, movement, expected) in [
        ("b", SiblingMove::Up, vec!["b", "a", "c"]),
        ("b", SiblingMove::Down, vec!["a", "c", "b"]),
        ("c", SiblingMove::First, vec!["c", "a", "b"]),
        ("a", SiblingMove::Last, vec!["b", "c", "a"]),
    ] {
        let mut scene = sibling_scene();
        let original = scene.clone();
        let untouched_children = [scene.entities[1].clone(), scene.entities[3].clone()];
        let mut history = CommandHistory::new();
        history
            .push(
                Box::new(MoveEntitySibling::new(entity_id.into(), movement)),
                &mut scene,
            )
            .unwrap();
        assert_eq!(
            sibling_ids(&scene, None),
            expected.into_iter().map(str::to_owned).collect::<Vec<_>>()
        );
        assert_eq!(scene.entities[1], untouched_children[0]);
        assert_eq!(scene.entities[3], untouched_children[1]);
        let moved = scene.clone();

        history.undo(&mut scene).unwrap();
        assert_eq!(scene, original);
        history.redo(&mut scene).unwrap();
        assert_eq!(scene, moved);
    }
}

#[test]
fn sibling_boundary_and_stale_undo_fail_atomically() {
    let mut scene = sibling_scene();
    let original = scene.clone();
    let mut history = CommandHistory::new();
    assert!(history
        .push(
            Box::new(MoveEntitySibling::new("a".into(), SiblingMove::Up)),
            &mut scene,
        )
        .is_err());
    assert_eq!(scene, original);
    assert!(!history.can_undo());

    history
        .push(
            Box::new(MoveEntitySibling::new("b".into(), SiblingMove::Up)),
            &mut scene,
        )
        .unwrap();
    scene.entities.swap(0, 2);
    let stale = scene.clone();
    assert!(history.undo(&mut scene).is_err());
    assert_eq!(scene, stale);
    assert!(history.can_undo());
}

#[test]
fn add_and_recursive_remove_preserve_order_and_camera_through_history() {
    let mut scene = hierarchy_scene();
    scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "child")
        .unwrap()
        .components
        .insert(
            "engine.camera".into(),
            ComponentRecord {
                schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::new(),
            },
        );
    scene.scene_settings.active_camera = Some("child".into());
    let original = scene.clone();
    let mut history = CommandHistory::new();
    history
        .push(Box::new(RemoveEntity::new("root".into())), &mut scene)
        .unwrap();
    assert!(scene.scene_settings.active_camera.is_none());
    assert!(!scene.entities.iter().any(|entity| matches!(
        entity.persistent_id.as_str(),
        "root" | "child" | "grandchild"
    )));
    history.undo(&mut scene).unwrap();
    assert_eq!(scene, original);
    history.redo(&mut scene).unwrap();
    history.undo(&mut scene).unwrap();
    assert_eq!(scene, original);

    let before_duplicate = scene.clone();
    assert!(history
        .push(
            Box::new(AddEntity::new(entity("external", None))),
            &mut scene
        )
        .is_err());
    assert_eq!(scene, before_duplicate);
}

#[test]
fn scene_settings_are_undoable_and_reject_stale_undo() {
    let mut scene = engine_scene::sample_scene();
    let original = scene.scene_settings.clone();
    let mut replacement = original.clone();
    replacement.fixed_timestep_seconds = 1.0 / 120.0;
    replacement.default_render_layer = "Gameplay".into();
    let mut history = CommandHistory::new();
    history
        .push(
            Box::new(SetSceneSettings::prepare(&scene, replacement.clone())),
            &mut scene,
        )
        .unwrap();
    assert_eq!(scene.scene_settings, replacement);
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.scene_settings, original);
    history.redo(&mut scene).unwrap();
    assert_eq!(scene.scene_settings, replacement);

    scene.scene_settings.ambient[0] = 0.5;
    let stale = scene.clone();
    assert!(history.undo(&mut scene).is_err());
    assert_eq!(scene, stale);
    assert!(history.can_undo());
}

#[test]
fn invalid_scene_settings_never_enter_history() {
    let mut scene = engine_scene::sample_scene();
    let original = scene.clone();
    let mut invalid = scene.scene_settings.clone();
    invalid.fixed_timestep_seconds = f32::NAN;
    let mut history = CommandHistory::new();
    assert!(history
        .push(
            Box::new(SetSceneSettings::prepare(&scene, invalid)),
            &mut scene,
        )
        .is_err());
    assert_eq!(scene, original);
    assert!(!history.can_undo());
}
