fn valid_mesh_upload() -> MeshUpload {
    MeshUpload {
        mesh_id: AssetId::new("mesh.triangle"),
        vertex_format: MeshVertexFormat::Pbr32,
        vertex_count: 3,
        vertex_bytes: vec![0; 3 * 32],
        index_format: IndexFormat::U16,
        index_count: 3,
        index_bytes: vec![0, 0, 1, 0, 2, 0],
        bounds: AxisAlignedBox::UNIT,
        content_hash: [1; 32],
    }
}

fn valid_texture_upload() -> TextureUpload {
    TextureUpload {
        texture_id: AssetId::new("texture.checker"),
        width: 2,
        height: 2,
        format: TextureUploadFormat::Rgba8,
        color_space: super::ColorSpace::Srgb,
        mip_levels: vec![
            TextureMipLevel {
                width: 2,
                height: 2,
                bytes: vec![255; 16],
            },
            TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            },
        ],
        sampler: SamplerDescriptor::default(),
        content_hash: [2; 32],
    }
}

fn valid_material_upload() -> MaterialUpload {
    MaterialUpload {
        material_id: AssetId::new("material.default"),
        base_color: [1.0, 1.0, 1.0, 1.0],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [0.0; 3],
        base_color_texture: Some(AssetId::new("texture.checker")),
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: super::AdvancedMaterialParameters::default(),
        transparency: Transparency::Opaque,
        double_sided: false,
        content_hash: [3; 32],
    }
}

#[test]
fn no_backend_fails_closed_for_draw_resize_uploads_and_removal() {
    let mut renderer = Renderer::new();
    let draw = renderer.draw_scene(&valid_frame()).unwrap_err();
    assert!(draw.iter().any(|d| d.code == DIAG_BACKEND_MISSING));

    for diagnostics in [
        renderer.resize(1280, 720).unwrap_err(),
        renderer.upload_mesh(valid_mesh_upload()).unwrap_err(),
        renderer.upload_texture(valid_texture_upload()).unwrap_err(),
        renderer
            .upload_material(valid_material_upload())
            .unwrap_err(),
        renderer
            .remove_resource(ResourceRemoval {
                kind: ResourceKind::Mesh,
                resource_id: AssetId::new("mesh.triangle"),
            })
            .unwrap_err(),
    ] {
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DIAG_BACKEND_MISSING));
    }
}

#[derive(Default)]
struct UnsupportedBackend;

impl BackendRenderer for UnsupportedBackend {}

#[test]
fn backend_default_uploads_report_stable_unsupported_diagnostics() {
    let mut renderer = Renderer::new_with_backend(Box::<UnsupportedBackend>::default());
    assert!(renderer
        .upload_mesh(valid_mesh_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_MESH_UPLOAD_UNSUPPORTED));
    assert!(renderer
        .upload_texture(valid_texture_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_TEXTURE_UPLOAD_UNSUPPORTED));
    assert!(renderer
        .upload_material(valid_material_upload())
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_MATERIAL_UPLOAD_UNSUPPORTED));
}

#[test]
fn backend_default_barriers_fail_closed_when_work_is_required() {
    use crate::render_graph2::{CompiledBarrier, PassKind, PassNode, PipeStage, ResourceState};

    let mut backend = UnsupportedBackend;
    let input = valid_frame();
    let pass = PassNode {
        kind: PassKind::ToneMap,
        name: "tone_map_pass",
        view_id: 0,
        inputs: Vec::new(),
        outputs: Vec::new(),
        depth_stencil: None,
    };
    let barrier = CompiledBarrier {
        resource_name: "hdr_color".into(),
        src_stage: PipeStage::ColorAttachmentOutput,
        dst_stage: PipeStage::FragmentShader,
        old_state: ResourceState::ColorAttachmentOptimal,
        new_state: ResourceState::ShaderReadOnlyOptimal,
    };

    assert!(backend.apply_pass_barriers(&input, &pass, &[]).is_ok());
    let diagnostics = backend
        .apply_pass_barriers(&input, &pass, &[barrier])
        .expect_err("non-empty barriers must not be dropped");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_BARRIERS_UNSUPPORTED));
}

#[test]
fn backend_default_rejects_undeclared_custom_graph_passes() {
    let mut input = valid_frame();
    input.render_options.pass_graph_config.passes.insert(
        2,
        crate::PassConfigEntry {
            kind: "custom_bloom".into(),
            enabled: true,
        },
    );
    let mut graph = crate::render_graph2::RenderGraph::build_with_config(
        &input,
        &input.render_options.pass_graph_config,
    );
    let diagnostics = UnsupportedBackend
        .configure_render_graph(&input, &mut graph)
        .expect_err("custom resource declarations must not be silently empty");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_CUSTOM_RENDER_GRAPH_UNSUPPORTED));
}

struct CountingBackend {
    upload_calls: Arc<AtomicUsize>,
}

impl BackendRenderer for CountingBackend {
    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.upload_calls.fetch_add(1, Ordering::SeqCst);
        Ok(UploadReceipt::new(1))
    }
}

#[test]
fn invalid_uploads_never_enter_the_backend() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(CountingBackend {
        upload_calls: Arc::clone(&calls),
    }));

    let mut mesh = valid_mesh_upload();
    mesh.vertex_bytes.pop();
    let mesh_errors = renderer.upload_mesh(mesh).unwrap_err();
    assert!(mesh_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MESH_VERTICES));

    let mut texture = valid_texture_upload();
    texture.mip_levels[1].bytes.pop();
    let texture_errors = renderer.upload_texture(texture).unwrap_err();
    assert!(texture_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_TEXTURE_MIPS));

    let mut material = valid_material_upload();
    material.roughness = f32::NAN;
    let material_errors = renderer.upload_material(material).unwrap_err();
    assert!(material_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MATERIAL_VALUES));

    let mut emissive_material = valid_material_upload();
    emissive_material.emissive[1] = 1.1;
    let emissive_errors = renderer.upload_material(emissive_material).unwrap_err();
    assert!(emissive_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MATERIAL_VALUES));

    let mut advanced_material = valid_material_upload();
    advanced_material.advanced.anisotropy = 1.1;
    let advanced_errors = renderer.upload_material(advanced_material).unwrap_err();
    assert!(advanced_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MATERIAL_VALUES));

    let malformed_morph = super::MorphTargetSetUpload {
        target_set_id: AssetId::new("morph.face"),
        vertex_count: 2,
        targets: vec![super::MorphTarget {
            name: "smile".into(),
            position_deltas: vec![[0.0; 3]],
            normal_deltas: vec![[0.0; 3]],
        }],
        content_hash: [9; 32],
    };
    let morph_errors = renderer
        .upload_morph_target_set(malformed_morph)
        .unwrap_err();
    assert!(morph_errors
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_INVALID_MORPH_TARGET_SET));

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
