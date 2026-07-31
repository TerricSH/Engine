    use super::*;
    use crate::World;

    fn assert_mat4_approx(actual: &[f32; 16], expected: glam::Mat4) {
        for (index, (actual, expected)) in actual
            .iter()
            .zip(expected.to_cols_array().iter())
            .enumerate()
        {
            assert!(
                (actual - expected).abs() <= 1.0e-5,
                "matrix element {index} differs: actual={actual}, expected={expected}"
            );
        }
    }

    fn add_default_camera(world: &mut World) -> crate::Entity {
        let camera = world.create_entity();
        world.add_component(camera, components::Camera::default());
        world.add_component(camera, components::Transform::default());
        camera
    }
