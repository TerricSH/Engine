fn surface_upload(
    transparency: engine_renderer::Transparency,
    double_sided: bool,
) -> MaterialUpload {
    MaterialUpload {
        material_id: AssetId::new("material.surface"),
        base_color: [0.2, 0.3, 0.4, 0.6],
        metallic: 0.1,
        roughness: 0.7,
        ambient_occlusion: 0.8,
        emissive: [0.05, 0.1, 0.15],
        base_color_texture: None,
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency,
        double_sided,
        content_hash: [7; 32],
    }
}

#[test]
fn uploaded_material_binding_preserves_surface_state_and_mask_cutoff() {
    let binding = uploaded_material_binding(&surface_upload(
        engine_renderer::Transparency::Masked { cutoff: 0.37 },
        true,
    ));
    assert_eq!(
        binding.transparency,
        engine_renderer::Transparency::Masked { cutoff: 0.37 }
    );
    assert!(binding.double_sided);
    assert_eq!(
        SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).alpha_cutoff,
        0.37
    );
    assert_eq!(
        SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).emissive,
        [0.05, 0.1, 0.15, 0.0]
    );
    assert_eq!(binding.uniforms.bytes.len(), MATERIAL_UBO_SIZE);

    let blended =
        uploaded_material_binding(&surface_upload(engine_renderer::Transparency::Blend, false));
    assert_eq!(
        SceneRenderer::parse_material_ubo(&blended.uniforms.bytes).alpha_cutoff,
        -1.0
    );
}

#[test]
fn particle_batches_pack_one_fixed_stride_instance_stream() {
    let batch = ParticleBatch {
        emitter: None,
        mesh: AssetId::new("mesh.quad"),
        material: AssetId::new("material.particle"),
        instances: vec![
            engine_renderer::ParticleInstance {
                position: [1.0, 2.0, 3.0],
                size: 4.0,
                rotation_radians: 0.5,
                normalized_age: 0.25,
                color: [255, 128, 64, 32],
            },
            engine_renderer::ParticleInstance {
                position: [-1.0, -2.0, -3.0],
                size: 2.0,
                rotation_radians: 1.0,
                normalized_age: 0.75,
                color: [10, 20, 30, 40],
            },
        ],
        gpu_simulation: None,
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "Transparent".into(),
        sort_key: 0,
    };
    let prepared = prepare_particle_instances(&[batch]).unwrap();
    assert_eq!(
        prepared.instance_bytes.len(),
        2 * VFX_INSTANCE_STRIDE as usize
    );
    assert_eq!(
        prepared.draws,
        vec![PreparedParticleDraw {
            batch_index: 0,
            first_instance: 0,
            instance_count: 2,
        }]
    );
    assert_eq!(&prepared.instance_bytes[24..28], &[255, 128, 64, 32]);
    assert_eq!(&prepared.instance_bytes[56..60], &[10, 20, 30, 40]);
}

#[test]
fn particle_vertex_shader_uses_instance_rate_billboards() {
    let source = include_str!("../../shaders/vfx_billboard.vert");
    assert!(source.contains("i_position_size"));
    assert!(source.contains("unpackUnorm4x8"));
    assert!(source.contains("camera_right"));
    assert!(source.contains("gl_Position = ubo.view_proj"));
}

#[test]
fn consecutive_static_surfaces_are_packed_into_one_instance_batch() {
    let make_drawable = |x: f32, material: &str| {
        let mut world = engine_renderer::IDENTITY_MAT4;
        world[12] = x;
        RenderableItem {
            entity: None,
            mesh: AssetId::new("mesh.tree"),
            material: AssetId::new(material),
            world_transform: world,
            bounds: engine_renderer::AxisAlignedBox::UNIT,
            render_layer: "Default".into(),
            cast_shadows: true,
            sort_key: 0,
            radial_vertex_morph: None,
            triplanar_material_mapping: None,
        }
    };
    let first = make_drawable(1.0, "material.leaves");
    let second = make_drawable(2.0, "material.leaves");
    let single = make_drawable(3.0, "material.other");
    let prepared = prepare_static_instances(&[&first, &second, &single]).unwrap();
    assert_eq!(
        prepared.instance_bytes.len(),
        2 * STATIC_INSTANCE_STRIDE as usize
    );
    assert_eq!(
        prepared.draws,
        vec![PreparedStaticDraw {
            first_drawable: 0,
            drawable_count: 2,
            first_instance: 0,
            instance_count: 2,
        }]
    );
}

#[test]
fn radial_geomorph_surfaces_are_not_static_instanced() {
    let mut drawable = RenderableItem {
        entity: None,
        mesh: AssetId::new("mesh.planet"),
        material: AssetId::new("material.planet"),
        world_transform: engine_renderer::IDENTITY_MAT4,
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "default".into(),
        cast_shadows: true,
        sort_key: 0,
        radial_vertex_morph: Some(engine_renderer::RadialVertexMorph {
            factor: 0.5,
            delta_scale: 10.0,
            local_origin: [0.0; 3],
        }),
        triplanar_material_mapping: None,
    };
    let first = drawable.clone();
    drawable.world_transform[12] = 1.0;
    let prepared = prepare_static_instances(&[&first, &drawable]).unwrap();
    assert!(prepared.draws.is_empty());
    assert!(prepared.instance_bytes.is_empty());
}

