use super::*;

#[test]
fn externally_restored_document_can_be_marked_dirty_without_fake_command() {
    let mut history = CommandHistory::new();
    history.mark_dirty();
    assert!(history.is_dirty());
    assert!(!history.can_undo());
}

#[test]
fn undoing_the_first_push_returns_to_the_initial_clean_state() {
    let mut scene = hierarchy_scene();
    let original = scene.clone();
    let mut history = CommandHistory::new();

    history
        .push(
            Box::new(SetEntityName::new(
                "root".into(),
                Some("renamed root".into()),
            )),
            &mut scene,
        )
        .unwrap();
    assert!(history.is_dirty());

    history.undo(&mut scene).unwrap();

    assert_eq!(scene, original);
    assert!(!history.is_dirty());
    assert!(!history.can_undo());
    assert!(history.can_redo());
}

#[test]
fn save_checkpoint_tracks_exact_state_across_undo_and_redo() {
    let mut scene = hierarchy_scene();
    let mut history = CommandHistory::new();

    history
        .push(
            Box::new(SetEntityName::new("root".into(), Some("saved name".into()))),
            &mut scene,
        )
        .unwrap();
    history.mark_clean();
    let saved_scene = scene.clone();
    assert!(!history.is_dirty());

    history
        .push(
            Box::new(SetEntityName::new(
                "root".into(),
                Some("unsaved name".into()),
            )),
            &mut scene,
        )
        .unwrap();
    assert!(history.is_dirty());

    history.undo(&mut scene).unwrap();
    assert_eq!(scene, saved_scene);
    assert!(!history.is_dirty());

    history.redo(&mut scene).unwrap();
    assert!(history.is_dirty());

    history.undo(&mut scene).unwrap();
    assert_eq!(scene, saved_scene);
    assert!(!history.is_dirty());

    history.undo(&mut scene).unwrap();
    assert!(history.is_dirty());

    history.redo(&mut scene).unwrap();
    assert_eq!(scene, saved_scene);
    assert!(!history.is_dirty());
}

#[test]
fn branching_after_undo_cannot_reuse_a_discarded_clean_state() {
    let mut scene = hierarchy_scene();
    let mut history = CommandHistory::new();

    history
        .push(
            Box::new(SetEntityName::new("root".into(), Some("first".into()))),
            &mut scene,
        )
        .unwrap();
    history
        .push(
            Box::new(SetEntityName::new(
                "root".into(),
                Some("discarded clean branch".into()),
            )),
            &mut scene,
        )
        .unwrap();
    history.mark_clean();
    assert!(!history.is_dirty());

    history.undo(&mut scene).unwrap();
    assert!(history.is_dirty());
    assert!(history.can_redo());

    history
        .push(
            Box::new(SetEntityName::new(
                "root".into(),
                Some("replacement branch".into()),
            )),
            &mut scene,
        )
        .unwrap();

    assert!(history.is_dirty());
    assert!(!history.can_redo());
}

#[test]
fn failed_undo_keeps_command_on_done_stack() {
    let mut history = CommandHistory::new();
    let mut scene = engine_scene::sample_scene();
    history.push(Box::new(UndoFails), &mut scene).unwrap();
    history.mark_clean();

    assert!(history.undo(&mut scene).is_err());
    assert!(history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.is_dirty());
}

#[test]
fn failed_redo_keeps_command_on_undone_stack() {
    let mut history = CommandHistory::new();
    let mut scene = engine_scene::sample_scene();
    history
        .push(Box::new(RedoFails { executions: 0 }), &mut scene)
        .unwrap();
    history.undo(&mut scene).unwrap();
    history.mark_clean();

    assert!(history.redo(&mut scene).is_err());
    assert!(!history.can_undo());
    assert!(history.can_redo());
    assert!(!history.is_dirty());
}

#[test]
fn command_history_restores_scene_after_partial_forward_and_undo_failures() {
    let mut scene = hierarchy_scene();
    let original = scene.clone();
    let mut history = CommandHistory::new();
    assert!(history
        .push(Box::new(MutatesThenFails), &mut scene)
        .is_err());
    assert_eq!(scene, original);
    assert!(!history.can_undo());

    history
        .push(Box::new(UndoMutatesThenFails), &mut scene)
        .unwrap();
    let before_undo = scene.clone();
    assert!(history.undo(&mut scene).is_err());
    assert_eq!(scene, before_undo);
    assert!(history.can_undo());
}

