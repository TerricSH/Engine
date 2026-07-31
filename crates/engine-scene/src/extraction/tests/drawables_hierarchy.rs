    #[test]
    fn extract_from_world_culls_invisible_drawables() {
        let mut world = World::new();
        // Camera looking down -Z.
        let e_cam = world.create_entity();
        world.add_component(e_cam, components::Camera::default());
        world.add_component(e_cam, components::Transform::default());

        // Renderable in front of camera (should be visible).
        let e_front = world.create_entity();
        world.add_component(
            e_front,
            components::Renderable {
                mesh_asset: "mesh-visible".into(),
                material_asset: "mat-default".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            e_front,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -5.0),
                ..Default::default()
            },
        );
        world.add_component(
            e_front,
            components::Bounds {
                center: [0.0, 0.0, 0.0],
                half_extents: [0.5, 0.5, 0.5],
            },
        );

        // Renderable behind camera (should be culled).
        let e_back = world.create_entity();
        world.add_component(
            e_back,
            components::Renderable {
                mesh_asset: "mesh-culled".into(),
                material_asset: "mat-default".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            e_back,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, 10.0),
                ..Default::default()
            },
        );
        world.add_component(
            e_back,
            components::Bounds {
                center: [0.0, 0.0, 0.0],
                half_extents: [0.5, 0.5, 0.5],
            },
        );

        let result = extract_renderer_input_from_world(&world, 1);
        assert!(result.is_ok(), "extraction failed: {:?}", result.err());
        let input = result.unwrap();

        // Only the front drawable should survive culling.
        assert_eq!(input.drawables.len(), 1, "expected 1 visible drawable");
        assert_eq!(input.drawables[0].mesh.id, "mesh-visible");
        assert_eq!(
            input.extraction_stats,
            Some(ExtractionStats {
                visible_drawables: 1,
                culled_drawables: 1,
                visible_lights: 0,
                culled_lights: 0,
            })
        );
    }

    #[test]
    fn extraction_selects_regular_object_lod_before_batching() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(camera, components::Transform::default());

        let object = world.create_entity();
        world.add_component(
            object,
            components::Renderable {
                mesh_asset: "mesh.hero-lod0".into(),
                material_asset: "material.hero".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            object,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -20.0),
                ..Default::default()
            },
        );
        world.add_component(
            object,
            components::LodGroup {
                minimum_distance: 0.0,
                cull_distance: 100.0,
                levels: vec![components::LodLevel {
                    distance: 10.0,
                    mesh_asset: "mesh.hero-lod1".into(),
                    material_asset: None,
                }],
            },
        );

        let input = extract_renderer_input_from_world(&world, 3).unwrap();
        assert_eq!(input.drawables.len(), 1);
        assert_eq!(input.drawables[0].mesh.id, "mesh.hero-lod1");
        assert_eq!(input.drawables[0].material.id, "material.hero");
    }

    #[test]
    fn hlod_cluster_automatically_switches_sources_and_proxy() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(camera, components::Transform::default());

        let source = world.create_entity();
        world.add_component(
            source,
            components::Renderable {
                mesh_asset: "mesh.building-detail".into(),
                material_asset: "material.city".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            source,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -20.0),
                ..Default::default()
            },
        );
        world.add_component(
            source,
            components::HlodCluster {
                cluster_id: "city-block-a".into(),
                role: components::HlodRole::Source,
                activation_distance: 0.0,
                cull_distance: 0.0,
            },
        );

        let proxy = world.create_entity();
        world.add_component(
            proxy,
            components::Renderable {
                mesh_asset: "mesh.city-block-proxy".into(),
                material_asset: "material.city-proxy".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            proxy,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -20.0),
                ..Default::default()
            },
        );
        world.add_component(
            proxy,
            components::HlodCluster {
                cluster_id: "city-block-a".into(),
                role: components::HlodRole::Proxy,
                activation_distance: 10.0,
                cull_distance: 100.0,
            },
        );

        let distant = extract_renderer_input_from_world(&world, 4).unwrap();
        assert_eq!(distant.drawables.len(), 1);
        assert_eq!(distant.drawables[0].mesh.id, "mesh.city-block-proxy");

        world
            .get_mut::<components::Transform>(source)
            .unwrap()
            .translation
            .z = -5.0;
        world
            .get_mut::<components::Transform>(proxy)
            .unwrap()
            .translation
            .z = -5.0;
        let near = extract_renderer_input_from_world(&world, 5).unwrap();
        assert_eq!(near.drawables.len(), 1);
        assert_eq!(near.drawables[0].mesh.id, "mesh.building-detail");
    }

    #[test]
    fn world_extraction_with_light_produces_light_item() {
        let mut world = World::new();
        let e_cam = world.create_entity();
        world.add_component(e_cam, components::Camera::default());
        world.add_component(e_cam, components::Transform::default());

        let e_light = world.create_entity();
        world.add_component(
            e_light,
            crate::components::Light {
                kind: crate::components::LightKind::Point,
                color: [1.0, 0.5, 0.2],
                intensity: 100.0,
                range: 20.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        let input = extract_renderer_input_from_world(&world, 2).expect("world extraction OK");
        assert_eq!(input.lights.len(), 1);
        assert_eq!(input.lights[0].color, [1.0, 0.5, 0.2]);
        assert_eq!(input.lights[0].intensity, 100.0);
        assert_eq!(input.lights[0].range, 20.0);
    }

    #[test]
    fn drawable_uses_cached_multilevel_parent_world_transform() {
        let mut world = World::new();
        add_default_camera(&mut world);

        let root = world.create_entity();
        let root_transform = components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, -8.0),
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::splat(2.0),
            parent: None,
        };
        world.add_component(root, root_transform.clone());

        let middle = world.create_entity();
        let middle_transform = components::Transform {
            translation: glam::Vec3::X,
            rotation: glam::Quat::from_rotation_y(0.25),
            scale: glam::Vec3::new(0.5, 1.0, 0.5),
            parent: Some(root),
        };
        world.add_component(middle, middle_transform.clone());

        let drawable = world.create_entity();
        let drawable_transform = components::Transform {
            translation: glam::Vec3::Y,
            rotation: glam::Quat::from_rotation_x(-0.4),
            scale: glam::Vec3::new(1.0, 0.75, 1.25),
            parent: Some(middle),
        };
        world.add_component(drawable, drawable_transform.clone());
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "hierarchy-mesh".into(),
                material_asset: "hierarchy-material".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Bounds {
                center: [0.0; 3],
                half_extents: [0.1; 3],
            },
        );

        let input = extract_renderer_input_from_world(&world, 3).expect("hierarchy extracts");
        let item = input
            .drawables
            .iter()
            .find(|item| item.mesh.id == "hierarchy-mesh")
            .expect("drawable remains visible");
        let expected = glam::Mat4::from_scale_rotation_translation(
            root_transform.scale,
            root_transform.rotation,
            root_transform.translation,
        ) * glam::Mat4::from_scale_rotation_translation(
            middle_transform.scale,
            middle_transform.rotation,
            middle_transform.translation,
        ) * glam::Mat4::from_scale_rotation_translation(
            drawable_transform.scale,
            drawable_transform.rotation,
            drawable_transform.translation,
        );
        assert_mat4_approx(&item.world_transform, expected);
        let expected_center = expected.transform_point3(glam::Vec3::ZERO);
        let actual_center = glam::Vec3::from_array([
            (item.bounds.min[0] + item.bounds.max[0]) * 0.5,
            (item.bounds.min[1] + item.bounds.max[1]) * 0.5,
            (item.bounds.min[2] + item.bounds.max[2]) * 0.5,
        ]);
        assert!(actual_center.abs_diff_eq(expected_center, 1.0e-5));
    }

    #[test]
    fn parented_camera_view_is_inverse_of_resolved_world_transform() {
        let mut world = World::new();
        let parent = world.create_entity();
        let parent_transform = components::Transform {
            translation: glam::Vec3::new(1.0, 2.0, 3.0),
            rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::new(2.0, 1.0, 0.5),
            parent: None,
        };
        world.add_component(parent, parent_transform.clone());

        let camera = world.create_entity();
        let camera_transform = components::Transform {
            translation: glam::Vec3::new(0.0, 0.0, 2.0),
            rotation: glam::Quat::from_rotation_x(0.2),
            scale: glam::Vec3::ONE,
            parent: Some(parent),
        };
        world.add_component(camera, camera_transform.clone());
        world.add_component(camera, components::Camera::default());

        let input =
            extract_renderer_input_from_world(&world, 4).expect("camera hierarchy extracts");
        let parent_world = glam::Mat4::from_scale_rotation_translation(
            parent_transform.scale,
            parent_transform.rotation,
            parent_transform.translation,
        );
        let local_camera = glam::Mat4::from_scale_rotation_translation(
            camera_transform.scale,
            camera_transform.rotation,
            camera_transform.translation,
        );
        assert_mat4_approx(
            &input.views[0].view_matrix,
            (parent_world * local_camera).inverse(),
        );
    }

    #[test]
    fn parented_light_uses_world_position_and_rotated_direction() {
        let mut world = World::new();
        add_default_camera(&mut world);

        let parent = world.create_entity();
        let parent_transform = components::Transform {
            translation: glam::Vec3::new(1.0, 2.0, -5.0),
            rotation: glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: glam::Vec3::new(2.0, 3.0, 4.0),
            parent: None,
        };
        world.add_component(parent, parent_transform.clone());

        let light = world.create_entity();
        let light_transform = components::Transform {
            translation: glam::Vec3::X,
            parent: Some(parent),
            ..Default::default()
        };
        world.add_component(light, light_transform.clone());
        world.add_component(
            light,
            components::Light {
                kind: components::LightKind::Directional,
                color: [1.0; 3],
                intensity: 1.0,
                range: 10.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [1.0, 0.0, 0.0],
            },
        );

        let input = extract_renderer_input_from_world(&world, 5).expect("light hierarchy extracts");
        let parent_world = glam::Mat4::from_scale_rotation_translation(
            parent_transform.scale,
            parent_transform.rotation,
            parent_transform.translation,
        );
        let local_light = glam::Mat4::from_scale_rotation_translation(
            light_transform.scale,
            light_transform.rotation,
            light_transform.translation,
        );
        let light_world = parent_world * local_light;
        let expected_position = light_world.transform_point3(glam::Vec3::ZERO);
        let expected_direction = light_world.transform_vector3(glam::Vec3::X).normalize();
        assert!(glam::Vec3::from(input.lights[0].position).abs_diff_eq(expected_position, 1.0e-5));
        assert!(glam::Vec3::from(input.lights[0].direction).abs_diff_eq(expected_direction, 1.0e-5));
    }

    #[test]
    fn stale_parent_fails_closed_with_structured_diagnostic() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let stale_parent = world.create_entity();
        assert!(world.destroy_entity(stale_parent));

        let child = world.create_entity();
        world.add_component(
            child,
            components::Transform {
                parent: Some(stale_parent),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 6)
            .expect_err("stale parent must reject extraction");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SC0026")
            .expect("invalid-parent diagnostic");
        assert_eq!(
            diagnostic.fields.get("reason").map(String::as_str),
            Some("stale_or_foreign_domain")
        );
        assert_eq!(
            diagnostic.fields.get("parent_generation"),
            Some(&stale_parent.generation().to_string())
        );
    }
