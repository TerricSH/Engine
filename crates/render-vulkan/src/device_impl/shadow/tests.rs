    use super::{CascadeDataError, VulkanDevice};
    use glam::{Mat4, Vec3};

    fn assert_approx(actual: f32, expected: f32) {
        let tolerance = expected.abs().max(1.0) * 1.0e-4;
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn derives_clip_planes_from_rh_zo_perspective_projection() {
        let expected_near = 0.25;
        let expected_far = 750.0;
        let projection = Mat4::perspective_rh(
            60.0f32.to_radians(),
            16.0 / 9.0,
            expected_near,
            expected_far,
        );

        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection)
            .expect("finite perspective clip planes should be recoverable");

        assert_approx(near, expected_near);
        assert_approx(far, expected_far);
    }

    #[test]
    fn derives_clip_planes_from_rh_zo_orthographic_projection() {
        let expected_near = 2.0;
        let expected_far = 42.0;
        let projection = Mat4::orthographic_rh(-8.0, 12.0, -5.0, 7.0, expected_near, expected_far);

        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection)
            .expect("finite orthographic clip planes should be recoverable");

        assert_approx(near, expected_near);
        assert_approx(far, expected_far);
    }

    #[test]
    fn different_directional_lights_produce_different_cascade_matrices() {
        let view = Mat4::look_at_rh(Vec3::new(3.0, 4.0, 8.0), Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective_rh(55.0f32.to_radians(), 1.5, 0.2, 80.0);
        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection).unwrap();

        let (_, first) = VulkanDevice::compute_cascade_data(
            &view,
            &projection,
            near,
            far,
            Vec3::new(1.0, -2.0, 0.5),
        )
        .expect("first light direction should produce valid cascades");
        let (_, second) = VulkanDevice::compute_cascade_data(
            &view,
            &projection,
            near,
            far,
            Vec3::new(-0.25, -1.0, -1.5),
        )
        .expect("second light direction should produce valid cascades");

        let maximum_difference = first
            .iter()
            .zip(second.iter())
            .flat_map(|(left, right)| {
                left.to_cols_array()
                    .into_iter()
                    .zip(right.to_cols_array())
                    .map(|(left, right)| (left - right).abs())
            })
            .fold(0.0f32, f32::max);
        assert!(maximum_difference > 1.0e-3);
    }

    #[test]
    fn camera_relative_view_produces_consistent_cascade_matrices() {
        // ENG-01: cascades derive from the same view matrix the scene pass
        // uses, so a consistent camera-relative shift must map shifted world
        // points to (nearly) the same light-clip coordinates as the absolute
        // frame — shadows come free with the shift.
        let origin = Vec3::new(100_000.0, 0.0, 100_000.0);
        let rotation = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let view_absolute = Mat4::from_rotation_translation(rotation, origin).inverse();
        let view_relative = Mat4::from_quat(rotation).inverse(); // translation-free

        let projection = Mat4::perspective_rh(55.0f32.to_radians(), 16.0 / 9.0, 0.1, 500.0);
        let (near, far) = VulkanDevice::derive_rh_zo_clip_planes(&projection).unwrap();
        let light_direction = Vec3::new(0.4, -1.0, 0.25);

        let (splits_absolute, vps_absolute) = VulkanDevice::compute_cascade_data(
            &view_absolute,
            &projection,
            near,
            far,
            light_direction,
        )
        .expect("absolute view should produce valid cascades");
        let (splits_relative, vps_relative) = VulkanDevice::compute_cascade_data(
            &view_relative,
            &projection,
            near,
            far,
            light_direction,
        )
        .expect("camera-relative view should produce valid cascades");

        // View-space split distances are translation-invariant.
        for (absolute, relative) in splits_absolute.iter().zip(splits_relative.iter()) {
            assert_approx(*relative, *absolute);
        }

        // A consistently shifted point must land on the same light-clip
        // coordinate in both frames. Tolerance: the absolute frame itself
        // quantizes the point and matrices to a ~8 mm grid at 100 km, so
        // the frames cannot agree below that floor.
        for (vp_absolute, vp_relative) in vps_absolute.iter().zip(vps_relative.iter()) {
            for local in [
                Vec3::new(1.5, -0.25, -3.0),
                Vec3::new(-2.0, 0.5, -12.0),
                Vec3::new(0.25, 1.0, -60.0),
            ] {
                let point_absolute = origin + rotation * local;
                let point_relative = point_absolute - origin;
                let clip_absolute = vp_absolute.project_point3(point_absolute);
                let clip_relative = vp_relative.project_point3(point_relative);
                let mismatch = (clip_absolute - clip_relative).length();
                assert!(
                    mismatch <= 2.0e-2,
                    "cascade light-clip mismatch {mismatch} between absolute and camera-relative frames"
                );
            }
        }
    }

    #[test]
    fn invalid_shadow_inputs_are_rejected_without_a_fixed_fallback() {
        assert_eq!(
            VulkanDevice::normalize_shadow_light_direction(Vec3::ZERO),
            Err(CascadeDataError::InvalidLightDirection)
        );
        assert_eq!(
            VulkanDevice::normalize_shadow_light_direction(Vec3::new(f32::NAN, 0.0, 1.0)),
            Err(CascadeDataError::InvalidLightDirection)
        );
        assert_eq!(
            VulkanDevice::derive_rh_zo_clip_planes(&Mat4::IDENTITY),
            Err(CascadeDataError::InvalidClipPlanes)
        );

        let projection = Mat4::perspective_rh(60.0f32.to_radians(), 1.0, 0.1, 100.0);
        assert_eq!(
            VulkanDevice::compute_cascade_data(
                &Mat4::IDENTITY,
                &projection,
                0.1,
                100.0,
                Vec3::ZERO,
            ),
            Err(CascadeDataError::InvalidLightDirection)
        );
    }
