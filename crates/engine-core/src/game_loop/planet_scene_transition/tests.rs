use engine_scene::components::Transform;
use engine_terrain::{PlanetSceneBand, TerrainTopology};
use glam::Vec3;

use super::*;
use crate::{game_loop::GameLoop, EngineConfig};

fn test_loop(scene_id: &str) -> GameLoop {
    let mut scene = engine_scene::sample_scene();
    scene.scene_id = scene_id.into();
    let mut game_loop = GameLoop::new(EngineConfig::default());
    game_loop.load_scene(scene).unwrap();
    game_loop
}

fn planet(center: [f64; 3]) -> TerrainVolume {
    TerrainVolume {
        enabled: true,
        topology: TerrainTopology::CubeSphere,
        base_resolution: 3,
        height_scale: 0.0,
        planet_center: center,
        planet_radius: 1_000.0,
        planet_max_lod: 0,
        lod_distances: vec![1_000.0],
        ..TerrainVolume::default()
    }
}

fn transition(
    terrain_volume_id: &str,
    surface_scene_id: &str,
    enter: f64,
    exit: f64,
    dwell: f64,
) -> PlanetSceneTransitionConfig {
    PlanetSceneTransitionConfig {
        enabled: true,
        terrain_volume_id: terrain_volume_id.into(),
        orbit_scene_id: "orbit".into(),
        surface_scene_id: surface_scene_id.into(),
        enter_surface_altitude: enter,
        exit_surface_altitude: exit,
        minimum_dwell_seconds: dwell,
    }
}

fn add_planet(
    game_loop: &mut GameLoop,
    terrain_id: &str,
    volume: TerrainVolume,
    controller_id: &str,
    config: PlanetSceneTransitionConfig,
) {
    game_loop
        .runtime
        .with_world_mut(|world| {
            let terrain_entity = world.create_persistent_entity(terrain_id).unwrap();
            world.add_component(terrain_entity, volume);
            let controller_entity = world.create_persistent_entity(controller_id).unwrap();
            world.add_component(controller_entity, config);
        })
        .unwrap();
}

fn set_camera(game_loop: &mut GameLoop, position: [f32; 3], origin: [f64; 3]) {
    game_loop
        .runtime
        .with_world_mut(|world| {
            world.restore_world_origin(origin).unwrap();
            let camera = world.entity_by_persistent_id("camera-main").unwrap();
            if let Some(transform) = world.get_mut::<Transform>(camera) {
                transform.translation = Vec3::from(position);
            } else {
                world.add_component(
                    camera,
                    Transform {
                        translation: Vec3::from(position),
                        ..Transform::default()
                    },
                );
            }
        })
        .unwrap();
}

#[test]
fn main_update_requests_surface_scene_below_authored_altitude() {
    let mut game_loop = test_loop("orbit");
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-a-transition",
        transition("planet-a", "surface-a", 150.0, 250.0, 0.0),
    );
    set_camera(&mut game_loop, [0.0, 1_100.0, 0.0], [0.0; 3]);

    game_loop.update(1.0 / 60.0);
    let ticket = game_loop
        .take_pending_planet_scene_transition()
        .expect("surface transition");
    assert_eq!(ticket.controller_id, "planet-a-transition");
    assert_eq!(ticket.terrain_volume_id, "planet-a");
    assert_eq!(ticket.request.from, PlanetSceneBand::Orbit);
    assert_eq!(ticket.request.to, PlanetSceneBand::Surface);
    assert_eq!(ticket.request.scene_id, "surface-a");
}

