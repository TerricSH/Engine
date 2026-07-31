#[cfg(feature = "subsystem-terrain")]
struct PlanetaryLensCaptureBackend {
    captured: std::sync::Arc<std::sync::Mutex<Vec<engine_renderer::PlanetaryLensSettings>>>,
}

#[cfg(feature = "subsystem-terrain")]
impl engine_renderer::BackendRenderer for PlanetaryLensCaptureBackend {
    fn begin_frame(
        &mut self,
        input: &engine_renderer::RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(input.render_options.post_process.planetary_lens);
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }
}

#[cfg(feature = "subsystem-terrain")]
fn planetary_lens_test_world() -> engine_scene::World {
    use engine_scene::components::{Camera, Transform};
    use engine_terrain::{TerrainTopology, TerrainVolume};

    let mut world = engine_scene::World::new();
    let camera = world
        .create_persistent_entity("camera-main")
        .expect("camera persistent id");
    world.add_component(
        camera,
        Transform {
            translation: glam::Vec3::new(0.0, 0.0, 1_200.0),
            ..Transform::default()
        },
    );
    world.add_component(camera, Camera::default());

    let planet = world
        .create_persistent_entity("planet")
        .expect("planet persistent id");
    world.add_component(
        planet,
        TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            height_scale: 0.0,
            planet_radius: 1_000.0,
            ..TerrainVolume::default()
        },
    );

    let settings = world.scene_settings_mut();
    settings.active_camera = Some("camera-main".to_string());
    settings.post_process.planetary_lens = engine_renderer::PlanetaryLensSettings {
        enabled: true,
        mode: engine_renderer::PlanetaryLensMode::CameraAltitude,
        altitude_fade_start: 100.0,
        altitude_fade_end: 300.0,
        barrel_distortion: 0.12,
        horizon_curvature: 0.08,
        atmosphere_intensity: 0.4,
        chromatic_aberration: 0.006,
    };
    world
}

#[cfg(feature = "subsystem-terrain")]
#[test]
fn render_extraction_resolves_planetary_lens_from_cube_sphere_altitude() {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(PlanetaryLensCaptureBackend {
        captured: std::sync::Arc::clone(&captured),
    }));
    runtime.set_world(planetary_lens_test_world());

    runtime
        .render_frame(0)
        .expect("automatic planetary lens frame");
    runtime
        .with_world_mut(|world| {
            world.scene_settings_mut().post_process.planetary_lens.mode =
                engine_renderer::PlanetaryLensMode::Manual;
        })
        .expect("world remains installed");
    runtime
        .render_frame(1)
        .expect("manual planetary lens frame");
    runtime
        .with_world_mut(|world| {
            for (_, terrain) in world.query_mut::<engine_terrain::TerrainVolume>() {
                terrain.enabled = false;
            }
            world.scene_settings_mut().post_process.planetary_lens.mode =
                engine_renderer::PlanetaryLensMode::CameraAltitude;
        })
        .expect("world remains installed");
    runtime
        .render_frame(2)
        .expect("automatic lens without a planet");
    runtime
        .with_world_mut(|world| {
            for (_, terrain) in world.query_mut::<engine_terrain::TerrainVolume>() {
                terrain.enabled = true;
            }
            let second_planet = world
                .create_persistent_entity("planet-two")
                .expect("second planet persistent id");
            world.add_component(
                second_planet,
                engine_terrain::TerrainVolume {
                    topology: engine_terrain::TerrainTopology::CubeSphere,
                    height_scale: 0.0,
                    // Camera is 150 m inside this surface. Absolute surface
                    // distance makes it nearer than the original 200 m
                    // altitude and keeps automatic selection deterministic.
                    planet_radius: 1_350.0,
                    ..engine_terrain::TerrainVolume::default()
                },
            );
        })
        .expect("world remains installed");
    runtime
        .render_frame(3)
        .expect("automatic lens with multiple planets");

    let frames = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(frames.len(), 4);

    let automatic = frames[0];
    assert!(automatic.enabled);
    assert_eq!(
        automatic.mode,
        engine_renderer::PlanetaryLensMode::CameraAltitude
    );
    assert!((automatic.barrel_distortion - 0.06).abs() < f32::EPSILON);
    assert!((automatic.horizon_curvature - 0.04).abs() < f32::EPSILON);
    assert!((automatic.atmosphere_intensity - 0.2).abs() < f32::EPSILON);
    assert!((automatic.chromatic_aberration - 0.003).abs() < f32::EPSILON);

    let manual = frames[1];
    assert!(manual.enabled);
    assert_eq!(manual.mode, engine_renderer::PlanetaryLensMode::Manual);
    assert!((manual.barrel_distortion - 0.12).abs() < f32::EPSILON);
    assert!((manual.horizon_curvature - 0.08).abs() < f32::EPSILON);
    assert!((manual.atmosphere_intensity - 0.4).abs() < f32::EPSILON);
    assert!((manual.chromatic_aberration - 0.006).abs() < f32::EPSILON);

    let missing_planet = frames[2];
    assert!(!missing_planet.enabled);
    assert_eq!(missing_planet.barrel_distortion, 0.0);
    assert_eq!(missing_planet.horizon_curvature, 0.0);
    assert_eq!(missing_planet.atmosphere_intensity, 0.0);
    assert_eq!(missing_planet.chromatic_aberration, 0.0);

    let nearest_planet = frames[3];
    assert!(nearest_planet.enabled);
    assert!((nearest_planet.barrel_distortion - 0.01875).abs() < f32::EPSILON);
}
