fn scenario_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Deterministic PRNG (xorshift64*) for fixture derivation. Nothing in the
/// scenario consumes wall-clock or thread-timing randomness; every varying
/// detail comes from this generator seeded with [`SOAK_SEED`].
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish f32 in `[min, max)` with 24-bit granularity.
    fn next_range(&mut self, min: f32, max: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        min + (max - min) * unit
    }
}

/// Camera logical X position: triangle wave between 0 and [`PATROL_RANGE`].
fn patrol_position(frame: u64) -> f32 {
    let distance = (frame % PATROL_PERIOD_FRAMES) as f32 * PATROL_SPEED;
    if distance <= PATROL_RANGE {
        distance
    } else {
        2.0 * PATROL_RANGE - distance
    }
}

// Headless backend.

/// Minimal headless backend mirroring the QA backend semantics: validates
/// frame ordering, counts forward-pass draw calls and triangles, and accepts
/// every resource upload without owning GPU objects.
#[derive(Default)]
struct SoakBackend {
    frame_active: bool,
    mesh_triangles: BTreeMap<AssetId, u64>,
}

impl SoakBackend {
    fn error(code: &'static str, message: impl Into<String>) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            code,
            DiagnosticSeverity::Error,
            "sandbox.soak",
            message.into(),
        )]
    }
}

impl BackendRenderer for SoakBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        if self.frame_active {
            return Err(Self::error("SOAK0001", "frame already active"));
        }
        self.frame_active = true;
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &engine_renderer::render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(Self::error("SOAK0002", "render pass outside a frame"));
        }
        if pass.kind == engine_renderer::render_graph2::PassKind::OpaquePbrForward {
            let meshes = input
                .drawables
                .iter()
                .map(|item| &item.mesh)
                .chain(input.skinned_items.iter().map(|item| &item.mesh));
            let mut draw_calls = 0u32;
            for mesh in meshes {
                draw_calls = draw_calls.saturating_add(1);
                stats.triangles = stats
                    .triangles
                    .saturating_add(self.mesh_triangles.get(mesh).copied().unwrap_or(0));
            }
            stats.draw_calls = stats.draw_calls.saturating_add(draw_calls);
            stats.visible_drawables = draw_calls;
        }
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(Self::error("SOAK0003", "ending an inactive frame"));
        }
        self.frame_active = false;
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.frame_active = false;
        Ok(())
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.mesh_triangles
            .insert(upload.mesh_id, u64::from(upload.index_count / 3));
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }
}

// Fixture.

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

fn entity(id: &str, components: Vec<(&str, ComponentRecord)>) -> EntityRecord {
    EntityRecord {
        persistent_id: id.to_string(),
        parent: None,
        name: Some(id.to_string()),
        enabled: true,
        components: components
            .into_iter()
            .map(|(type_id, record)| (type_id.to_string(), record))
            .collect(),
    }
}

/// Write a `MaterialSource-v0` next to the cooked output and cook it, so the
/// cell's only runtime copy must arrive through the background asset stream.
fn cook_cell_material(cooked_dir: &Path, id: &str, rng: &mut XorShift64) {
    let color = [
        rng.next_range(0.2, 1.0),
        rng.next_range(0.2, 1.0),
        rng.next_range(0.2, 1.0),
        1.0,
    ];
    let source = cooked_dir.join(format!("{id}.material.json"));
    std::fs::write(
        &source,
        format!(
            "{{\n  \"schema\": \"MaterialSource-v0\",\n  \"base_color\": [{}, {}, {}, {}],\n  \"metallic\": 0.25,\n  \"roughness\": 0.5,\n  \"ambient_occlusion\": 1.0,\n  \"transparency\": \"Opaque\",\n  \"double_sided\": false\n}}\n",
            color[0], color[1], color[2], color[3]
        ),
    )
    .expect("write material source");
    cook_material(&source, &cooked_dir.join(format!("{id}.cooked"))).expect("cook cell material");
}

struct SoakFixture {
    _tempdir: tempfile::TempDir,
    project: GameProject,
    partition: WorldPartition,
    material_ids: Vec<AssetId>,
}