#[test]
fn committed_surface_policy_survives_scene_replacement_and_returns_to_orbit() {
    let mut game_loop = test_loop("orbit");
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-a-transition",
        transition("planet-a", "surface-a", 150.0, 250.0, 0.0),
    );
    set_camera(&mut game_loop, [0.0, 1_100.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(0.0);
    let landing = game_loop.take_pending_planet_scene_transition().unwrap();

    let mut surface = engine_scene::sample_scene();
    surface.scene_id = "surface-a".into();
    game_loop.load_scene(surface).unwrap();
    game_loop.commit_planet_scene_transition(&landing).unwrap();
    set_camera(&mut game_loop, [0.0, 1_300.0, 0.0], [0.0; 3]);

    game_loop.tick_planet_scene_transitions(0.0);
    let ascent = game_loop
        .take_pending_planet_scene_transition()
        .expect("orbit transition");
    assert_eq!(ascent.request.from, PlanetSceneBand::Surface);
    assert_eq!(ascent.request.to, PlanetSceneBand::Orbit);
    assert_eq!(ascent.request.scene_id, "orbit");
}

#[test]
fn dwell_and_hysteresis_prevent_boundary_jitter() {
    let mut game_loop = test_loop("orbit");
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-a-transition",
        transition("planet-a", "surface-a", 150.0, 250.0, 1.0),
    );
    set_camera(&mut game_loop, [0.0, 1_149.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(0.5);
    assert!(game_loop.take_pending_planet_scene_transition().is_none());
    set_camera(&mut game_loop, [0.0, 1_151.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(0.49);
    assert!(game_loop.take_pending_planet_scene_transition().is_none());
    set_camera(&mut game_loop, [0.0, 1_149.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(0.01);
    let landing = game_loop
        .take_pending_planet_scene_transition()
        .expect("dwell elapsed");

    let mut surface = engine_scene::sample_scene();
    surface.scene_id = "surface-a".into();
    game_loop.load_scene(surface).unwrap();
    game_loop.commit_planet_scene_transition(&landing).unwrap();
    for altitude in [149.0, 151.0, 148.0, 152.0] {
        set_camera(
            &mut game_loop,
            [0.0, 1_000.0 + altitude as f32, 0.0],
            [0.0; 3],
        );
        game_loop.tick_planet_scene_transitions(0.5);
        assert!(game_loop.take_pending_planet_scene_transition().is_none());
    }
}

#[test]
fn explicit_targets_choose_the_nearest_configured_planet_in_f64_space() {
    let mut game_loop = test_loop("orbit");
    let origin = [1.0e12, 0.0, 0.0];
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([origin[0] - 10_000.0, 0.0, 0.0]),
        "planet-a-transition",
        transition("planet-a", "surface-a", 20_000.0, 30_000.0, 0.0),
    );
    add_planet(
        &mut game_loop,
        "planet-b",
        planet(origin),
        "planet-b-transition",
        transition("planet-b", "surface-b", 20_000.0, 30_000.0, 0.0),
    );
    set_camera(&mut game_loop, [0.0, 1_100.0, 0.0], origin);

    game_loop.tick_planet_scene_transitions(0.0);
    let ticket = game_loop
        .take_pending_planet_scene_transition()
        .expect("nearest planet transition");
    assert_eq!(ticket.controller_id, "planet-b-transition");
    assert_eq!(ticket.terrain_volume_id, "planet-b");
    assert_eq!(ticket.request.scene_id, "surface-b");
}

#[test]
fn omitted_target_is_fail_closed_when_multiple_planets_are_valid() {
    let mut game_loop = test_loop("orbit");
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-transition",
        transition("", "surface-a", 150.0, 250.0, 0.0),
    );
    game_loop
        .runtime
        .with_world_mut(|world| {
            let terrain = world.create_persistent_entity("planet-b").unwrap();
            world.add_component(terrain, planet([10_000.0, 0.0, 0.0]));
        })
        .unwrap();
    set_camera(&mut game_loop, [0.0, 1_100.0, 0.0], [0.0; 3]);

    game_loop.tick_planet_scene_transitions(0.0);
    assert!(game_loop.take_pending_planet_scene_transition().is_none());
    assert!(game_loop
        .runtime
        .diagnostics_collector()
        .all()
        .iter()
        .any(|diagnostic| {
            diagnostic.code == "PLANET_SCENE_TRANSITION"
                && diagnostic.message.contains("set an explicit persistent ID")
        }));
}

#[test]
fn absent_or_default_disabled_configuration_never_switches_scenes() {
    let mut game_loop = test_loop("orbit");
    set_camera(&mut game_loop, [0.0, 1_000.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(100.0);
    assert!(game_loop.take_pending_planet_scene_transition().is_none());

    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-transition",
        PlanetSceneTransitionConfig::default(),
    );
    game_loop.tick_planet_scene_transitions(100.0);
    assert!(game_loop.take_pending_planet_scene_transition().is_none());
}

#[test]
fn late_ticket_cannot_acknowledge_a_rebuilt_controller_with_reused_serial() {
    let mut game_loop = test_loop("orbit");
    add_planet(
        &mut game_loop,
        "planet-a",
        planet([0.0; 3]),
        "planet-transition",
        transition("planet-a", "surface-a", 150.0, 250.0, 0.0),
    );
    set_camera(&mut game_loop, [0.0, 1_100.0, 0.0], [0.0; 3]);
    game_loop.tick_planet_scene_transitions(0.0);
    let stale = game_loop.take_pending_planet_scene_transition().unwrap();
    game_loop.reject_planet_scene_transition(&stale).unwrap();

    game_loop
        .runtime
        .with_world_mut(|world| {
            let controller = world.entity_by_persistent_id("planet-transition").unwrap();
            let config = world
                .get_mut::<PlanetSceneTransitionConfig>(controller)
                .unwrap();
            config.enter_surface_altitude = 160.0;
            config.exit_surface_altitude = 260.0;
        })
        .unwrap();
    game_loop.tick_planet_scene_transitions(0.0);
    let current = game_loop.take_pending_planet_scene_transition().unwrap();
    assert_eq!(stale.request.serial, current.request.serial);
    assert_ne!(stale.transaction_id, current.transaction_id);

    assert_eq!(
        game_loop.commit_planet_scene_transition(&stale),
        Err(PlanetSceneTransitionError::RequestMismatch)
    );
    game_loop
        .commit_planet_scene_transition(&current)
        .expect("current transaction remains pending");
}
