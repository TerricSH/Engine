    #[test]
    fn camera_relative_rendering_collapses_far_origin_error() {
        let absolute = measure_far_origin_precision(1.0e5, false);
        let near = measure_far_origin_precision(1.0e3, true);
        let mid = measure_far_origin_precision(1.0e4, true);
        let far = measure_far_origin_precision(1.0e5, true);
        for (label, m) in [("1km", &near), ("10km", &mid), ("100km", &far)] {
            println!(
                "camera-relative worst-case {label:>5}: view_space_error={:.3e} m (relative {:.3e}), ndc_xy_error={:.3e}",
                m.view_space_error_m, m.view_space_relative_error, m.ndc_xy_error
            );
        }

        // Phase 1 acceptance: the Phase 0 baseline measured 5.5e-3 relative
        // view-space error and 7.8e-3 NDC error at 100 km with the flag off.
        for (label, m) in [("1km", &near), ("10km", &mid), ("100km", &far)] {
            assert!(
                m.view_space_relative_error <= 1.0e-4,
                "camera-relative view-space error at {label} must collapse to ≤1e-4, got {:.3e}",
                m.view_space_relative_error
            );
            assert!(
                m.ndc_xy_error <= 1.0e-4,
                "camera-relative NDC error at {label} must collapse to ≤1e-4, got {:.3e}",
                m.ndc_xy_error
            );
        }
        // At least 100x better than the absolute pipeline at 100 km.
        assert!(
            far.view_space_error_m <= absolute.view_space_error_m * 0.01,
            "camera-relative error {:.3e} m should be ≥100x below absolute error {:.3e} m",
            far.view_space_error_m,
            absolute.view_space_error_m
        );
    }

    #[test]
    fn camera_relative_rendering_emits_translation_free_base_view_and_relative_drawable() {
        let (mut world, _camera, _drawable, drawable_pos64) =
            far_origin_world(1.0e5, std::f32::consts::FRAC_PI_4, true);
        // Re-extract from the same world with the flag enabled.
        world.scene_settings.camera_relative_rendering = true;
        let input = extract_renderer_input_from_world(&world, 0).expect("extracts");

        // Base view matrix is translation-free: the camera sits at the
        // camera-relative origin.
        let view = glam::Mat4::from_cols_array(&input.views[0].view_matrix);
        assert!(
            view.w_axis.truncate().length() <= 1.0e-6,
            "base view must be translation-free, got w_axis {:?}",
            view.w_axis
        );

        // The drawable world transform is translated by exactly `-origin`,
        // where origin is the base camera's stored world position.
        let origin = camera_relative_render_origin(&world).expect("flag enabled");
        let model = glam::Mat4::from_cols_array(&input.drawables[0].world_transform);
        let expected_translation = glam::Vec3::new(
            drawable_pos64.x as f32,
            drawable_pos64.y as f32,
            drawable_pos64.z as f32,
        ) - origin;
        assert!(
            model
                .w_axis
                .truncate()
                .abs_diff_eq(expected_translation, 1.0e-4),
            "drawable translation {:?} should be camera-relative {:?}",
            model.w_axis,
            expected_translation
        );

        // Emitted bounds move with the drawable.
        let bounds_center = glam::Vec3::from(input.drawables[0].bounds.min)
            + glam::Vec3::from(input.drawables[0].bounds.max);
        assert!(
            (bounds_center * 0.5).abs_diff_eq(expected_translation, 1.0e-4),
            "drawable bounds should shift with the transform"
        );
    }

    #[test]
    fn camera_relative_rendering_uses_resolved_parented_camera_position() {
        let mut world = World::new();
        world.scene_settings.camera_relative_rendering = true;

        let parent = world.create_entity();
        let parent_transform = components::Transform {
            translation: glam::Vec3::new(1.0e5, 50.0, 1.0e5),
            rotation: glam::Quat::from_rotation_y(0.3),
            scale: glam::Vec3::splat(2.0),
            parent: None,
        };
        world.add_component(parent, parent_transform.clone());

        let camera = world.create_entity();
        let camera_transform = components::Transform {
            translation: glam::Vec3::new(3.0, -1.0, 2.0),
            rotation: glam::Quat::from_rotation_x(0.1),
            scale: glam::Vec3::ONE,
            parent: Some(parent),
        };
        world.add_component(camera, camera_transform.clone());
        world.add_component(camera, components::Camera::default());

        // The origin is the *resolved* world position of the parented camera.
        let expected_world = glam::Mat4::from_scale_rotation_translation(
            parent_transform.scale,
            parent_transform.rotation,
            parent_transform.translation,
        ) * glam::Mat4::from_scale_rotation_translation(
            camera_transform.scale,
            camera_transform.rotation,
            camera_transform.translation,
        );
        let expected_origin = expected_world.transform_point3(glam::Vec3::ZERO);
        let origin = camera_relative_render_origin(&world).expect("flag enabled");
        assert!(
            origin.abs_diff_eq(expected_origin, 1.0e-3),
            "origin {origin:?} should be the resolved camera position {expected_origin:?}"
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("extracts");
        let view = glam::Mat4::from_cols_array(&input.views[0].view_matrix);
        assert!(
            view.w_axis.truncate().length() <= 1.0e-5,
            "parented base view must be translation-free, got {:?}",
            view.w_axis
        );

        // The shifted view equals the inverse of the camera world matrix
        // translated by `-origin` (shift-then-invert, exactly as extraction
        // builds it).
        let expected_view =
            (glam::Mat4::from_translation(-expected_origin) * expected_world).inverse();
        assert_mat4_approx(&input.views[0].view_matrix, expected_view);
    }

    #[test]
    fn camera_relative_rendering_shifts_lights_but_not_directions() {
        let mut world = World::new();
        world.scene_settings.camera_relative_rendering = true;

        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(
            camera,
            components::Transform {
                translation: glam::Vec3::new(1.0e5, 0.0, 1.0e5),
                rotation: glam::Quat::from_rotation_y(0.7),
                scale: glam::Vec3::ONE,
                parent: None,
            },
        );

        let point_light = world.create_entity();
        world.add_component(
            point_light,
            components::Transform {
                translation: glam::Vec3::new(100_003.0, 1.5, 99_998.0),
                rotation: glam::Quat::from_rotation_z(0.4),
                scale: glam::Vec3::ONE,
                parent: None,
            },
        );
        world.add_component(
            point_light,
            components::Light {
                kind: components::LightKind::Point,
                color: [1.0; 3],
                intensity: 5.0,
                range: 25.0,
                spot_angles: None,
                shadow_mode: 0,
                direction: [0.0, -1.0, 0.0],
            },
        );

        let relative = extract_renderer_input_from_world(&world, 0).expect("extracts");
        let origin = camera_relative_render_origin(&world).unwrap();
        let expected_position = glam::Vec3::new(100_003.0, 1.5, 99_998.0) - origin;
        assert!(
            glam::Vec3::from(relative.lights[0].position).abs_diff_eq(expected_position, 1.0e-3),
            "light position {:?} should be camera-relative {:?}",
            relative.lights[0].position,
            expected_position
        );

        // Directions are translation-invariant: identical to the absolute
        // extraction, bit for bit.
        world.scene_settings.camera_relative_rendering = false;
        let absolute = extract_renderer_input_from_world(&world, 0).expect("extracts");
        assert_eq!(relative.lights[0].direction, absolute.lights[0].direction);
        let expected_absolute_position = glam::Vec3::new(100_003.0, 1.5, 99_998.0);
        assert!(
            glam::Vec3::from(absolute.lights[0].position)
                .abs_diff_eq(expected_absolute_position, 1.0e-3),
            "absolute-mode light position stays in world space"
        );
    }

    #[test]
    fn camera_relative_rendering_overlay_view_keeps_offset_from_base_camera() {
        let mut world = World::new();
        world.scene_settings.camera_relative_rendering = true;

        let base_translation = glam::Vec3::new(1.0e5, 0.0, 1.0e5);
        let base = world.create_entity();
        world.add_component(
            base,
            components::Camera {
                priority: -10,
                ..Default::default()
            },
        );
        world.add_component(
            base,
            components::Transform {
                translation: base_translation,
                ..Default::default()
            },
        );

        let overlay_translation = base_translation + glam::Vec3::new(0.0, 2.0, -3.0);
        let overlay = world.create_entity();
        world.add_component(
            overlay,
            components::Camera {
                priority: 10,
                ..Default::default()
            },
        );
        world.add_component(
            overlay,
            components::Transform {
                translation: overlay_translation,
                ..Default::default()
            },
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("extracts");
        assert_eq!(input.views.len(), 2);

        // Base view is translation-free.
        let base_view = glam::Mat4::from_cols_array(&input.views[0].view_matrix);
        assert!(base_view.w_axis.truncate().length() <= 1.0e-6);

        // The overlay view keeps the overlay camera's offset from the *base*
        // origin (the documented v1 approximation): it equals the inverse of
        // the overlay world matrix translated by `-origin`.
        let origin = camera_relative_render_origin(&world).unwrap();
        assert!(origin.abs_diff_eq(base_translation, 1.0e-3));
        let expected_overlay_view = (glam::Mat4::from_translation(-origin)
            * glam::Mat4::from_translation(overlay_translation))
        .inverse();
        assert_mat4_approx(&input.views[1].view_matrix, expected_overlay_view);
        // It is *not* translation-free: the 2/-3 m offset from the base
        // camera is preserved in camera-relative space.
        let overlay_view = glam::Mat4::from_cols_array(&input.views[1].view_matrix);
        assert!(
            (overlay_view.w_axis.truncate().length() - (2.0_f32 * 2.0 + 3.0 * 3.0).sqrt()).abs()
                <= 1.0e-3,
            "overlay view should keep its offset from the base camera"
        );
    }

    #[test]
    fn camera_relative_render_origin_follows_the_active_camera_setting() {
        let mut world = World::new();
        world.scene_settings.camera_relative_rendering = true;

        // No cameras: no origin.
        assert_eq!(camera_relative_render_origin(&world), None);

        let first = world.create_persistent_entity("camera-first").unwrap();
        world.add_component(
            first,
            components::Camera {
                priority: -10,
                ..Default::default()
            },
        );
        world.add_component(
            first,
            components::Transform {
                translation: glam::Vec3::new(10.0, 0.0, 0.0),
                ..Default::default()
            },
        );
        let second = world.create_persistent_entity("camera-second").unwrap();
        world.add_component(
            second,
            components::Camera {
                priority: 10,
                ..Default::default()
            },
        );
        world.add_component(
            second,
            components::Transform {
                translation: glam::Vec3::new(20.0, 0.0, 0.0),
                ..Default::default()
            },
        );

        // Without an active-camera override the lowest-priority camera wins.
        assert_eq!(
            camera_relative_render_origin(&world),
            Some(glam::Vec3::new(10.0, 0.0, 0.0))
        );
        // The scene's active camera always supplies the origin.
        world.scene_settings.active_camera = Some("camera-second".to_string());
        assert_eq!(
            camera_relative_render_origin(&world),
            Some(glam::Vec3::new(20.0, 0.0, 0.0))
        );

        // Disabled flag: no origin regardless of cameras.
        world.scene_settings.camera_relative_rendering = false;
        assert_eq!(camera_relative_render_origin(&world), None);
    }

    #[test]
    fn active_camera_world_position_ignores_render_flags_and_reads_hierarchy() {
        let mut world = World::new();
        // No camera at all: no position, regardless of flags.
        assert_eq!(active_camera_world_position(&world), None);

        let root = world.create_persistent_entity("rig").unwrap();
        world.add_component(
            root,
            components::Transform {
                translation: glam::Vec3::new(100.0, 0.0, 0.0),
                ..Default::default()
            },
        );
        let camera = world.create_persistent_entity("camera-main").unwrap();
        world.add_component(camera, components::Camera::default());
        world.add_component(
            camera,
            components::Transform {
                translation: glam::Vec3::new(1.0, 2.0, 3.0),
                parent: Some(root),
                ..Default::default()
            },
        );

        // The flag-gated origin stays off while the position resolves
        // through the parent chain.
        assert_eq!(camera_relative_render_origin(&world), None);
        assert_eq!(
            active_camera_world_position(&world),
            Some(glam::Vec3::new(101.0, 2.0, 3.0))
        );
    }

    #[test]
    fn active_camera_view_builds_renderer_consistent_center_ray() {
        let mut world = World::new();
        let camera = world.create_persistent_entity("camera-main").unwrap();
        world.add_component(camera, components::Camera::default());
        world.add_component(
            camera,
            components::Transform {
                translation: glam::Vec3::new(2.0, 3.0, 4.0),
                ..Default::default()
            },
        );
        let viewport = RenderViewportContext::new(1280, 720, engine_renderer::Rect::FULL).unwrap();
        let view = active_camera_view(&world, viewport).unwrap();
        let (origin, direction) = view.screen_ray([640.0, 360.0]).unwrap();
        assert_eq!(view.entity_id.as_deref(), Some("camera-main"));
        assert!(origin.abs_diff_eq(glam::Vec3::new(2.0, 3.0, 4.0), 1.0e-5));
        assert!(direction.abs_diff_eq(glam::Vec3::NEG_Z, 1.0e-4));
        assert!(view.screen_ray([-1.0, 360.0]).is_none());
    }

    #[test]
    fn camera_relative_rendering_preserves_rendered_view_space_near_origin() {
        // The shift must not change what is rendered: `view * model` is
        // invariant. Near the origin both modes agree to f32 noise.
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(
            camera,
            components::Transform {
                translation: glam::Vec3::new(3.0, 2.0, 5.0),
                rotation: glam::Quat::from_rotation_y(0.6),
                scale: glam::Vec3::ONE,
                parent: None,
            },
        );
        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-invariance".into(),
                material_asset: "mat-invariance".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Transform {
                translation: glam::Vec3::new(4.0, 1.0, -2.0),
                rotation: glam::Quat::from_rotation_z(0.9),
                scale: glam::Vec3::new(2.0, 0.5, 1.5),
                parent: None,
            },
        );

        let absolute = extract_renderer_input_from_world(&world, 0).expect("extracts");
        world.scene_settings.camera_relative_rendering = true;
        let relative = extract_renderer_input_from_world(&world, 0).expect("extracts");

        let absolute_chain = glam::Mat4::from_cols_array(&absolute.views[0].view_matrix)
            * glam::Mat4::from_cols_array(&absolute.drawables[0].world_transform);
        let relative_chain = glam::Mat4::from_cols_array(&relative.views[0].view_matrix)
            * glam::Mat4::from_cols_array(&relative.drawables[0].world_transform);
        for (index, (absolute, relative)) in absolute_chain
            .to_cols_array()
            .iter()
            .zip(relative_chain.to_cols_array().iter())
            .enumerate()
        {
            assert!(
                (absolute - relative).abs() <= 1.0e-5,
                "view·model element {index} changed under the shift: {absolute} vs {relative}"
            );
        }
    }
