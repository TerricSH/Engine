#[test]
fn editor_scene_execute_unknown_entity_entity_not_found() {
    let mut es = EditorScene::new(sample_scene());
    let cmd = Box::new(SetEntityName::new(
        "non-existent".to_string(),
        Some("Nope".to_string()),
    ));
    let result = es.execute(cmd);
    assert!(
        matches!(result, Err(EditorError::EntityNotFound(_))),
        "expected EntityNotFound, got: {:?}",
        result
    );
}

#[test]
fn editor_scene_undo_when_empty_is_noop() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();
    es.undo().expect("undo on empty history should be a no-op");
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "undo on empty history should not change scene"
    );
}

#[test]
fn editor_scene_redo_when_empty_is_noop() {
    let mut es = EditorScene::new(sample_scene());
    let count_before = es.scene.entities.len();
    es.redo().expect("redo on empty history should be a no-op");
    assert_eq!(
        es.scene.entities.len(),
        count_before,
        "redo on empty history should not change scene"
    );
}

// ============================================================================
// Scene validation integration
// ============================================================================

#[test]
fn editor_mutations_never_produce_invalid_scene() {
    // Run a randomized sequence of commands and verify the scene always
    // passes validation.
    let mut es = EditorScene::new(sample_scene());
    let original_id = "camera-main".to_string();

    // Sequence: rename -> add entity -> rename -> undo -> undo
    let cmds: Vec<Box<dyn engine_editor::commands::Command>> = vec![
        Box::new(SetEntityName::new(
            original_id.clone(),
            Some("Cam".to_string()),
        )),
        Box::new(AddEntity::new(make_entity("temp"))),
        Box::new(SetComponentField::new(
            "cube-01".to_string(),
            "engine.renderable".to_string(),
            "visible".to_string(),
            Value::Bool(false),
        )),
    ];

    for cmd in cmds {
        es.execute(cmd).unwrap();
        let diags = validate_scene(&es.scene);
        assert!(
            diags.is_empty(),
            "scene should stay valid after command: {:?}",
            diags
        );
    }

    es.undo().unwrap();
    let diags = validate_scene(&es.scene);
    assert!(diags.is_empty(), "scene valid after 1 undo: {:?}", diags);

    es.undo().unwrap();
    let diags = validate_scene(&es.scene);
    assert!(diags.is_empty(), "scene valid after 2 undo: {:?}", diags);
}

#[test]
fn play_mode_runtime_changes_are_discarded_on_stop() {
    use engine_scene::components::Renderable;

    let mut editor_scene = EditorScene::new(sample_scene());
    editor_scene.selected_entity = Some("cube-01".to_string());
    let authoring_scene = editor_scene.scene.clone();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(authoring_scene.clone()).unwrap();
    let mut play = EditorPlaySession::default();

    assert!(play
        .start(&editor_scene.scene, |scene| game_loop.load_scene(scene))
        .unwrap());
    game_loop.runtime.with_world_mut(|world| {
        let entities = world
            .query::<Renderable>()
            .map(|(entity, _)| entity)
            .collect::<Vec<_>>();
        let cube = entities
            .into_iter()
            .find(|entity| world.persistent_id(*entity) == Some("cube-01"))
            .expect("sample cube entity");
        world.get_mut::<Renderable>(cube).unwrap().visible = false;
    });

    let runtime_visible = game_loop.runtime.with_world(|world| {
        world
            .query::<Renderable>()
            .find_map(|(entity, renderable)| {
                (world.persistent_id(entity) == Some("cube-01")).then_some(renderable.visible)
            })
            .unwrap()
    });
    assert_eq!(runtime_visible, Some(false));
    assert_eq!(editor_scene.scene, authoring_scene);

    assert!(play.stop(|scene| game_loop.load_scene(scene)).unwrap());
    assert_eq!(play.mode(), EditorPlayMode::Editing);
    let restored_visible = game_loop.runtime.with_world(|world| {
        world
            .query::<Renderable>()
            .find_map(|(entity, renderable)| {
                (world.persistent_id(entity) == Some("cube-01")).then_some(renderable.visible)
            })
            .unwrap()
    });
    assert_eq!(restored_visible, Some(true));
    assert_eq!(editor_scene.selected_entity.as_deref(), Some("cube-01"));
    assert!(!editor_scene.is_dirty());
}
