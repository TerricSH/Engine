pub(crate) fn cooked_case(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "engine_core_cooked_material_{name}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn cook_test_material(dir: &Path, id: &str, texture: Option<&str>) {
    cook_test_material_with_color(dir, id, texture, [0.8, 0.7, 0.6, 1.0]);
}

pub(crate) fn cook_test_material_with_color(
    dir: &Path,
    id: &str,
    texture: Option<&str>,
    base_color: [f32; 4],
) {
    let texture_field = texture
        .map(|texture| format!(r#", "base_color_texture": "{texture}""#))
        .unwrap_or_default();
    let source = dir.join(format!("{id}.material.json"));
    std::fs::write(
        &source,
        format!(
            r#"{{
                "schema": "MaterialSource-v0",
                "base_color": [{}, {}, {}, {}],
                "metallic": 0.25,
                "roughness": 0.5,
                "ambient_occlusion": 1.0{texture_field},
                "transparency": "Opaque",
                "double_sided": false
            }}"#,
            base_color[0], base_color[1], base_color[2], base_color[3]
        ),
    )
    .unwrap();
    engine_asset::cook::cook_material(&source, &dir.join(format!("{id}.cooked"))).unwrap();
}

fn cook_test_surface_material(
    dir: &Path,
    id: &str,
    transparency: &str,
    alpha_cutoff: f32,
    double_sided: bool,
) {
    let source = dir.join(format!("{id}.material.json"));
    std::fs::write(
        &source,
        format!(
            r#"{{
                "schema": "MaterialSource-v0",
                "base_color": [0.8, 0.7, 0.6, 0.5],
                "metallic": 0.25,
                "roughness": 0.5,
                "ambient_occlusion": 1.0,
                "transparency": "{transparency}",
                "alpha_cutoff": {alpha_cutoff},
                "double_sided": {double_sided}
            }}"#
        ),
    )
    .unwrap();
    engine_asset::cook::cook_material(&source, &dir.join(format!("{id}.cooked"))).unwrap();
}

fn texture_upload(id: &str) -> TextureUpload {
    TextureUpload {
        texture_id: AssetId::new(id),
        width: 1,
        height: 1,
        format: TextureUploadFormat::Rgba8,
        color_space: ColorSpace::Srgb,
        mip_levels: vec![TextureMipLevel {
            width: 1,
            height: 1,
            bytes: vec![255; 4],
        }],
        sampler: SamplerDescriptor::default(),
        content_hash: [1; 32],
    }
}

fn material_upload(id: &str, texture: Option<&str>) -> MaterialUpload {
    MaterialUpload {
        material_id: AssetId::new(id),
        base_color: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [0.0; 3],
        base_color_texture: texture.map(AssetId::new),
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency: Transparency::Opaque,
        double_sided: false,
        content_hash: [2; 32],
    }
}

fn drain_until_idle(
    runtime: &mut EngineRuntime,
    max_iterations: usize,
) -> crate::StreamDrainReport {
    let mut last = crate::StreamDrainReport::default();
    for _ in 0..max_iterations {
        last = runtime.drain_cooked_asset_stream();
        if last.is_complete() {
            return last;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    last
}