#[test]
fn set_component_field_cannot_blind_insert_a_new_field() {
    let mut scene = engine_scene::sample_scene();
    let original = scene.clone();
    let mut history = CommandHistory::new();
    let result = history.push(
        Box::new(SetComponentField::new(
            "cube-01".into(),
            "engine.renderable".into(),
            "invented_field".into(),
            Value::Bool(true),
        )),
        &mut scene,
    );

    assert!(matches!(
        result,
        Err(EditorError::ComponentFieldNotFound { .. })
    ));
    assert_eq!(scene, original);
    assert!(!history.can_undo());
}

#[test]
fn invalid_core_field_type_is_rejected_after_execute_without_history() {
    let mut scene = engine_scene::sample_scene();
    let original = scene.clone();
    let mut history = CommandHistory::new();
    let result = history.push(
        Box::new(SetComponentField::new(
            "cube-01".into(),
            "engine.renderable".into(),
            "visible".into(),
            Value::Str("not-a-bool".into()),
        )),
        &mut scene,
    );

    assert!(matches!(
        result,
        Err(EditorError::SceneCommandRejected { .. })
    ));
    assert_eq!(scene, original);
    assert!(!history.can_undo());
    assert!(!history.is_dirty());
}

#[test]
fn extension_deserialize_failure_is_rejected_by_runtime_registry_preflight() {
    let mut scene = engine_scene::sample_scene();
    scene.entities[1].components.insert(
        TestExternal::TYPE_ID.into(),
        ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([("value".into(), Value::UInt(7))]),
        },
    );
    // Script fields remain arbitrary Scene-only metadata and must not be
    // handed to the ECS extension registry.
    scene.entities[1].components.insert(
        "engine.script".into(),
        ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: BTreeMap::from([
                ("assembly_id".into(), Value::Str("game".into())),
                ("class_name".into(), Value::Str("Player".into())),
                ("user_field".into(), Value::Map(BTreeMap::new())),
            ]),
        },
    );
    let registry = strict_test_registry();
    let mut editor = crate::EditorScene::new_with_component_registry(scene, registry).unwrap();
    let original = editor.scene.clone();

    let result = editor.execute(Box::new(SetComponentField::new(
        "cube-01".into(),
        TestExternal::TYPE_ID.into(),
        "value".into(),
        Value::Str("invalid".into()),
    )));

    assert!(matches!(
        result,
        Err(EditorError::SceneCommandRejected { .. })
    ));
    assert_eq!(editor.scene, original);
    assert!(!editor.history.can_undo());
}

#[test]
fn validation_failure_during_undo_and_redo_is_atomic_and_keeps_stack_side() {
    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();
    history
        .push(Box::new(UndoProducesInvalidScene), &mut scene)
        .unwrap();
    history.mark_clean();
    let before_undo = scene.clone();

    assert!(matches!(
        history.undo(&mut scene),
        Err(EditorError::SceneCommandRejected { .. })
    ));
    assert_eq!(scene, before_undo);
    assert!(history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.is_dirty());

    let mut scene = engine_scene::sample_scene();
    let mut history = CommandHistory::new();
    history
        .push(
            Box::new(RedoProducesInvalidScene { executions: 0 }),
            &mut scene,
        )
        .unwrap();
    history.undo(&mut scene).unwrap();
    history.mark_clean();
    let before_redo = scene.clone();

    assert!(matches!(
        history.redo(&mut scene),
        Err(EditorError::SceneCommandRejected { .. })
    ));
    assert_eq!(scene, before_redo);
    assert!(!history.can_undo());
    assert!(history.can_redo());
    assert!(!history.is_dirty());
}

#[test]
fn command_batch_failure_is_atomic_even_when_a_child_partially_mutates() {
    let mut scene = hierarchy_scene();
    let original = scene.clone();
    let mut batch = CommandBatch::new(
        "Atomic batch",
        vec![
            Box::new(SetEntityName::new("root".into(), Some("changed".into()))),
            Box::new(MutatesThenFails),
        ],
    );
    assert!(batch.execute(&mut scene).is_err());
    assert_eq!(scene, original);
}