#[test]
fn triplanar_surfaces_pack_per_draw_projection_and_skip_static_instancing() {
    let mut drawable = RenderableItem {
        entity: None,
        mesh: AssetId::new("mesh.terrain"),
        material: AssetId::new("material.terrain"),
        world_transform: engine_renderer::IDENTITY_MAT4,
        bounds: engine_renderer::AxisAlignedBox::UNIT,
        render_layer: "default".into(),
        cast_shadows: true,
        sort_key: 0,
        radial_vertex_morph: None,
        triplanar_material_mapping: Some(engine_renderer::TriplanarMaterialMapping {
            local_origin: [10.0, -20.0, 30.0],
            meters_per_tile: 8.0,
            blend_sharpness: 6.0,
        }),
    };
    let constants = static_draw_push_constants(&drawable);
    let read_f32 =
        |offset: usize| f32::from_ne_bytes(constants[offset..offset + 4].try_into().unwrap());
    assert_eq!(read_f32(96), 1.0);
    assert_eq!(read_f32(100), 0.125);
    assert_eq!(read_f32(104), 6.0);
    assert_eq!(read_f32(112), 10.0);
    assert_eq!(read_f32(116), -20.0);
    assert_eq!(read_f32(120), 30.0);

    let first = drawable.clone();
    drawable.world_transform[12] = 1.0;
    let prepared = prepare_static_instances(&[&first, &drawable]).unwrap();
    assert!(prepared.draws.is_empty());
    assert!(prepared.instance_bytes.is_empty());
}

#[test]
fn instanced_vertex_shader_consumes_four_model_columns() {
    let source = include_str!("../../shaders/instanced.vert");
    for column in ["i_model_0", "i_model_1", "i_model_2", "i_model_3"] {
        assert!(source.contains(column));
    }
    assert!(source.contains("mat4 model = mat4"));
}

#[test]
fn uploaded_material_binding_preserves_all_pbr_texture_slots_and_flags() {
    let mut upload = surface_upload(engine_renderer::Transparency::Opaque, false);
    upload.base_color_texture = Some(AssetId::new("base"));
    upload.normal_texture = Some(AssetId::new("normal"));
    upload.metallic_roughness_texture = Some(AssetId::new("metallic-roughness"));
    upload.occlusion_texture = Some(AssetId::new("occlusion"));
    upload.emissive_texture = Some(AssetId::new("emissive"));
    let binding = uploaded_material_binding(&upload);
    assert_eq!(
        binding
            .textures
            .iter()
            .map(|slot| (slot.binding, slot.texture.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (1, "base"),
            (3, "normal"),
            (4, "metallic-roughness"),
            (5, "occlusion"),
            (6, "emissive"),
        ]
    );
    assert_eq!(
        SceneRenderer::parse_material_ubo(&binding.uniforms.bytes).emissive[3],
        31.0
    );
    assert_eq!(SceneRenderer::material_texture_flags(&binding), 31.0);
}

#[test]
fn uploaded_pbr_textures_resolve_to_the_vulkan_descriptor_slot_order() {
    let texture = |id: &str, pixel: [u8; 4]| engine_renderer::TextureUpload {
        texture_id: AssetId::new(id),
        width: 1,
        height: 1,
        format: engine_renderer::TextureUploadFormat::Rgba8,
        color_space: engine_renderer::ColorSpace::Linear,
        mip_levels: vec![engine_renderer::TextureMipLevel {
            width: 1,
            height: 1,
            bytes: pixel.to_vec(),
        }],
        sampler: engine_renderer::SamplerDescriptor::default(),
        content_hash: [pixel[0]; 32],
    };
    let uploaded_textures = [
        texture("base", [180, 140, 100, 255]),
        texture("normal", [128, 128, 255, 255]),
        texture("metallic-roughness", [0, 170, 64, 255]),
        texture("occlusion", [220, 0, 0, 255]),
        texture("emissive", [8, 16, 32, 255]),
    ];
    let mut upload = surface_upload(engine_renderer::Transparency::Opaque, false);
    upload.base_color_texture = Some(AssetId::new("base"));
    upload.normal_texture = Some(AssetId::new("normal"));
    upload.metallic_roughness_texture = Some(AssetId::new("metallic-roughness"));
    upload.occlusion_texture = Some(AssetId::new("occlusion"));
    upload.emissive_texture = Some(AssetId::new("emissive"));
    let binding = uploaded_material_binding(&upload);
    let selected = material_texture_ids_for_descriptor(&binding, |id| {
        uploaded_textures
            .iter()
            .any(|texture| texture.texture_id.id == id)
    })
    .expect("all five texture uploads are available to descriptor selection");

    assert_eq!(
        selected,
        [
            "base".to_string(),
            "normal".to_string(),
            "metallic-roughness".to_string(),
            "occlusion".to_string(),
            "emissive".to_string(),
        ]
    );
}

#[test]
fn forward_shader_projects_every_pbr_texture_slot_but_keeps_the_uv_fallback() {
    let vertex = include_str!("../../shaders/forward.vert");
    let fragment = include_str!("../../shaders/forward.frag");
    assert!(vertex.contains("v_mapping_position"));
    assert!(vertex.contains("draw.mapping_origin"));
    assert!(fragment.contains("sample_triplanar_normal("));
    assert!(fragment.matches("sample_triplanar(").count() >= 5);
    for sampler in [
        "u_base_color_texture",
        "u_normal_texture",
        "u_metallic_roughness_texture",
        "u_occlusion_texture",
        "u_emissive_texture",
    ] {
        assert!(fragment.contains(sampler));
    }
    assert!(
        fragment.contains(": texture(u_base_color_texture, v_uv)"),
        "ordinary models must retain their authored UV sampling path"
    );
}
