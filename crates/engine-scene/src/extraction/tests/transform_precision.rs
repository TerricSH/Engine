    #[test]
    fn foreign_world_parent_fails_closed_with_structured_diagnostic() {
        let mut foreign_world = World::new();
        let foreign_parent = foreign_world.create_entity();

        let mut world = World::new();
        add_default_camera(&mut world);
        let child = world.create_entity();
        world.add_component(
            child,
            components::Transform {
                parent: Some(foreign_parent),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 7)
            .expect_err("foreign parent must reject extraction");
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "SC0026"
                && diagnostic.fields.get("reason").map(String::as_str)
                    == Some("stale_or_foreign_domain")
        }));
    }

    #[test]
    fn parent_cycle_fails_closed_without_recursing_forever() {
        let mut world = World::new();
        add_default_camera(&mut world);
        let first = world.create_entity();
        let second = world.create_entity();
        world.add_component(
            first,
            components::Transform {
                parent: Some(second),
                ..Default::default()
            },
        );
        world.add_component(
            second,
            components::Transform {
                parent: Some(first),
                ..Default::default()
            },
        );

        let diagnostics = extract_renderer_input_from_world(&world, 8)
            .expect_err("cyclic hierarchy must reject extraction");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SC0027")
            .expect("cycle diagnostic");
        assert_eq!(
            diagnostic.fields.get("reason").map(String::as_str),
            Some("parent_cycle")
        );
        assert!(diagnostic.fields.get("cycle").is_some_and(|cycle| {
            cycle.contains(&format!("{}:{}", first.index(), first.generation()))
                && cycle.contains(&format!("{}:{}", second.index(), second.generation()))
        }));
    }

    // ── Far-from-origin precision measurements (ENG-01 Phase 0) ─────────────
    //
    // Transforms are f32 end to end, so stored translations quantise to a
    // distance-dependent grid (ulp ≈ 1.2e-4 m at 1 km, ≈ 9.8e-4 m at 10 km,
    // ≈ 7.8e-3 m at 100 km). The `proj * view * model` matrix chain then
    // accumulates rounding at the magnitude of the coordinates, turning
    // sub-grid authored offsets into view-space jitter. These tests measure
    // the loss at 1/10/100 km. They are the acceptance baseline for the
    // camera-relative rendering flag (ENG-01 Phase 1): with the flag enabled
    // the relative view-space error must collapse to ≤ 1e-4 at 100 km.

    /// Precision measurement for one far-from-origin benchmark scene.
    struct FarOriginMeasurement {
        /// |f32 stored drawable translation − f64 authored position| (m).
        /// The irreducible data floor: f32 grid spacing at this distance.
        data_quantization_m: f64,
        /// |extracted f32 model translation − f64 authored position| (m).
        model_translation_error_m: f64,
        /// |f32 (view·model)·origin − f64 reference| in view space (m).
        view_space_error_m: f64,
        /// View-space error relative to the 2 m camera↔drawable offset.
        view_space_relative_error: f64,
        /// |f32 NDC xy − f64 reference NDC xy| for the drawable origin.
        ndc_xy_error: f64,
    }

    fn dvec3_from(value: glam::Vec3) -> glam::DVec3 {
        glam::DVec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
    }

    fn dquat_from(value: glam::Quat) -> glam::DQuat {
        glam::DQuat::from_xyzw(
            f64::from(value.x),
            f64::from(value.y),
            f64::from(value.z),
            f64::from(value.w),
        )
    }

    /// Build the benchmark world: camera at `(d, 0, d)` yawed about Y,
    /// drawable 2 m ahead of the camera along its view axis. Returns the
    /// world, the camera/drawable entities, and the f64 authored drawable
    /// position.
    fn far_origin_world(
        distance: f32,
        yaw_radians: f32,
        camera_relative: bool,
    ) -> (World, crate::Entity, crate::Entity, glam::DVec3) {
        let camera_translation = glam::Vec3::new(distance, 0.0, distance);
        let camera_rotation = glam::Quat::from_rotation_y(yaw_radians);
        let forward64 = dquat_from(camera_rotation) * glam::DVec3::new(0.0, 0.0, -1.0);
        let camera_pos64 = dvec3_from(camera_translation);
        let drawable_pos64 = camera_pos64 + forward64 * 2.0;

        let mut world = World::new();
        world.scene_settings.camera_relative_rendering = camera_relative;
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(
            camera,
            components::Transform {
                translation: camera_translation,
                rotation: camera_rotation,
                scale: glam::Vec3::ONE,
                parent: None,
            },
        );

        let drawable = world.create_entity();
        world.add_component(
            drawable,
            components::Renderable {
                mesh_asset: "mesh-far-origin".into(),
                material_asset: "mat-far-origin".into(),
                visible: true,
                cast_shadows: false,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            drawable,
            components::Transform {
                translation: glam::Vec3::new(
                    drawable_pos64.x as f32,
                    drawable_pos64.y as f32,
                    drawable_pos64.z as f32,
                ),
                ..Default::default()
            },
        );
        (world, camera, drawable, drawable_pos64)
    }

    /// Extract one benchmark frame and compare the f32 matrix chain against
    /// an f64 evaluation of the same stored f32 transforms. The f64
    /// reference isolates pipeline rounding from authored-data quantization.
    /// The reference is mode-agnostic: the camera-relative shift cancels in
    /// `view * model`, so both modes target the same f64 view-space truth.
    fn measure_far_origin_sample(
        distance: f32,
        yaw_radians: f32,
        camera_relative: bool,
    ) -> FarOriginMeasurement {
        let (world, camera, drawable, drawable_pos64) =
            far_origin_world(distance, yaw_radians, camera_relative);
        let camera_component = world.get::<components::Camera>(camera).unwrap().clone();
        let camera_transform = world.get::<components::Transform>(camera).unwrap().clone();
        let drawable_transform = world
            .get::<components::Transform>(drawable)
            .unwrap()
            .clone();

        let input =
            extract_renderer_input_from_world(&world, 0).expect("far-origin benchmark extracts");
        assert_eq!(
            input.drawables.len(),
            1,
            "benchmark drawable must stay visible at {distance} m"
        );

        let view32 = glam::Mat4::from_cols_array(&input.views[0].view_matrix);
        let proj32 = glam::Mat4::from_cols_array(&input.views[0].projection_matrix);
        let model32 = glam::Mat4::from_cols_array(&input.drawables[0].world_transform);

        // f32 pipeline, exactly what the shader chain evaluates.
        let view_pos32 = (view32 * model32).transform_point3(glam::Vec3::ZERO);
        let ndc32 = (proj32 * view32 * model32).project_point3(glam::Vec3::ZERO);

        // f64 reference over the same f32 scene data.
        let camera_world64 = glam::DMat4::from_scale_rotation_translation(
            dvec3_from(camera_transform.scale),
            dquat_from(camera_transform.rotation),
            dvec3_from(camera_transform.translation),
        );
        let drawable_world64 = glam::DMat4::from_scale_rotation_translation(
            dvec3_from(drawable_transform.scale),
            dquat_from(drawable_transform.rotation),
            dvec3_from(drawable_transform.translation),
        );
        let view64 = camera_world64.inverse();
        let view_pos64 = (view64 * drawable_world64).transform_point3(glam::DVec3::ZERO);
        let proj64 = glam::DMat4::perspective_rh(
            f64::from(camera_component.fov_y),
            16.0 / 9.0,
            f64::from(camera_component.near),
            f64::from(camera_component.far),
        );
        let ndc64 = (proj64 * view64 * drawable_world64).project_point3(glam::DVec3::ZERO);

        let view_space_error_m = (dvec3_from(view_pos32) - view_pos64).length();
        FarOriginMeasurement {
            data_quantization_m: (dvec3_from(drawable_transform.translation) - drawable_pos64)
                .length(),
            model_translation_error_m: (dvec3_from(model32.w_axis.truncate()) - drawable_pos64)
                .length(),
            view_space_error_m,
            view_space_relative_error: view_space_error_m / 2.0,
            ndc_xy_error: {
                let dx = f64::from(ndc32.x) - ndc64.x;
                let dy = f64::from(ndc32.y) - ndc64.y;
                (dx * dx + dy * dy).sqrt()
            },
        }
    }

    /// Worst-case measurement over eight camera yaws at the given distance.
    /// Single-sample f32 rounding is essentially random; the worst case over
    /// several orientations is what content actually suffers in practice.
    fn measure_far_origin_precision(distance: f32, camera_relative: bool) -> FarOriginMeasurement {
        let mut aggregate = FarOriginMeasurement {
            data_quantization_m: 0.0,
            model_translation_error_m: 0.0,
            view_space_error_m: 0.0,
            view_space_relative_error: 0.0,
            ndc_xy_error: 0.0,
        };
        for step in 0..8 {
            let yaw = step as f32 * std::f32::consts::FRAC_PI_4;
            let sample = measure_far_origin_sample(distance, yaw, camera_relative);
            println!(
                "  distance={distance:>7} m yaw={:>3}°: data_quantization={:.3e} m, model_error={:.3e} m, view_space_error={:.3e} m (relative {:.3e}), ndc_xy_error={:.3e}",
                step * 45,
                sample.data_quantization_m,
                sample.model_translation_error_m,
                sample.view_space_error_m,
                sample.view_space_relative_error,
                sample.ndc_xy_error
            );
            aggregate.data_quantization_m = aggregate
                .data_quantization_m
                .max(sample.data_quantization_m);
            aggregate.model_translation_error_m = aggregate
                .model_translation_error_m
                .max(sample.model_translation_error_m);
            aggregate.view_space_error_m =
                aggregate.view_space_error_m.max(sample.view_space_error_m);
            aggregate.view_space_relative_error = aggregate
                .view_space_relative_error
                .max(sample.view_space_relative_error);
            aggregate.ndc_xy_error = aggregate.ndc_xy_error.max(sample.ndc_xy_error);
        }
        aggregate
    }

    #[test]
    fn far_origin_world_transform_quantizes_to_distance_dependent_grid() {
        // f32 ulp at 1/10/100 km — the stored-position grid spacing.
        for (distance, expected_ulp_m) in [(1.0e3_f32, 1.2e-4), (1.0e4, 9.8e-4), (1.0e5, 7.8e-3)] {
            let m = measure_far_origin_precision(distance, false);
            println!(
                "distance={distance:>7} m: data_quantization={:.3e} m, model_translation_error={:.3e} m",
                m.data_quantization_m, m.model_translation_error_m
            );
            assert!(
                m.data_quantization_m <= expected_ulp_m,
                "stored-position quantization {:.3e} m exceeds the f32 grid {:.3e} m at {distance} m",
                m.data_quantization_m,
                expected_ulp_m
            );
            assert!(
                m.model_translation_error_m <= expected_ulp_m,
                "extracted world_transform error {:.3e} m exceeds the f32 grid {:.3e} m at {distance} m",
                m.model_translation_error_m,
                expected_ulp_m
            );
        }
    }

    #[test]
    fn far_origin_view_space_error_grows_into_visible_jitter() {
        let near = measure_far_origin_precision(1.0e3, false);
        let mid = measure_far_origin_precision(1.0e4, false);
        let far = measure_far_origin_precision(1.0e5, false);
        for (label, m) in [("1km", &near), ("10km", &mid), ("100km", &far)] {
            println!(
                "worst-case {label:>5}: view_space_error={:.3e} m (relative {:.3e}), ndc_xy_error={:.3e}",
                m.view_space_error_m, m.view_space_relative_error, m.ndc_xy_error
            );
        }

        // The defect this documents: at 100 km the pipeline adds well over
        // 1e-4 relative error to a 2 m camera-relative offset — visible
        // vertex jitter. ENG-01 Phase 1 must collapse this to ≤ 1e-4.
        assert!(
            far.view_space_relative_error > 1.0e-4,
            "expected measurable f32 pipeline error at 100 km, got {:.3e}",
            far.view_space_relative_error
        );
        assert!(
            far.view_space_relative_error < 1.0,
            "pipeline error sanity bound at 100 km, got {:.3e}",
            far.view_space_relative_error
        );
        // Error accumulates at coordinate magnitude: each decade of distance
        // makes the worst-case jitter roughly an order of magnitude worse.
        assert!(
            far.view_space_error_m > mid.view_space_error_m * 4.0,
            "100 km error {:.3e} should dwarf 10 km error {:.3e}",
            far.view_space_error_m,
            mid.view_space_error_m
        );
        assert!(
            mid.view_space_error_m > near.view_space_error_m * 4.0,
            "10 km error {:.3e} should dwarf 1 km error {:.3e}",
            mid.view_space_error_m,
            near.view_space_error_m
        );
        // The composed proj*view chain (what shaders evaluate) degrades to
        // multi-pixel jitter at 100 km.
        assert!(
            far.ndc_xy_error > 1.0e-4,
            "expected visible NDC jitter at 100 km, got {:.3e}",
            far.ndc_xy_error
        );
    }

    // ── Camera-relative rendering (ENG-01 Phase 1) ───────────────────────────
