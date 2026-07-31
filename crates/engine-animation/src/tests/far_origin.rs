// Skinned items bypass engine-scene's `resolve_world_transforms` and emit
// their own f32 world matrix from the entity `Transform`, so they share the
// same far-from-origin quantization as static drawables. This twin of the
// engine-scene measurement documents the defect on the animation path and
// becomes the Phase 1 acceptance test once the camera-relative flag shifts
// skinned items too.

/// Worst-case |f32 (view·model)·origin − f64 reference| for a skinned item
/// 2 m ahead of a camera `(distance, 0, distance)` from the origin, over
/// eight camera yaws. Returns (view-space error m, relative error). The f64
/// reference is mode-agnostic: the camera-relative shift cancels in
/// `view * model`, so both modes target the same view-space truth.
fn measure_far_origin_skinned(distance: f32, camera_relative: bool) -> (f64, f64) {
    let mut worst_error_m = 0.0_f64;
    let mut worst_relative = 0.0_f64;
    for step in 0..8 {
        let yaw = step as f32 * std::f32::consts::FRAC_PI_4;
        let camera_translation = Vec3::new(distance, 0.0, distance);
        let camera_rotation = Quat::from_rotation_y(yaw);
        let forward64 = glam::DQuat::from_xyzw(
            f64::from(camera_rotation.x),
            f64::from(camera_rotation.y),
            f64::from(camera_rotation.z),
            f64::from(camera_rotation.w),
        ) * glam::DVec3::new(0.0, 0.0, -1.0);
        let camera_pos64 = glam::DVec3::new(
            f64::from(camera_translation.x),
            f64::from(camera_translation.y),
            f64::from(camera_translation.z),
        );
        let drawable_pos64 = camera_pos64 + forward64 * 2.0;
        let drawable_translation = Vec3::new(
            drawable_pos64.x as f32,
            drawable_pos64.y as f32,
            drawable_pos64.z as f32,
        );

        let mut world = engine_scene::World::new();
        world.scene_settings_mut().camera_relative_rendering = camera_relative;
        let camera = world.create_entity();
        world.add_component(camera, engine_scene::components::Camera::default());
        world.add_component(
            camera,
            engine_scene::components::Transform {
                translation: camera_translation,
                rotation: camera_rotation,
                scale: Vec3::ONE,
                parent: None,
            },
        );
        let skinned = world.create_entity();
        world.add_component(
            skinned,
            engine_scene::components::Renderable {
                mesh_asset: "mesh-skinned-far".into(),
                material_asset: "mat-skinned-far".into(),
                visible: true,
                cast_shadows: false,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            skinned,
            engine_scene::components::Transform {
                translation: drawable_translation,
                ..Default::default()
            },
        );
        world.add_component(
            skinned,
            SkeletonComponent {
                skeleton_asset: Some("skel-far".into()),
                bind_shape: [0.5, 0.5, 0.5],
                morph_target_set: None,
                morph_weights: Vec::new(),
            },
        );

        let mut skeletons = std::collections::HashMap::new();
        skeletons.insert("skel-far".to_string(), test_skeleton());
        let clips = std::collections::HashMap::new();

        let input = engine_scene::extract_renderer_input_from_world(&world, 0)
            .expect("far-origin skinned benchmark extracts");
        let producer = SkinnedExtractProducer::new();
        bridge_skinned_items(&mut world, &skeletons, &clips, &producer, 0.0);
        let pending = producer.drain();
        assert_eq!(pending.len(), 1, "skinned benchmark item must be produced");

        // f32 pipeline, exactly what the shader chain evaluates.
        let view32 = Mat4::from_cols_array(&input.views[0].view_matrix);
        let model32 = Mat4::from_cols_array_2d(&pending[0].world_transform);
        let view_pos32 = (view32 * model32).transform_point3(Vec3::ZERO);

        // f64 reference over the same stored f32 transforms.
        let camera_world64 = glam::DMat4::from_scale_rotation_translation(
            glam::DVec3::ONE,
            glam::DQuat::from_xyzw(
                f64::from(camera_rotation.x),
                f64::from(camera_rotation.y),
                f64::from(camera_rotation.z),
                f64::from(camera_rotation.w),
            ),
            camera_pos64,
        );
        let drawable_world64 = glam::DMat4::from_translation(glam::DVec3::new(
            f64::from(drawable_translation.x),
            f64::from(drawable_translation.y),
            f64::from(drawable_translation.z),
        ));
        let view_pos64 =
            (camera_world64.inverse() * drawable_world64).transform_point3(glam::DVec3::ZERO);

        let error_m = (glam::DVec3::new(
            f64::from(view_pos32.x),
            f64::from(view_pos32.y),
            f64::from(view_pos32.z),
        ) - view_pos64)
            .length();
        println!(
            "  distance={distance:>7} m yaw={:>3}°: skinned view_space_error={error_m:.3e} m (relative {:.3e})",
            step * 45,
            error_m / 2.0
        );
        worst_error_m = worst_error_m.max(error_m);
        worst_relative = worst_relative.max(error_m / 2.0);
    }
    (worst_error_m, worst_relative)
}

#[test]
fn far_origin_skinned_world_transform_quantizes_like_static_path() {
    let (_, near_relative) = measure_far_origin_skinned(1.0e3, false);
    let (_, mid_relative) = measure_far_origin_skinned(1.0e4, false);
    let (far_error_m, far_relative) = measure_far_origin_skinned(1.0e5, false);
    println!(
        "worst-case skinned relative view-space error: 1km={near_relative:.3e}, 10km={mid_relative:.3e}, 100km={far_relative:.3e}"
    );

    // Same defect as the static path: at 100 km the f32 pipeline adds well
    // over 1e-4 relative error to a 2 m camera-relative offset. ENG-01
    // Phase 1 must collapse this to ≤ 1e-4 on the skinned path too.
    assert!(
        far_relative > 1.0e-4,
        "expected measurable f32 pipeline error on the skinned path at 100 km, got {far_relative:.3e}"
    );
    assert!(
        far_relative < 1.0,
        "skinned pipeline error sanity bound at 100 km, got {far_relative:.3e}"
    );
    assert!(
        far_error_m > 1.0e-4,
        "expected absolute view-space error on the skinned path at 100 km, got {far_error_m:.3e} m"
    );
    let _ = (near_relative, mid_relative);
}

#[test]
fn camera_relative_rendering_collapses_far_origin_skinned_error() {
    let near = measure_far_origin_skinned(1.0e3, true);
    let mid = measure_far_origin_skinned(1.0e4, true);
    let far = measure_far_origin_skinned(1.0e5, true);
    for (label, (error_m, relative)) in [("1km", near), ("10km", mid), ("100km", far)] {
        println!(
            "camera-relative skinned worst-case {label:>5}: view_space_error={error_m:.3e} m (relative {relative:.3e})"
        );
        assert!(
            relative <= 1.0e-4,
            "camera-relative skinned view-space error at {label} must collapse to ≤1e-4, got {relative:.3e}"
        );
    }
}

#[test]
fn bridge_skinned_items_shifts_world_transform_when_camera_relative_enabled() {
    let build = |camera_relative: bool| {
        let mut world = engine_scene::World::new();
        world.scene_settings_mut().camera_relative_rendering = camera_relative;
        let camera = world.create_entity();
        world.add_component(camera, engine_scene::components::Camera::default());
        world.add_component(
            camera,
            engine_scene::components::Transform {
                translation: Vec3::new(100.0, 0.0, 0.0),
                ..Default::default()
            },
        );
        let skinned = world.create_entity();
        world.add_component(
            skinned,
            engine_scene::components::Renderable {
                mesh_asset: "mesh-skinned-shift".into(),
                material_asset: "mat-skinned-shift".into(),
                visible: true,
                cast_shadows: false,
                render_layer: "Default".into(),
            },
        );
        world.add_component(
            skinned,
            engine_scene::components::Transform {
                translation: Vec3::new(100.0, 0.0, -2.0),
                ..Default::default()
            },
        );
        world.add_component(
            skinned,
            SkeletonComponent {
                skeleton_asset: Some("skel-shift".into()),
                bind_shape: [0.5, 0.5, 0.5],
                morph_target_set: None,
                morph_weights: Vec::new(),
            },
        );

        let mut skeletons = std::collections::HashMap::new();
        skeletons.insert("skel-shift".to_string(), test_skeleton());
        let clips = std::collections::HashMap::new();
        let producer = SkinnedExtractProducer::new();
        bridge_skinned_items(&mut world, &skeletons, &clips, &producer, 0.0);
        let pending = producer.drain();
        assert_eq!(pending.len(), 1);
        Mat4::from_cols_array_2d(&pending[0].world_transform)
            .w_axis
            .truncate()
    };

    // Flag off (default): the skinned world transform stays absolute.
    let absolute_translation = build(false);
    assert!(absolute_translation.abs_diff_eq(Vec3::new(100.0, 0.0, -2.0), 1.0e-6));

    // Flag on: translated by `-base_camera_position`, matching the shift
    // extraction applies to static drawables and views.
    let relative_translation = build(true);
    assert!(
        relative_translation.abs_diff_eq(Vec3::new(0.0, 0.0, -2.0), 1.0e-5),
        "skinned world transform should be camera-relative, got {relative_translation:?}"
    );
}

// ── Debug draw tests ───────────────────────────────────────────────────
