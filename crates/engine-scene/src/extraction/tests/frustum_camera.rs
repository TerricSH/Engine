    // Frustum culling tests.

    #[test]
    fn frustum_planes_from_identity() {
        let view_proj = glam::Mat4::IDENTITY;
        let planes = extract_frustum_planes(&view_proj);
        assert_eq!(planes.len(), 6);
        // All planes should be normalised.
        for (i, plane) in planes.iter().enumerate() {
            let len = plane.truncate().length();
            assert!(
                (len - 1.0).abs() < 1e-6,
                "plane {} not normalised (len={})",
                i,
                len
            );
        }
    }

    #[test]
    fn aabb_inside_default_frustum() {
        // A simple perspective frustum looking down -Z.
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box at origin (in front of camera).
        assert!(aabb_in_frustum([0.0, 0.0, -5.0], [0.5, 0.5, 0.5], &frustum));
    }

    #[test]
    fn aabb_outside_frustum_culled() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far behind the camera.
        assert!(!aabb_in_frustum(
            [0.0, 0.0, 10.0],
            [0.5, 0.5, 0.5],
            &frustum
        ));
    }

    #[test]
    fn aabb_far_beyond_far_plane() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Box far beyond the far plane.
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -200.0],
            [1.0, 1.0, 1.0],
            &frustum
        ));
    }

    #[test]
    fn aabb_partially_inside_is_visible() {
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 0.1, 100.0);
        let view = glam::Mat4::IDENTITY;
        let frustum = extract_frustum_planes(&(proj * view));

        // Large box straddling the camera should be visible.
        assert!(aabb_in_frustum(
            [0.0, 0.0, -2.0],
            [10.0, 10.0, 10.0],
            &frustum
        ));
    }

    #[test]
    fn zero_to_one_frustum_uses_exact_near_and_far_planes() {
        let near = 1.0;
        let far = 10.0;
        let projection = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, near, far);
        let frustum = extract_frustum_planes(&projection);

        assert!(aabb_in_frustum(
            [0.0, 0.0, -(near + 0.01)],
            [0.0; 3],
            &frustum
        ));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(near - 0.1)],
            [0.0; 3],
            &frustum
        ));
        assert!(aabb_in_frustum(
            [0.0, 0.0, -(far - 0.01)],
            [0.0; 3],
            &frustum
        ));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(far + 0.1)],
            [0.0; 3],
            &frustum
        ));

        let near_clip = projection * glam::Vec4::new(0.0, 0.0, -near, 1.0);
        let far_clip = projection * glam::Vec4::new(0.0, 0.0, -far, 1.0);
        assert!((near_clip.z / near_clip.w).abs() <= 1.0e-6);
        assert!((far_clip.z / far_clip.w - 1.0).abs() <= 1.0e-6);
    }

    #[test]
    fn zero_to_one_orthographic_frustum_uses_exact_near_and_far_planes() {
        let near = 2.0;
        let far = 6.0;
        let projection = glam::Mat4::orthographic_rh(-2.0, 2.0, -2.0, 2.0, near, far);
        let frustum = extract_frustum_planes(&projection);

        assert!(aabb_in_frustum([0.0, 0.0, -near], [0.0; 3], &frustum));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(near - 0.1)],
            [0.0; 3],
            &frustum
        ));
        assert!(aabb_in_frustum([0.0, 0.0, -far], [0.0; 3], &frustum));
        assert!(!aabb_in_frustum(
            [0.0, 0.0, -(far + 0.1)],
            [0.0; 3],
            &frustum
        ));
    }

    // World extraction tests.

    #[test]
    fn extract_from_world_with_camera_yields_view() {
        let mut world = World::new();
        let e = world.create_entity();
        world.add_component(e, components::Camera::default());
        world.add_component(e, components::Transform::default());

        let result = extract_renderer_input_from_world(&world, 0);
        assert!(result.is_ok(), "extraction failed: {:?}", result.err());
        let input = result.unwrap();
        assert_eq!(input.views.len(), 1);
        assert_eq!(input.frame_index, 0);
    }

    #[test]
    fn concrete_surface_context_composes_base_and_overlay_viewports_and_projection_aspects() {
        let mut world = World::new();
        let base = world.create_entity();
        let base_camera = components::Camera::default();
        world.add_component(base, base_camera.clone());
        world.add_component(base, components::Transform::default());

        let overlay = world.create_entity();
        world.add_component(
            overlay,
            components::Camera {
                viewport_rect: Some([0.25, 0.0, 0.5, 1.0]),
                priority: 1,
                ..base_camera.clone()
            },
        );
        world.add_component(overlay, components::Transform::default());

        let output = Rect {
            min: [0.2, 0.25],
            max: [0.8, 0.75],
        };
        let context = RenderViewportContext::new(1000, 800, output).unwrap();
        let input = extract_renderer_input_from_world_with_viewport(&world, 4, context).unwrap();

        assert_eq!(input.views[0].viewport_rect_normalized, output);
        assert_eq!(input.views[0].viewport, output);
        let overlay_viewport = input.views[1].viewport_rect_normalized;
        for (actual, expected) in overlay_viewport
            .min
            .into_iter()
            .chain(overlay_viewport.max)
            .zip([0.35, 0.25, 0.65, 0.75])
        {
            assert!((actual - expected).abs() <= 1.0e-6);
        }
        assert_mat4_approx(
            &input.views[0].projection_matrix,
            glam::Mat4::perspective_rh(
                base_camera.fov_y,
                600.0 / 400.0,
                base_camera.near,
                base_camera.far,
            ),
        );
        assert_mat4_approx(
            &input.views[1].projection_matrix,
            glam::Mat4::perspective_rh(
                base_camera.fov_y,
                300.0 / 400.0,
                base_camera.near,
                base_camera.far,
            ),
        );
    }

    #[test]
    fn world_extraction_preserves_scene_render_options_and_camera_exposure() {
        let mut world = World::new();
        world.scene_settings.tone_mapping = engine_renderer::ToneMapping::Reinhard;
        world.scene_settings.pass_graph_config.enabled = false;
        world.scene_settings.environment_map =
            Some(engine_serialize::AssetId::new("sunset-environment"));
        world.scene_settings.environment_intensity = 1.75;
        world.scene_settings.environment_rotation_radians = 0.5;
        world.scene_settings.reflection_probes = vec![engine_renderer::ReflectionProbe {
            entity: Some("probe-lobby".into()),
            environment_map: engine_serialize::AssetId::new("lobby-environment"),
            position: [1.0, 2.0, 3.0],
            half_extents: [4.0, 5.0, 6.0],
            blend_distance: 2.0,
            priority: 3,
        }];
        world.scene_settings.post_process.bloom.enabled = true;
        world.scene_settings.post_process.bloom.intensity = 0.4;
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                aperture: 2.0,
                shutter_speed: 0.25,
                iso: 100.0,
                ev_compensation: 1.0,
                msaa_samples: 4,
                ..Default::default()
            },
        );
        world.add_component(camera, components::Transform::default());

        let input = extract_renderer_input_from_world(&world, 9).expect("valid extraction");

        assert_eq!(
            input.render_options.tone_mapping,
            engine_renderer::ToneMapping::Reinhard
        );
        assert!(!input.render_options.pass_graph_config.enabled);
        assert_eq!(
            input
                .render_options
                .environment
                .environment_map
                .as_ref()
                .map(|asset| asset.id.as_str()),
            Some("sunset-environment")
        );
        assert_eq!(input.render_options.environment.intensity, 1.75);
        assert_eq!(input.render_options.environment.rotation_radians, 0.5);
        assert_eq!(input.render_options.environment.reflection_probes.len(), 1);
        assert!(input.render_options.post_process.bloom.enabled);
        assert_eq!(input.render_options.post_process.bloom.intensity, 0.4);
        assert_eq!(input.render_options.msaa_samples, 4);
        assert_eq!(input.views[0].msaa_samples, 4);
        assert!((input.render_options.exposure_ev100.unwrap() - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn world_extraction_rejects_invalid_physical_exposure() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                aperture: 0.0,
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("invalid exposure must not reach tone mapping");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0034"));
    }

    #[test]
    fn extract_from_world_without_camera_fails() {
        let world = World::new();
        let result = extract_renderer_input_from_world(&world, 0);
        assert!(
            result.is_err(),
            "expected extraction to fail without camera"
        );
    }

    #[test]
    fn registered_render_layers_have_stable_bits() {
        assert_eq!(render_layer_bit("Default"), Some(0));
        assert_eq!(render_layer_bit("opaque"), Some(0));
        assert_eq!(render_layer_bit("Transparent"), Some(1));
        assert_eq!(render_layer_bit("UI"), Some(2));
        assert_eq!(render_layer_bit("post-process"), Some(3));
        assert_eq!(render_layer_bit("Debug"), Some(4));
        assert_eq!(render_layer_bit("User0"), Some(5));
        assert_eq!(render_layer_bit("User26"), Some(31));
        assert_eq!(render_layer_bit("User27"), None);
        assert_eq!(render_layer_bit("unregistered"), None);
    }

    #[test]
    fn camera_layer_mask_culls_non_matching_drawables() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                render_layer_mask: 1 << 1,
                ..Default::default()
            },
        );

        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-layered".into(),
                material_asset: "material-layered".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Transform {
                translation: glam::Vec3::new(0.0, 0.0, -5.0),
                ..Default::default()
            },
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("valid extraction");
        assert!(input.drawables.is_empty());
        assert_eq!(input.extraction_stats.unwrap().culled_drawables, 1);
    }

    #[test]
    fn unregistered_render_layer_fails_closed() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-unknown-layer".into(),
                material_asset: "material-unknown-layer".into(),
                visible: true,
                cast_shadows: true,
                render_layer: "GameplaySecret".into(),
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("unknown layers must not be rendered implicitly");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0033"));
    }

    #[test]
    fn additional_cameras_extract_as_non_clearing_overlays() {
        let mut world = World::new();
        let overlay = world.create_entity();
        world.add_component(
            overlay,
            components::Camera {
                priority: 10,
                ..Default::default()
            },
        );
        let base = world.create_entity();
        world.add_component(
            base,
            components::Camera {
                priority: -10,
                ..Default::default()
            },
        );

        let input = extract_renderer_input_from_world(&world, 0).expect("valid extraction");
        assert!(matches!(input.views[0].compose, ViewCompose::Base { .. }));
        assert!(matches!(
            input.views[1].compose,
            ViewCompose::Overlay {
                base_view_id: 0,
                blend_mode: BlendMode::Replace
            }
        ));
        assert_eq!(input.views[1].clear_flags, ClearFlags::Nothing);
    }

    #[test]
    fn camera_skybox_clear_flag_maps_to_renderer_contract() {
        assert_eq!(map_clear_flags(0b100), ClearFlags::Skybox);
        assert_eq!(map_clear_flags(0b111), ClearFlags::Skybox);
        assert_eq!(map_clear_flags(0b011), ClearFlags::ColorAndDepth);
    }

    #[test]
    fn invalid_camera_viewport_and_msaa_are_rejected() {
        let mut world = World::new();
        let camera = world.create_entity();
        world.add_component(
            camera,
            components::Camera {
                viewport_rect: Some([0.75, 0.0, 0.5, 1.0]),
                msaa_samples: 3,
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 0)
            .expect_err("invalid camera settings must fail extraction");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0031"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SC0032"));
    }
