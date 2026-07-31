use crate::cooked_assets::tests::cook_test_material;
use engine_asset::partition::{PartitionCell, WORLD_PARTITION_SCHEMA};
use engine_asset::project::ProjectManifest;
use engine_scene::components::Transform;
use engine_scene::{sample_scene, ComponentRecord, EntityRecord};
use engine_serialize::{SchemaVersion, Value};

// ── Fixture helpers ─────────────────────────────────────────────────────

struct StreamFixture {
    project: GameProject,
    partition: WorldPartition,
    scenes: BTreeMap<String, Scene>,
}

fn bounds(center: [f32; 3], half_extents: [f32; 3]) -> CellBounds {
    CellBounds {
        center,
        half_extents,
    }
}

fn origin_bounds() -> CellBounds {
    bounds([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])
}

/// Write a project to a unique temp directory: every scene lands at
/// `assets/scenes/<scene-id>.scene.ron`, the manifest catalogs them all,
/// and `main` stays the startup scene. The partition is built in memory.
fn stream_fixture(
    name: &str,
    scenes: Vec<Scene>,
    cells: Vec<(&str, &str, CellBounds)>,
) -> StreamFixture {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "engine-cell-stream-{name}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture root");

    let mut manifest = ProjectManifest::new("Cell Stream Test");
    manifest.input_actions = None;
    manifest.scenes = scenes
        .iter()
        .map(|scene| {
            (
                scene.scene_id.clone(),
                PathBuf::from(format!("assets/scenes/{}.scene.ron", scene.scene_id)),
            )
        })
        .collect();
    assert!(
        manifest.scenes.contains_key("main"),
        "fixtures must include a \"main\" startup scene"
    );
    let manifest_path = manifest.write_to_root(&root).expect("write manifest");

    let mut scene_map = BTreeMap::new();
    for scene in scenes {
        let path = root.join(format!("assets/scenes/{}.scene.ron", scene.scene_id));
        scene.save_to_file(&path).expect("write cell scene");
        scene_map.insert(scene.scene_id.clone(), scene);
    }

    let project = GameProject {
        startup_scene: root.join(&manifest.startup_scene),
        asset_source: root.join(&manifest.asset_source),
        cooked_assets: root.join(&manifest.cooked_assets),
        manifest_path,
        root: root.clone(),
        manifest,
        script_project: None,
        script_assembly: None,
        input_actions: None,
    };
    let partition = WorldPartition {
        schema: WORLD_PARTITION_SCHEMA.to_string(),
        cells: cells
            .into_iter()
            .map(|(cell_id, scene_id, cell_bounds)| {
                (
                    cell_id.to_string(),
                    PartitionCell {
                        scene: scene_id.to_string(),
                        bounds: cell_bounds,
                    },
                )
            })
            .collect(),
    };
    StreamFixture {
        project,
        partition,
        scenes: scene_map,
    }
}

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn transform_component(translation: [f32; 3]) -> ComponentRecord {
    component(BTreeMap::from([
        ("translation".to_string(), Value::Vec3(translation)),
        ("rotation".to_string(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
        ("scale".to_string(), Value::Vec3([1.0, 1.0, 1.0])),
    ]))
}

fn renderable_component(mesh: &str, material: &str) -> ComponentRecord {
    component(BTreeMap::from([
        ("mesh".to_string(), Value::Asset(AssetId::new(mesh))),
        ("material".to_string(), Value::Asset(AssetId::new(material))),
        ("visible".to_string(), Value::Bool(true)),
        (
            "render_layer".to_string(),
            Value::Str("Default".to_string()),
        ),
        ("cast_shadows".to_string(), Value::Bool(true)),
    ]))
}

fn entity_record(
    id: &str,
    parent: Option<&str>,
    components: BTreeMap<String, ComponentRecord>,
) -> EntityRecord {
    EntityRecord {
        persistent_id: id.to_string(),
        parent: parent.map(str::to_string),
        name: Some(id.to_string()),
        enabled: true,
        components,
    }
}

fn cube_record(
    id: &str,
    parent: Option<&str>,
    translation: [f32; 3],
    material: &str,
) -> EntityRecord {
    entity_record(
        id,
        parent,
        BTreeMap::from([
            (
                "engine.transform".to_string(),
                transform_component(translation),
            ),
            (
                "engine.renderable".to_string(),
                renderable_component("mesh-cube", material),
            ),
        ]),
    )
}

/// Startup scene: a movable camera at the origin plus one static cube.
fn startup_scene() -> Scene {
    let mut scene = sample_scene();
    scene.scene_id = "main".to_string();
    scene.entities = vec![
        entity_record(
            "camera-main",
            None,
            BTreeMap::from([
                ("engine.camera".to_string(), component(BTreeMap::new())),
                (
                    "engine.transform".to_string(),
                    transform_component([0.0, 0.0, 0.0]),
                ),
            ]),
        ),
        cube_record("cube-01", None, [0.0, 0.0, 0.0], "mat-default"),
    ];
    scene.scene_settings.active_camera = Some("camera-main".to_string());
    scene.dependencies = vec![];
    scene
}

/// Cell scenes carry no camera of their own; the startup camera stays the
/// active one and streamed cameras would only become overlay cameras.
fn cell_scene(scene_id: &str, entities: Vec<EntityRecord>) -> Scene {
    let mut scene = sample_scene();
    scene.scene_id = scene_id.to_string();
    scene.entities = entities;
    scene.scene_settings.active_camera = None;
    scene.dependencies = vec![];
    scene
}

fn two_cell_fixture(name: &str) -> StreamFixture {
    stream_fixture(
        name,
        vec![
            startup_scene(),
            cell_scene(
                "level-a",
                vec![cube_record("cube-a", None, [0.0, 0.0, 0.0], "mat-default")],
            ),
            cell_scene(
                "level-b",
                vec![cube_record("cube-b", None, [5.0, 0.0, 0.0], "mat-default")],
            ),
        ],
        vec![
            ("cell-a", "level-a", origin_bounds()),
            (
                "cell-b",
                "level-b",
                bounds([5.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            ),
        ],
    )
}

fn running_driver(
    fixture: &StreamFixture,
    config: CellStreamingConfig,
) -> (EngineRuntime, CellStreamingDriver) {
    let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
    runtime
        .load_scene(fixture.scenes["main"].clone())
        .expect("startup scene loads");
    let mut driver = match CellStreamingDriver::new(&fixture.partition, &fixture.project, config) {
        Ok(driver) => driver,
        Err(error) => panic!("driver construction failed: {error}"),
    };
    driver.rebaseline(&runtime);
    (runtime, driver)
}

fn set_camera_position(runtime: &EngineRuntime, position: Vec3) {
    runtime.with_world_mut(|world| {
        let camera = world
            .entity_by_persistent_id("camera-main")
            .expect("camera entity");
        world
            .get_mut::<Transform>(camera)
            .expect("camera transform")
            .translation = position;
    });
}

fn has_entity(runtime: &EngineRuntime, id: &str) -> bool {
    runtime
        .with_world(|world| world.entity_by_persistent_id(id).is_some())
        .unwrap_or(false)
}

/// Pure in-memory partition for validation tests.
fn partition_of(cells: &[(&str, &str)]) -> WorldPartition {
    WorldPartition {
        schema: WORLD_PARTITION_SCHEMA.to_string(),
        cells: cells
            .iter()
            .map(|(cell_id, scene_id)| {
                (
                    cell_id.to_string(),
                    PartitionCell {
                        scene: scene_id.to_string(),
                        bounds: origin_bounds(),
                    },
                )
            })
            .collect(),
    }
}
