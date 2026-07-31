#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation",
    feature = "subsystem-ui"
))]
#[test]
fn ragdoll_generates_physics_graph_switches_pose_ownership_and_recovers() {
    use engine_animation::{
        AnimationPlayer, Joint, JointTransform, RagdollBody, RagdollComponent, RagdollConstraint,
        RagdollMode, Skeleton, SkeletonComponent,
    };

    let skeleton = Skeleton {
        joints: vec![
            Joint {
                name: "hips".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            },
            Joint {
                name: "chest".into(),
                parent_index: Some(0),
                local_transform: JointTransform {
                    translation: [0.0, 0.75, 0.0],
                    ..JointTransform::IDENTITY
                },
            },
        ],
        inverse_bind_matrices: vec![
            glam::Mat4::IDENTITY.to_cols_array_2d(),
            glam::Mat4::from_translation(Vec3::new(0.0, -0.75, 0.0)).to_cols_array_2d(),
        ],
    };
    skeleton.validate().unwrap();

    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(engine_scene::sample_scene()).unwrap();
    let skeleton_id = engine_serialize::AssetId::new("npc.skeleton");
    game_loop
        .runtime
        .asset_registry_mut()
        .insert_typed(skeleton_id.clone(), skeleton);
    game_loop
        .runtime
        .loaded_extension_asset_ids
        .entry("skeleton".into())
        .or_default()
        .insert(skeleton_id);
    game_loop
        .runtime
        .with_world_mut(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            world.add_component(
                owner,
                engine_scene::components::Transform {
                    translation: Vec3::new(0.0, 3.0, 0.0),
                    ..Default::default()
                },
            );
            world.add_component(owner, SkeletonComponent::new("npc.skeleton"));
            world.add_component(owner, AnimationPlayer::default());
            world.add_component(
                owner,
                RagdollComponent {
                    bodies: vec![
                        RagdollBody {
                            bone: "hips".into(),
                            ..Default::default()
                        },
                        RagdollBody {
                            bone: "chest".into(),
                            ..Default::default()
                        },
                    ],
                    constraints: vec![RagdollConstraint {
                        parent_bone: "hips".into(),
                        child_bone: "chest".into(),
                        limits: Some([-0.6, 0.6]),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
        })
        .unwrap();

    game_loop.update(0.0);
    let (body_ids, joint_ids) = game_loop
        .runtime
        .with_world(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            let ragdoll = world.get::<RagdollComponent>(owner).unwrap();
            (
                ragdoll
                    .generated_body_ids
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
                ragdoll.generated_joint_ids.clone(),
            )
        })
        .unwrap();
    assert_eq!(body_ids.len(), 2);
    assert_eq!(joint_ids.len(), 1);
    assert_eq!(game_loop.physics.as_ref().unwrap().body_count(), 2);
    assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

    assert_eq!(
        game_loop
            .set_ragdoll_active("cube-01", true, 0.0, Vec3::new(4.0, 0.0, 0.0))
            .unwrap(),
        body_ids
    );
    game_loop.update(1.0 / 60.0);
    game_loop
        .runtime
        .with_world(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            let ragdoll = world.get::<RagdollComponent>(owner).unwrap();
            assert_eq!(ragdoll.mode, RagdollMode::Simulated);
            let player = world.get::<AnimationPlayer>(owner).unwrap();
            assert_eq!(
                player
                    .external_pose_override
                    .as_ref()
                    .expect("physics owns the rendered pose")
                    .weight,
                1.0
            );
            for id in &body_ids {
                let body = world.entity_by_persistent_id(id).unwrap();
                assert_eq!(
                    world
                        .get::<engine_physics::RigidBody>(body)
                        .unwrap()
                        .body_type,
                    engine_physics::BodyType::Dynamic
                );
            }
        })
        .unwrap();

    let checkpoint = game_loop
        .capture_save_game(std::collections::BTreeMap::new())
        .unwrap();
    game_loop
        .runtime
        .with_world_mut(|world| {
            let first_body = world.entity_by_persistent_id(&body_ids[0]).unwrap();
            assert!(world.destroy_entity(first_body));
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            world.get_mut::<RagdollComponent>(owner).unwrap().mode = RagdollMode::Animated;
        })
        .unwrap();
    let restore = game_loop.restore_save_game(checkpoint).unwrap();
    assert_eq!(restore.restored_physics_bodies, 2);
    game_loop
        .runtime
        .with_world(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            assert_eq!(
                world.get::<RagdollComponent>(owner).unwrap().mode,
                RagdollMode::Simulated
            );
            assert!(body_ids
                .iter()
                .all(|id| world.entity_by_persistent_id(id).is_some()));
            assert!(joint_ids
                .iter()
                .all(|id| world.entity_by_persistent_id(id).is_some()));
        })
        .unwrap();
    assert_eq!(game_loop.physics.as_ref().unwrap().joint_count(), 1);

    game_loop
        .set_ragdoll_active("cube-01", false, 0.1, Vec3::ZERO)
        .unwrap();
    game_loop.update(0.05);
    let blend_weight = game_loop
        .runtime
        .with_world(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            world
                .get::<AnimationPlayer>(owner)
                .unwrap()
                .external_pose_override
                .as_ref()
                .unwrap()
                .weight
        })
        .unwrap();
    assert!((blend_weight - 0.5).abs() < 1.0e-4, "{blend_weight}");

    game_loop.update(0.05);
    game_loop
        .runtime
        .with_world(|world| {
            let owner = world.entity_by_persistent_id("cube-01").unwrap();
            assert_eq!(
                world.get::<RagdollComponent>(owner).unwrap().mode,
                RagdollMode::Animated
            );
            assert!(world
                .get::<AnimationPlayer>(owner)
                .unwrap()
                .external_pose_override
                .is_none());
        })
        .unwrap();
}
