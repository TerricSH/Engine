use super::*;

#[test]
fn project_world_to_screen_identity() {
    // With identity view/proj and viewport 2x2, origin should map to center
    let screen = project_world_to_screen(
        Vec3::ZERO,
        &Mat4::IDENTITY,
        &Mat4::IDENTITY,
        Vec2::new(2.0, 2.0),
    )
    .unwrap();
    // NDC = (0,0,0,1) → screen (1, 1)
    assert!((screen.x - 1.0).abs() < 0.001);
    assert!((screen.y - 1.0).abs() < 0.001);
}

#[test]
fn projection_and_hit_testing_reject_points_behind_camera() {
    let projection = Mat4::perspective_lh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let viewport = Vec2::new(800.0, 800.0);
    assert!(project_world_to_screen(
        Vec3::new(0.0, 0.0, -5.0),
        &Mat4::IDENTITY,
        &projection,
        viewport,
    )
    .is_none());
    assert_eq!(
        screen_distance_to_arrow(
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::X,
            1.0,
            Vec2::new(400.0, 400.0),
            &Mat4::IDENTITY,
            &projection,
            viewport,
        ),
        f32::MAX
    );
}

#[test]
fn gizmo_screen_scale_and_hit_target_are_depth_independent() {
    let projection = Mat4::perspective_lh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let viewport = Vec2::splat(800.0);
    let near = Vec3::new(0.0, 0.0, 5.0);
    let far = Vec3::new(0.0, 0.0, 50.0);
    let near_scale = gizmo_world_scale(near, &Mat4::IDENTITY, &projection, viewport).unwrap();
    let far_scale = gizmo_world_scale(far, &Mat4::IDENTITY, &projection, viewport).unwrap();
    assert!((far_scale / near_scale - 10.0).abs() < 0.01);

    for position in [near, far] {
        let center =
            project_world_to_screen(position, &Mat4::IDENTITY, &projection, viewport).unwrap();
        let mut gizmo = GizmoSystem::new();
        assert!(update_gizmo(
            &mut gizmo,
            position,
            Quat::IDENTITY,
            &Mat4::IDENTITY,
            &projection,
            viewport,
            center + Vec2::new(GIZMO_TARGET_LENGTH_PX * 0.5, 0.0),
            true,
        ));
        assert_eq!(gizmo.drag_axis, Some(GizmoAxis::X));
    }
}

#[test]
fn point_to_line_segment_distance_on_endpoint() {
    let d = point_to_line_segment_distance(
        Vec2::new(0.0, 0.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
    );
    assert!(d < 0.001);
}

#[test]
fn point_to_line_segment_distance_perpendicular() {
    let d = point_to_line_segment_distance(
        Vec2::new(0.5, 1.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
    );
    assert!((d - 1.0).abs() < 0.001);
}
