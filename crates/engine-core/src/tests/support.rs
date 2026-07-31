struct RecordingBackend {
    uploads: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    rendered_ui_batch_counts: Option<std::sync::Arc<std::sync::Mutex<Vec<usize>>>>,
}

struct CountingRenderExtension {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl engine_renderer::RenderExtensionProducer for CountingRenderExtension {
    fn name(&self) -> &str {
        "test-counting-extension"
    }

    fn produce(&self, _input: &mut engine_renderer::RenderFrameInput, _frame_index: u64) {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl engine_renderer::BackendRenderer for RecordingBackend {
    fn begin_frame(
        &mut self,
        _input: &engine_renderer::RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
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
        input: &engine_renderer::RenderFrameInput,
        _pass: &engine_renderer::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(counts) = &self.rendered_ui_batch_counts {
            counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(input.ui_batches.len());
        }
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(
        &mut self,
        upload: MeshUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("mesh:{}", upload.mesh_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_texture(
        &mut self,
        upload: TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("texture:{}", upload.texture_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        self.uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(format!("material:{}", upload.material_id.id));
        Ok(engine_renderer::UploadReceipt::new(1))
    }
}

static FFI_WORLD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn serial_ffi_world_test() -> std::sync::MutexGuard<'static, ()> {
    FFI_WORLD_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn one_pixel_texture(id: &str, hash: u8) -> TextureUpload {
    TextureUpload {
        texture_id: AssetId::new(id),
        width: 1,
        height: 1,
        format: engine_renderer::TextureUploadFormat::Rgba8,
        color_space: engine_renderer::ColorSpace::Srgb,
        mip_levels: vec![engine_renderer::TextureMipLevel {
            width: 1,
            height: 1,
            bytes: vec![255, 255, 255, 255],
        }],
        sampler: engine_renderer::SamplerDescriptor::default(),
        content_hash: [hash; 32],
    }
}

#[test]
fn temporary_preview_texture_cannot_clobber_persistent_assets() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let preview_id = AssetId::new("editor-preview-thumbnail");

    runtime
        .register_temporary_preview_texture(one_pixel_texture(&preview_id.id, 1))
        .expect("new temporary preview registers");
    runtime
        .register_temporary_preview_texture(one_pixel_texture(&preview_id.id, 2))
        .expect("the owning preview entry may be refreshed");
    assert_eq!(
        runtime
            .asset_registry()
            .get::<TextureUpload>(&preview_id)
            .expect("preview texture")
            .get()
            .content_hash,
        [2; 32]
    );
    assert!(runtime.unregister_temporary_preview_texture(preview_id.clone()));
    assert!(!runtime.unregister_temporary_preview_texture(preview_id));

    let persistent_mesh = AssetId::new("mesh-cube");
    let diagnostics = match runtime
        .register_temporary_preview_texture(one_pixel_texture(&persistent_mesh.id, 3))
    {
        Ok(_) => panic!("temporary preview must not replace a persistent asset"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(diagnostics[0].code, "AS0003");
    assert!(runtime
        .asset_registry()
        .get::<MeshUpload>(&persistent_mesh)
        .is_some());
}

#[test]
fn persistent_replacement_revokes_temporary_preview_ownership() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let id = AssetId::new("editor-preview-replaced");
    runtime
        .register_temporary_preview_texture(one_pixel_texture(&id.id, 1))
        .expect("preview registers");

    runtime.register_texture_asset(one_pixel_texture(&id.id, 2));

    assert!(
        !runtime.unregister_temporary_preview_texture(id.clone()),
        "a stale preview owner must not unregister the persistent replacement"
    );
    assert_eq!(
        runtime
            .asset_registry()
            .get::<TextureUpload>(&id)
            .expect("persistent texture remains registered")
            .get()
            .content_hash,
        [2; 32]
    );
}

#[test]
fn direct_registry_replacement_cannot_be_clobbered_by_stale_preview_owner() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    let id = AssetId::new("editor-preview-directly-replaced");
    runtime
        .register_temporary_preview_texture(one_pixel_texture(&id.id, 1))
        .expect("preview registers");
    runtime
        .asset_registry_mut()
        .insert_typed(id.clone(), one_pixel_texture(&id.id, 2));

    let diagnostics = match runtime.register_temporary_preview_texture(one_pixel_texture(&id.id, 3))
    {
        Ok(_) => panic!("stale preview owner must not replace the current registry entry"),
        Err(diagnostics) => diagnostics,
    };
    assert_eq!(diagnostics[0].code, "AS0003");
    assert!(
        !runtime.unregister_temporary_preview_texture(id.clone()),
        "stale preview owner must not unregister a direct replacement"
    );
    assert_eq!(
        runtime
            .asset_registry()
            .get::<TextureUpload>(&id)
            .expect("replacement remains registered")
            .get()
            .content_hash,
        [2; 32]
    );
}

fn insert_empty_component(scene: &mut Scene, type_id: &str) -> String {
    let entity = scene.entities.first_mut().expect("sample scene entity");
    entity.components.insert(
        type_id.to_string(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
        },
    );
    entity.persistent_id.clone()
}

struct AOnlyComponent;

struct BOnlyComponent;

impl engine_scene::Component for AOnlyComponent {
    const TYPE_ID: &'static str = "test.a_only";
}

impl engine_scene::Component for BOnlyComponent {
    const TYPE_ID: &'static str = "test.b_only";
}

fn a_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
    Box::new(engine_scene::SparseSet::<AOnlyComponent>::new())
}

fn b_only_storage() -> Box<dyn engine_scene::ComponentStorageDyn> {
    Box::new(engine_scene::SparseSet::<BOnlyComponent>::new())
}

fn serialize_a_only(
    _component: &dyn std::any::Any,
) -> std::collections::BTreeMap<String, engine_serialize::Value> {
    std::collections::BTreeMap::new()
}

fn deserialize_a_only(
    _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Box<dyn std::any::Any> {
    Box::new(AOnlyComponent)
}

fn deserialize_b_only(
    _fields: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Box<dyn std::any::Any> {
    Box::new(BOnlyComponent)
}

fn register_a_only(registry: &mut ComponentRegistry) {
    registry
        .register(engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: <AOnlyComponent as engine_scene::Component>::TYPE_ID,
                display_name: "A Only",
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: engine_scene::ScriptAccess::ReadWrite,
            },
            storage_factory: a_only_storage,
            serialize: Some(serialize_a_only),
            deserialize: Some(deserialize_a_only),
        })
        .expect("register A-only test extension");
}

fn register_b_only(registry: &mut ComponentRegistry) {
    registry
        .register(engine_scene::ComponentExtension {
            meta: engine_scene::ComponentMeta {
                type_id: <BOnlyComponent as engine_scene::Component>::TYPE_ID,
                display_name: "B Only",
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: engine_scene::ScriptAccess::ReadWrite,
            },
            storage_factory: b_only_storage,
            serialize: Some(serialize_a_only),
            deserialize: Some(deserialize_b_only),
        })
        .expect("register B-only test extension");
}
