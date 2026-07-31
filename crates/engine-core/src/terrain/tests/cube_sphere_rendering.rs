use super::*;

#[test]
fn cube_sphere_chunks_stream_into_runtime_meshes_and_trimesh_colliders() {
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(TerrainAssetUploadBackend {
        uploads: std::sync::Arc::clone(&uploads),
    }));
    let texture = |id: &str, color_space: engine_renderer::ColorSpace, pixel: [u8; 4], hash: u8| {
        engine_renderer::TextureUpload {
            texture_id: engine_renderer::AssetId::new(id),
            width: 1,
            height: 1,
            format: engine_renderer::TextureUploadFormat::Rgba8,
            color_space,
            mip_levels: vec![engine_renderer::TextureMipLevel {
                width: 1,
                height: 1,
                bytes: pixel.to_vec(),
            }],
            sampler: engine_renderer::SamplerDescriptor::default(),
            content_hash: [hash; 32],
        }
    };
    let base = engine_renderer::AssetId::new("planet.base");
    let normal = engine_renderer::AssetId::new("planet.normal");
    let metallic_roughness = engine_renderer::AssetId::new("planet.metallic-roughness");
    let occlusion = engine_renderer::AssetId::new("planet.occlusion");
    let emissive = engine_renderer::AssetId::new("planet.emissive");
    for upload in [
        texture(
            &base.id,
            engine_renderer::ColorSpace::Srgb,
            [180, 140, 100, 255],
            1,
        ),
        texture(
            &normal.id,
            engine_renderer::ColorSpace::Linear,
            [128, 128, 255, 255],
            2,
        ),
        texture(
            &metallic_roughness.id,
            engine_renderer::ColorSpace::Linear,
            [0, 170, 64, 255],
            3,
        ),
        texture(
            &occlusion.id,
            engine_renderer::ColorSpace::Linear,
            [220, 0, 0, 255],
            4,
        ),
        texture(
            &emissive.id,
            engine_renderer::ColorSpace::Srgb,
            [8, 16, 32, 255],
            5,
        ),
    ] {
        runtime.register_texture_asset(upload);
    }
    runtime.register_material_asset(engine_renderer::MaterialUpload {
        material_id: engine_renderer::AssetId::new("mat-planet"),
        base_color: [1.0; 4],
        metallic: 1.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [1.0; 3],
        base_color_texture: Some(base),
        normal_texture: Some(normal),
        metallic_roughness_texture: Some(metallic_roughness),
        occlusion_texture: Some(occlusion),
        emissive_texture: Some(emissive),
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency: engine_renderer::Transparency::Opaque,
        double_sided: false,
        content_hash: [6; 32],
    });
    let mut world = World::new();
    let entity = world.create_entity();
    world.add_component(
        entity,
        TerrainVolume {
            topology: engine_terrain::TerrainTopology::CubeSphere,
            planet_radius: 100.0,
            planet_max_lod: 1,
            base_resolution: 5,
            height_scale: 4.0,
            lod_distances: vec![30.0, 500.0],
            lod_hysteresis: 2.0,
            material_asset: "mat-planet".to_string(),
            ..TerrainVolume::default()
        },
    );
    let camera = world.create_entity();
    world.add_component(
        camera,
        Transform {
            translation: Vec3::new(0.0, 0.0, 180.0),
            ..Transform::default()
        },
    );
    world.add_component(
        camera,
        Camera {
            projection: CameraProjection::Perspective,
            near: 0.1,
            far: 1_000.0,
            ..Camera::default()
        },
    );
    runtime.set_world(world);
    let mut terrain = TerrainSystem::new(TerrainRuntimeConfig {
        worker_count: 1,
        max_in_flight: 2,
        ..TerrainRuntimeConfig::default()
    });
    tick_until_settled(&mut terrain, &mut runtime);

    assert!(terrain
        .chunks
        .keys()
        .all(|(id, _)| id.face != engine_terrain::TerrainFace::Planar));
    #[cfg(feature = "subsystem-physics")]
    assert!(runtime
        .with_world(|world| world.query::<engine_physics::Collider>().any(
            |(_, collider)| matches!(
                collider.shape,
                engine_physics::ColliderShape::TriMesh { .. }
            )
        ))
        .unwrap());
    let extracted = runtime
        .with_world(|world| extract_renderer_input_from_world(world, 0))
        .expect("world")
        .expect("extract generated planet");
    let planet_drawables = extracted
        .drawables
        .iter()
        .filter(|drawable| drawable.material.id == "mat-planet")
        .collect::<Vec<_>>();
    assert!(
        !planet_drawables.is_empty(),
        "at least the camera-facing sphere patches must survive extraction"
    );
    assert_eq!(
        extracted
            .extraction_stats
            .expect("extraction stats")
            .visible_drawables as usize,
        planet_drawables.len()
    );
    assert!(planet_drawables
        .iter()
        .any(|drawable| drawable.radial_vertex_morph.is_some()));
    for drawable in planet_drawables {
        let mapping = drawable
            .triplanar_material_mapping
            .as_ref()
            .expect("automatic cube-sphere projection must be planet-relative triplanar");
        assert_eq!(mapping.meters_per_tile, 16.0);
        assert_eq!(mapping.blend_sharpness, 4.0);
        for axis in 0..3 {
            let expected_phase =
                drawable.world_transform[12 + axis].rem_euclid(mapping.meters_per_tile);
            assert!(
                (expected_phase + mapping.local_origin[axis]).abs() < 0.001,
                "mapping origin must retain the planet-relative repeat phase on axis {axis}"
            );
        }
        assert!(drawable.world_transform.into_iter().all(f32::is_finite));
        assert!(drawable
            .bounds
            .min
            .into_iter()
            .chain(drawable.bounds.max)
            .all(f32::is_finite));
        assert!(drawable
            .bounds
            .min
            .into_iter()
            .zip(drawable.bounds.max)
            .all(|(min, max)| min <= max));
        if let Some(morph) = &drawable.radial_vertex_morph {
            assert!((0.0..=1.0).contains(&morph.factor));
            assert!(morph.delta_scale > 0.0);
        }
    }
    runtime
        .render_frame(1)
        .expect("terrain frame uploads its complete PBR dependency set");
    let uploads = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let material_index = uploads
        .iter()
        .position(|entry| entry == "material:mat-planet")
        .expect("terrain material upload");
    for texture_id in [
        "planet.base",
        "planet.normal",
        "planet.metallic-roughness",
        "planet.occlusion",
        "planet.emissive",
    ] {
        let texture_index = uploads
            .iter()
            .position(|entry| entry == &format!("texture:{texture_id}"))
            .unwrap_or_else(|| panic!("missing texture upload {texture_id}"));
        assert!(
            texture_index < material_index,
            "texture dependencies must upload before the terrain material"
        );
    }
    assert!(
        uploads[material_index + 1..]
            .iter()
            .any(|entry| entry.starts_with("mesh:runtime-mesh-terrain-")),
        "runtime terrain meshes must upload after their material dependencies: {uploads:?}"
    );
}
