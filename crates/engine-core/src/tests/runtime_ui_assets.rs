#[test]
fn engine_runtime_config_accessor() {
    let config = EngineConfig::default();
    let runtime = EngineRuntime::new(config);
    let retrieved = runtime.config();
    assert_eq!(retrieved.application_name, "engine");
}

#[test]
fn engine_runtime_render_frame_without_scene_fails() {
    let config = EngineConfig::default();
    let mut runtime = EngineRuntime::new(config);
    let result = runtime.render_frame(0);
    assert!(result.is_err());
}

#[test]
fn runtime_submits_host_ui_batches_with_the_scene() {
    let _guard = serial_ffi_world_test();
    let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    let batch = engine_renderer::UiBatch {
        canvas_id: "editor".into(),
        z_order: 0,
        clip_rect: engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [800.0, 600.0],
        },
        texture: None,
        vertices: vec![
            engine_renderer::UiVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [10.0, 0.0],
                uv: [1.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [10.0, 10.0],
                uv: [1.0, 1.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [0.0, 10.0],
                uv: [0.0, 1.0],
                color: [255; 4],
            },
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        material: AssetId::new("ui/default"),
    };

    runtime
        .render_frame_with_ui(7, vec![batch])
        .expect("scene and host UI should render together");
    let ui_counts = ui_counts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!ui_counts.is_empty());
    assert!(ui_counts.iter().all(|count| *count == 1));
}

#[cfg(feature = "subsystem-ui")]
#[test]
fn runtime_refreshes_generated_font_atlas_after_ui_batch_build() {
    if engine_ui::font_atlas_texture_upload().is_none() {
        return;
    }
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");

    let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
    canvas.add_element(engine_ui::UiElement::new(
        engine_ui::UiElementKind::Text {
            content: "Editor text".into(),
            font_size: 18.0,
            color: engine_ui::Color::WHITE,
        },
        engine_ui::Layout::FILL,
    ));
    canvas.layout_all();
    let batches = canvas.build_batches();
    assert!(batches.iter().any(|batch| {
        batch
            .texture
            .as_ref()
            .is_some_and(|texture| texture.id == engine_ui::FONT_ATLAS_ASSET)
    }));

    runtime
        .render_frame_with_ui(0, batches)
        .expect("generated font atlas should be registered before rendering");

    let texture_id = AssetId::new(engine_ui::FONT_ATLAS_ASSET);
    let atlas = runtime
        .asset_registry()
        .get::<TextureUpload>(&texture_id)
        .expect("font atlas must be owned by AssetRegistry");
    assert!(atlas.get().mip_levels[0]
        .bytes
        .chunks_exact(4)
        .any(|pixel| pixel[3] != 0));
    assert!(uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|upload| upload == "texture:engine/font-atlas"));
}

#[cfg(feature = "subsystem-ui")]
#[test]
fn game_loop_submits_retained_scene_canvas_batches_automatically() {
    let _guard = serial_ffi_world_test();
    let ui_counts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut game_loop = game_loop::GameLoop::new(EngineConfig::default());
    game_loop
        .runtime
        .set_renderer_backend(Box::new(RecordingBackend {
            uploads: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            rendered_ui_batch_counts: Some(std::sync::Arc::clone(&ui_counts)),
        }));
    game_loop
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    game_loop
        .runtime
        .with_world_mut(|world| {
            let entity = world.entity_by_persistent_id("camera-main").unwrap();
            let mut canvas = engine_ui::Canvas::new(320.0, 180.0);
            canvas.add_element(engine_ui::UiElement::new(
                engine_ui::UiElementKind::Panel {
                    color: engine_ui::Color::new(40, 80, 120, 255),
                },
                engine_ui::Layout::FILL,
            ));
            world.add_component(entity, canvas);
        })
        .expect("runtime world should be available");

    game_loop
        .render(7)
        .expect("retained scene Canvas should render automatically");

    let ui_counts = ui_counts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(!ui_counts.is_empty());
    assert!(ui_counts.iter().all(|count| *count == 1));
}

#[test]
fn runtime_uploads_and_deduplicates_ui_only_textures() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));

    let texture_id = AssetId::new("texture-ui-atlas");
    runtime.register_texture_asset(TextureUpload {
        texture_id: texture_id.clone(),
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
        content_hash: [9; 32],
    });
    runtime
        .load_scene(engine_scene::sample_scene())
        .expect("sample scene should load");
    let batch = engine_renderer::UiBatch {
        canvas_id: "hud".into(),
        z_order: 0,
        clip_rect: engine_renderer::Rect {
            min: [0.0, 0.0],
            max: [128.0, 128.0],
        },
        texture: Some(texture_id),
        vertices: vec![
            engine_renderer::UiVertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [1.0, 0.0],
                uv: [1.0, 0.0],
                color: [255; 4],
            },
            engine_renderer::UiVertex {
                position: [0.0, 1.0],
                uv: [0.0, 1.0],
                color: [255; 4],
            },
        ],
        indices: vec![0, 1, 2],
        material: AssetId::new("ui/default"),
    };

    runtime
        .render_frame_with_ui(8, vec![batch.clone(), batch])
        .expect("UI texture should be synchronised before rendering");

    let uploads = uploads
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(
        uploads
            .iter()
            .filter(|upload| upload.as_str() == "texture:texture-ui-atlas")
            .count(),
        1
    );
}

#[test]
fn runtime_uploads_registered_scene_resources_in_dependency_order() {
    let _guard = serial_ffi_world_test();
    let uploads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.set_renderer_backend(Box::new(RecordingBackend {
        uploads: std::sync::Arc::clone(&uploads),
        rendered_ui_batch_counts: None,
    }));

    let texture_id = AssetId::new("texture-auto");
    runtime.register_texture_asset(TextureUpload {
        texture_id: texture_id.clone(),
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
        content_hash: [1; 32],
    });
    let material_id = AssetId::new("material-auto");
    runtime.register_material_asset(MaterialUpload {
        material_id: material_id.clone(),
        base_color: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [0.0; 3],
        base_color_texture: Some(texture_id),
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency: engine_renderer::Transparency::Opaque,
        double_sided: false,
        content_hash: [2; 32],
    });

    let mut scene = engine_scene::sample_scene();
    let renderable = scene
        .entities
        .iter_mut()
        .find_map(|entity| entity.components.get_mut("engine.renderable"))
        .expect("sample renderable");
    renderable.fields.insert(
        "material".to_string(),
        engine_serialize::Value::Asset(material_id),
    );
    runtime.load_scene(scene).expect("scene load");

    runtime.render_frame(0).expect("render");

    assert_eq!(
        *uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            "texture:texture-auto".to_string(),
            "material:material-auto".to_string(),
            "mesh:mesh-cube".to_string(),
        ]
    );

    runtime
        .render_frame(1)
        .expect("unchanged resources render without re-upload");
    assert_eq!(
        *uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            "texture:texture-auto".to_string(),
            "material:material-auto".to_string(),
            "mesh:mesh-cube".to_string(),
        ],
        "unchanged large mesh and texture payloads must not be cloned and uploaded every frame"
    );
}
