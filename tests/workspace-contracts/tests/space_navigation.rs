use engine_nav::{SpaceCell, SpaceNavConfig, SpaceNavGrid};
use glam::Vec3;

#[test]
fn three_dimensional_path_crosses_only_the_authored_wall_gap() {
    let mut grid =
        SpaceNavGrid::new(Vec3::ZERO, [12, 4, 12], 1.0).expect("space grid fixture must be valid");
    for y in 0..4 {
        for z in 0..12 {
            if z != 7 {
                assert!(grid.set_blocked(SpaceCell { x: 6, y, z }, true));
            }
        }
    }

    let path = grid
        .find_path_with_config(
            Vec3::new(1.5, 1.5, 1.5),
            Vec3::new(10.5, 1.5, 10.5),
            SpaceNavConfig {
                simplify_path: false,
                ..SpaceNavConfig::default()
            },
        )
        .expect("the authored gap must keep the route connected");
    assert!(path.waypoints().len() >= 2);
    assert!(path.length().is_finite() && path.length() > 0.0);
    assert!(path.waypoints().iter().all(|point| {
        grid.world_to_cell(*point)
            .is_some_and(|cell| !grid.is_blocked(cell))
    }));
    assert!(path.waypoints().iter().any(|point| {
        grid.world_to_cell(*point)
            .is_some_and(|cell| cell.x == 6 && cell.z == 7)
    }));
}
