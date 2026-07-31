#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn game_loop_extracts_scene_canvases_in_stable_order_for_rendering() {
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            for (entity_id, color) in [
                ("cube-01", engine_ui::Color::new(255, 0, 0, 255)),
                ("camera-main", engine_ui::Color::new(0, 255, 0, 255)),
            ] {
                let entity = world.entity_by_persistent_id(entity_id).unwrap();
                let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
                canvas.add_element(engine_ui::UiElement::new(
                    engine_ui::UiElementKind::Panel { color },
                    engine_ui::Layout::FILL,
                ));
                world.add_component(entity, canvas);
            }
        })
        .unwrap();

    let batches = game_loop.runtime_ui_batches();

    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].canvas_id, "camera-main");
    assert_eq!(batches[1].canvas_id, "cube-01");
    assert!(batches.iter().all(|batch| batch.vertices.len() == 4));
    assert!(batches.iter().all(|batch| batch.indices.len() == 6));
    assert!(batches
        .iter()
        .all(|batch| batch.clip_rect.max == [320.0, 180.0]));
}

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn game_loop_advances_loaded_animation_assets_and_keeps_only_the_latest_pose() {
    use engine_animation::{
        AnimationClip, AnimationPlayer, Joint, JointTransform, Skeleton, SkeletonComponent,
    };
    use engine_asset::cook::{registered_asset_type_id, AssetType};

    let cooked = std::env::temp_dir().join(format!(
        "engine_core_game_loop_animation_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&cooked);
    std::fs::create_dir_all(&cooked).unwrap();

    let mut game_loop = GameLoop::new(EngineConfig::default());
    let write_extension = |id: &str, kind: AssetType, source: &[u8]| {
        let type_id = registered_asset_type_id(&kind).unwrap();
        let extension = game_loop
            .runtime
            .asset_type_registry()
            .get(type_id)
            .unwrap();
        let mut payload = Vec::new();
        extension.cooker.unwrap()(source, &mut payload).unwrap();
        engine_asset::cook::write_cooked_artifact(
            &cooked.join(format!("{id}.cooked")),
            kind.kind_code(),
            &payload,
            SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
    };
    let skeleton = Skeleton {
        joints: vec![Joint {
            name: "root".into(),
            parent_index: None,
            local_transform: JointTransform::IDENTITY,
        }],
        inverse_bind_matrices: vec![[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]],
    };
    write_extension(
        "hero.skeleton",
        AssetType::Skeleton,
        &bincode::serialize(&skeleton).unwrap(),
    );
    let clip = AnimationClip {
        name: "idle".into(),
        duration: 1.0,
        channels: vec![],
        joint_indices: vec![],
    };
    write_extension(
        "idle.animation",
        AssetType::Animation,
        &bincode::serialize(&clip).unwrap(),
    );
    game_loop.runtime.load_cooked_assets(&cooked).unwrap();

    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(entity, engine_scene::components::Transform::default());
            world.add_component(entity, SkeletonComponent::new("hero.skeleton"));
            world.add_component(entity, AnimationPlayer::with_clip("idle.animation"));
        })
        .unwrap();

    game_loop.update(0.25);
    assert_eq!(
        game_loop
            .runtime
            .animation_extension_handles()
            .skinned_extract
            .pending_count(),
        1
    );
    assert_eq!(
        game_loop.runtime.with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.get::<AnimationPlayer>(entity).unwrap().current_time
        }),
        Some(0.25)
    );

    game_loop.update(0.25);
    assert_eq!(
        game_loop
            .runtime
            .animation_extension_handles()
            .skinned_extract
            .pending_count(),
        1,
        "fixed updates before a render must replace rather than accumulate poses"
    );
    assert_eq!(
        game_loop.runtime.with_world(|world| {
            let entity = world.entity_by_persistent_id("cube-01").unwrap();
            world.get::<AnimationPlayer>(entity).unwrap().current_time
        }),
        Some(0.5)
    );

    let _ = std::fs::remove_dir_all(cooked);
}