/// Build the soak project: an origin-shifting startup scene with a movable
/// camera, plus a chain of streamed cells along the patrol path whose unique
/// materials exist only as cooked artifacts on disk.
fn build_fixture(seed: u64) -> SoakFixture {
    let mut rng = XorShift64::new(seed);
    let tempdir = tempfile::tempdir().expect("soak fixture tempdir");
    let root = tempdir.path().to_path_buf();
    let scene_dir = root.join("assets/scenes");
    let cooked_dir = root.join("build/cooked");
    std::fs::create_dir_all(&scene_dir).expect("scene directory");
    std::fs::create_dir_all(root.join("assets/source")).expect("source directory");
    std::fs::create_dir_all(&cooked_dir).expect("cooked directory");

    let mut main = engine_scene::sample_scene();
    main.scene_id = "main".to_string();
    main.name = "Soak Main".to_string();
    main.entities = vec![
        entity(
            "camera-main",
            vec![
                ("engine.camera", component(BTreeMap::new())),
                ("engine.transform", transform_component([0.0, 0.0, 0.0])),
            ],
        ),
        entity(
            "cube-home",
            vec![
                ("engine.transform", transform_component([0.0, 0.0, -5.0])),
                (
                    "engine.renderable",
                    renderable_component("mesh-cube", "mat-default"),
                ),
            ],
        ),
    ];
    main.scene_settings.active_camera = Some("camera-main".to_string());
    main.scene_settings.origin_shift.enabled = true;
    main.scene_settings.origin_shift.threshold = ORIGIN_SHIFT_THRESHOLD;
    main.dependencies = vec![];
    main.save_to_file(&scene_dir.join("main.scene.ron"))
        .expect("write startup scene");

    let mut manifest_scenes = BTreeMap::from([(
        "main".to_string(),
        PathBuf::from("assets/scenes/main.scene.ron"),
    )]);
    let mut cells = BTreeMap::new();
    let mut material_ids = Vec::new();

    for index in 0..CELL_COUNT {
        let center = CELL_HALF_EXTENT + index as f32 * CELL_SPACING;
        let material_id = format!("mat-soak-{index}");
        cook_cell_material(&cooked_dir, &material_id, &mut rng);

        let scene_id = format!("cell-{index}");
        let mut scene = engine_scene::sample_scene();
        scene.scene_id = scene_id.clone();
        scene.name = format!("Soak Cell {index}");
        scene.scene_settings.active_camera = None;
        scene.entities = (0..CELL_ENTITIES)
            .map(|cube| {
                let x_offset = rng.next_range(-16.0, 16.0);
                let y = rng.next_range(-2.0, 2.0);
                let z = rng.next_range(-8.0, -4.0);
                entity(
                    &format!("soak-cube-{index}-{cube}"),
                    vec![
                        (
                            "engine.transform",
                            transform_component([center + x_offset, y, z]),
                        ),
                        (
                            "engine.renderable",
                            renderable_component("mesh-cube", &material_id),
                        ),
                    ],
                )
            })
            .collect();
        scene.dependencies = vec![];
        let relative = format!("assets/scenes/{scene_id}.scene.ron");
        scene
            .save_to_file(&root.join(&relative))
            .expect("write cell scene");
        manifest_scenes.insert(scene_id.clone(), PathBuf::from(relative));
        cells.insert(
            format!("cell_{index}"),
            PartitionCell {
                scene: scene_id,
                bounds: CellBounds {
                    center: [center, 0.0, 0.0],
                    half_extents: [CELL_HALF_EXTENT, 10.0, 10.0],
                },
            },
        );
        material_ids.push(AssetId::new(&material_id));
    }

    let mut manifest = ProjectManifest::new("Soak Harness");
    manifest.startup_scene = PathBuf::from("main");
    manifest.input_actions = None;
    manifest.scenes = manifest_scenes;
    manifest
        .write_to_root(&root)
        .expect("write project manifest");
    let project = GameProject::load(&root).expect("load soak fixture project");

    let partition = WorldPartition {
        schema: WORLD_PARTITION_SCHEMA.to_string(),
        cells,
    };
    SoakFixture {
        _tempdir: tempdir,
        project,
        partition,
        material_ids,
    }
}

// Working-set sampling.
