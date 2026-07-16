use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine_asset::cook::{
    decode_cooked_material, decode_cooked_mesh, decode_cooked_texture, read_cooked_artifact,
    registered_asset_type_id, AssetType,
};
use engine_renderer::{
    AssetId, AxisAlignedBox, ColorSpace, IndexFormat, MaterialUpload, MeshUpload, MeshVertexFormat,
    SamplerDescriptor, TextureMipLevel, TextureUpload, TextureUploadFormat, Transparency,
};
use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::EngineRuntime;

/// Deterministic summary of project cooked assets installed into a runtime.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookedAssetLoadReport {
    pub discovered_assets: usize,
    pub loaded_meshes: usize,
    pub loaded_textures: usize,
    pub loaded_materials: usize,
    pub loaded_extension_assets: BTreeMap<String, usize>,
    pub skipped_assets: Vec<String>,
}

impl CookedAssetLoadReport {
    pub fn loaded_render_assets(&self) -> usize {
        self.loaded_meshes + self.loaded_textures + self.loaded_materials
    }

    pub fn loaded_extension_assets(&self) -> usize {
        self.loaded_extension_assets.values().sum()
    }

    pub fn loaded_assets(&self) -> usize {
        self.loaded_render_assets() + self.loaded_extension_assets()
    }
}

impl EngineRuntime {
    /// Validate and register every runtime-loadable cooked asset in `cooked_dir`.
    ///
    /// Meshes, RGBA8 textures, portable opaque materials, and assets owned by
    /// registered runtime extensions are installed transactionally. A corrupt
    /// or unsupported artifact leaves the previous successful batch intact.
    /// Shader, scene, logic, pipeline, and script artifacts are reported as
    /// skipped because their dedicated consumers do not use this cache.
    pub fn load_cooked_assets(
        &mut self,
        cooked_dir: &Path,
    ) -> Result<CookedAssetLoadReport, Vec<Diagnostic>> {
        self.load_cooked_render_assets(cooked_dir)
    }

    /// Compatibility entry point for loading a complete cooked asset batch.
    ///
    /// The historical name is retained for callers, but extension-owned
    /// assets are now loaded alongside render assets.
    pub fn load_cooked_render_assets(
        &mut self,
        cooked_dir: &Path,
    ) -> Result<CookedAssetLoadReport, Vec<Diagnostic>> {
        if !cooked_dir.exists() {
            return Ok(CookedAssetLoadReport::default());
        }
        if !cooked_dir.is_dir() {
            return Err(vec![cooked_error(
                cooked_dir,
                "configured cooked asset path is not a directory",
            )]);
        }

        let entries = match std::fs::read_dir(cooked_dir) {
            Ok(entries) => entries,
            Err(error) => {
                return Err(vec![cooked_error(
                    cooked_dir,
                    format!("could not enumerate cooked assets: {error}"),
                )]);
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    return Err(vec![cooked_error(
                        cooked_dir,
                        format!("could not enumerate a cooked asset entry: {error}"),
                    )]);
                }
            };
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("cooked"))
            {
                paths.push(path);
            }
        }
        paths.sort();

        let mut report = CookedAssetLoadReport {
            discovered_assets: paths.len(),
            ..CookedAssetLoadReport::default()
        };
        let mut meshes = Vec::new();
        let mut textures = Vec::new();
        let mut materials = Vec::new();
        let mut extensions = Vec::new();
        let mut diagnostics = Vec::new();
        for path in paths {
            match decode_cooked_asset(&path, &self.asset_type_registry) {
                Ok(DecodedCookedAsset::Mesh(upload)) => meshes.push(upload),
                Ok(DecodedCookedAsset::Texture(upload)) => textures.push(upload),
                Ok(DecodedCookedAsset::Material(upload)) => {
                    materials.push((path.clone(), upload));
                }
                Ok(DecodedCookedAsset::Extension(asset)) => extensions.push(asset),
                Ok(DecodedCookedAsset::Skipped(kind)) => {
                    report.skipped_assets.push(format!(
                        "{} ({kind:?})",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                Err(error) => diagnostics.push(cooked_error(&path, error)),
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        diagnostics.extend(validate_material_texture_dependencies(
            self,
            &textures,
            &materials,
            &self.loaded_cooked_asset_ids,
        ));
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        report.loaded_meshes = meshes.len();
        report.loaded_textures = textures.len();
        report.loaded_materials = materials.len();
        for asset in &extensions {
            *report
                .loaded_extension_assets
                .entry(asset.type_id.clone())
                .or_default() += 1;
        }

        for id in std::mem::take(&mut self.loaded_cooked_asset_ids) {
            self.asset_registry.unload(&id);
        }
        self.loaded_extension_asset_ids.clear();

        let mut installed_ids = BTreeSet::new();
        for upload in textures {
            installed_ids.insert(upload.texture_id.clone());
            self.register_texture_asset(upload);
        }
        for (_, upload) in materials {
            installed_ids.insert(upload.material_id.clone());
            self.register_material_asset(upload);
        }
        for upload in meshes {
            installed_ids.insert(upload.mesh_id.clone());
            self.register_mesh_asset(upload);
        }
        for asset in extensions {
            installed_ids.insert(asset.id.clone());
            self.loaded_extension_asset_ids
                .entry(asset.type_id)
                .or_default()
                .insert(asset.id.clone());
            self.asset_registry
                .insert_erased(asset.id, asset.payload, asset.value);
        }
        self.loaded_cooked_asset_ids = installed_ids;
        Ok(report)
    }
}

enum DecodedCookedAsset {
    Mesh(MeshUpload),
    Texture(TextureUpload),
    Material(MaterialUpload),
    Extension(DecodedExtensionAsset),
    Skipped(AssetType),
}

struct DecodedExtensionAsset {
    type_id: String,
    id: AssetId,
    payload: Vec<u8>,
    value: Box<dyn Any + Send + Sync>,
}

fn decode_cooked_asset(
    path: &Path,
    asset_type_registry: &AssetTypeRegistry,
) -> Result<DecodedCookedAsset, String> {
    let id = cooked_asset_id(path)?;
    let artifact = read_cooked_artifact(path).map_err(|error| error.to_string())?;
    let asset_type = AssetType::from_kind_code(artifact.header.asset_kind);
    if asset_type == AssetType::Unknown {
        return Err(format!(
            "unsupported cooked asset kind code {}",
            artifact.header.asset_kind
        ));
    }
    match asset_type {
        AssetType::Mesh => {
            let mesh = decode_cooked_mesh(&artifact).map_err(|error| error.to_string())?;
            if mesh.positions.is_empty() {
                return Err("cooked mesh has no vertices".into());
            }
            let (vertex_bytes, index_bytes, index_count, _) =
                engine_asset::mesh::mesh_data_to_upload_bytes(&mesh);
            Ok(DecodedCookedAsset::Mesh(MeshUpload {
                mesh_id: id,
                vertex_format: MeshVertexFormat::Pbr32,
                vertex_count: u32::try_from(mesh.positions.len())
                    .map_err(|_| "cooked mesh vertex count exceeds u32".to_string())?,
                vertex_bytes,
                index_format: IndexFormat::U32,
                index_count,
                index_bytes,
                bounds: AxisAlignedBox {
                    min: mesh.bounds.0.to_array(),
                    max: mesh.bounds.1.to_array(),
                },
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Texture => {
            let texture = decode_cooked_texture(&artifact).map_err(|error| error.to_string())?;
            if texture.format != engine_asset::cook::TextureFormat::Rgba8Unorm {
                return Err(format!(
                    "unsupported cooked texture format: {:?}",
                    texture.format
                ));
            }
            let mip_levels = split_rgba8_mips(
                texture.width,
                texture.height,
                texture.mip_count,
                &texture.data,
            )?;
            Ok(DecodedCookedAsset::Texture(TextureUpload {
                texture_id: id,
                width: texture.width,
                height: texture.height,
                format: TextureUploadFormat::Rgba8,
                color_space: ColorSpace::Srgb,
                mip_levels,
                sampler: SamplerDescriptor::default(),
                content_hash: artifact.header.content_hash,
            }))
        }
        AssetType::Material => {
            let material = decode_cooked_material(&artifact).map_err(|error| error.to_string())?;
            for (field, value) in [
                ("base_color[0]", material.base_color[0]),
                ("base_color[1]", material.base_color[1]),
                ("base_color[2]", material.base_color[2]),
                ("base_color[3]", material.base_color[3]),
                ("metallic", material.metallic),
                ("roughness", material.roughness),
                ("ambient_occlusion", material.ambient_occlusion),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!(
                        "cooked material field '{field}' must be finite and in the range 0..=1"
                    ));
                }
            }
            let transparency = match material.transparency {
                engine_asset::cook::MaterialTransparency::Opaque => Transparency::Opaque,
            };
            if material.double_sided {
                return Err("double-sided cooked materials are not supported".into());
            }
            Ok(DecodedCookedAsset::Material(MaterialUpload {
                material_id: id,
                base_color: material.base_color,
                metallic: material.metallic,
                roughness: material.roughness,
                ambient_occlusion: material.ambient_occlusion,
                base_color_texture: material.base_color_texture,
                transparency,
                double_sided: false,
                content_hash: artifact.header.content_hash,
            }))
        }
        kind @ (AssetType::Audio
        | AssetType::Animation
        | AssetType::Skeleton
        | AssetType::NavMesh) => {
            let type_id = registered_asset_type_id(&kind)
                .expect("extension-owned asset types have a stable registry mapping");
            let extension = asset_type_registry.get(type_id).ok_or_else(|| {
                format!("cooked {kind:?} asset requires registered extension '{type_id}'")
            })?;
            let loader = extension
                .loader
                .ok_or_else(|| format!("registered extension '{type_id}' has no runtime loader"))?;
            let value = loader(&artifact.payload).map_err(|error| {
                format!("extension loader '{type_id}' rejected cooked payload: {error}")
            })?;
            Ok(DecodedCookedAsset::Extension(DecodedExtensionAsset {
                type_id: type_id.to_string(),
                id,
                payload: artifact.payload,
                value,
            }))
        }
        AssetType::Font => {
            Err("cooked Font assets have no registered runtime loader mapping".into())
        }
        kind @ (AssetType::Shader
        | AssetType::Scene
        | AssetType::Pipeline
        | AssetType::Script
        | AssetType::Logic) => Ok(DecodedCookedAsset::Skipped(kind)),
        AssetType::Unknown => unreachable!("unknown kind was rejected above"),
    }
}

fn validate_material_texture_dependencies(
    runtime: &EngineRuntime,
    textures: &[TextureUpload],
    materials: &[(PathBuf, MaterialUpload)],
    replaced_asset_ids: &BTreeSet<AssetId>,
) -> Vec<Diagnostic> {
    let batch_texture_ids = textures
        .iter()
        .map(|upload| upload.texture_id.clone())
        .collect::<BTreeSet<_>>();

    materials
        .iter()
        .filter_map(|(path, upload)| {
            let texture_id = upload.base_color_texture.as_ref()?;
            let available = batch_texture_ids.contains(texture_id)
                || (!replaced_asset_ids.contains(texture_id)
                    && runtime
                        .asset_registry()
                        .get::<TextureUpload>(texture_id)
                        .is_some());
            (!available).then(|| {
                cooked_error(
                    path,
                    format!(
                        "cooked material '{}' references missing texture '{}'",
                        upload.material_id.id, texture_id.id
                    ),
                )
            })
        })
        .collect()
}

fn cooked_asset_id(path: &Path) -> Result<AssetId, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| format!("cooked asset has no UTF-8 file stem: {}", path.display()))?;
    Ok(AssetId::new(stem))
}

fn split_rgba8_mips(
    width: u32,
    height: u32,
    mip_count: u8,
    data: &[u8],
) -> Result<Vec<TextureMipLevel>, String> {
    if width == 0 || height == 0 || mip_count == 0 {
        return Err("cooked texture dimensions and mip count must be non-zero".into());
    }
    let mut levels = Vec::with_capacity(mip_count as usize);
    let mut offset = 0usize;
    let mut mip_width = width;
    let mut mip_height = height;
    for _ in 0..mip_count {
        let byte_count = (mip_width as usize)
            .checked_mul(mip_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "cooked texture mip size overflow".to_string())?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| "cooked texture mip offset overflow".to_string())?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| "cooked texture mip chain is truncated".to_string())?;
        levels.push(TextureMipLevel {
            width: mip_width,
            height: mip_height,
            bytes: bytes.to_vec(),
        });
        offset = end;
        mip_width = (mip_width / 2).max(1);
        mip_height = (mip_height / 2).max(1);
    }
    if offset != data.len() {
        return Err(format!(
            "cooked texture contains {} trailing bytes",
            data.len() - offset
        ));
    }
    Ok(levels)
}

fn cooked_error(path: &Path, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        "AS0002",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        message,
    )
    .path(path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cooked_case(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engine_core_cooked_material_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cook_test_material(dir: &Path, id: &str, texture: Option<&str>) {
        let texture_field = texture
            .map(|texture| format!(r#", "base_color_texture": "{texture}""#))
            .unwrap_or_default();
        let source = dir.join(format!("{id}.material.json"));
        std::fs::write(
            &source,
            format!(
                r#"{{
                    "schema": "MaterialSource-v0",
                    "base_color": [0.8, 0.7, 0.6, 1.0],
                    "metallic": 0.25,
                    "roughness": 0.5,
                    "ambient_occlusion": 1.0{texture_field},
                    "transparency": "Opaque",
                    "double_sided": false
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
            base_color_texture: texture.map(AssetId::new),
            transparency: Transparency::Opaque,
            double_sided: false,
            content_hash: [2; 32],
        }
    }

    #[test]
    fn rgba8_mip_split_rejects_truncated_and_trailing_data() {
        assert!(split_rgba8_mips(2, 2, 2, &[0; 19]).is_err());
        assert!(split_rgba8_mips(2, 2, 2, &[0; 21]).is_err());
        let levels = split_rgba8_mips(2, 2, 2, &[0; 20]).unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[1].width, levels[1].height), (1, 1));
    }

    #[test]
    fn missing_cooked_directory_is_an_empty_load() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        let missing = std::path::PathBuf::from("definitely-missing-cooked-assets");
        let report = runtime.load_cooked_render_assets(&missing).unwrap();
        assert_eq!(report, CookedAssetLoadReport::default());
    }

    #[test]
    fn material_texture_dependency_accepts_batch_or_typed_registry_texture() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.register_texture_asset(texture_upload("texture.registry"));
        let materials = vec![
            (
                PathBuf::from("batch.cooked"),
                material_upload("material.batch", Some("texture.batch")),
            ),
            (
                PathBuf::from("registry.cooked"),
                material_upload("material.registry", Some("texture.registry")),
            ),
        ];

        assert!(validate_material_texture_dependencies(
            &runtime,
            &[texture_upload("texture.batch")],
            &materials,
            &BTreeSet::new(),
        )
        .is_empty());
    }

    #[test]
    fn material_texture_dependency_requires_a_typed_texture() {
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        runtime.register_material_asset(material_upload("texture.wrong-type", None));
        let path = PathBuf::from("missing-dependency.cooked");
        let materials = vec![(
            path.clone(),
            material_upload("material.invalid", Some("texture.wrong-type")),
        )];

        let diagnostics =
            validate_material_texture_dependencies(&runtime, &[], &materials, &BTreeSet::new());

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].path.as_deref(), path.to_str());
        assert!(diagnostics[0].message.contains("texture.wrong-type"));
    }

    #[test]
    fn cooked_material_is_registered_and_counted() {
        let dir = cooked_case("load");
        cook_test_material(&dir, "material.plain", None);
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        let report = runtime.load_cooked_render_assets(&dir).unwrap();

        assert_eq!(report.discovered_assets, 1);
        assert_eq!(report.loaded_materials, 1);
        assert_eq!(report.loaded_render_assets(), 1);
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.plain"))
            .is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_material_texture_prevents_partial_batch_registration() {
        let dir = cooked_case("atomic_dependency_failure");
        cook_test_material(&dir, "material.valid", None);
        cook_test_material(&dir, "material.invalid", Some("texture.missing"));
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());

        let diagnostics = runtime.load_cooked_render_assets(&dir).unwrap_err();

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("texture.missing"));
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.valid"))
            .is_none());
        assert!(runtime
            .asset_registry()
            .get::<MaterialUpload>(&AssetId::new("material.invalid"))
            .is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(not(feature = "runtime-subsystems"))]
    fn test_extension_loader(cooked: &[u8]) -> Result<Box<dyn Any + Send + Sync>, String> {
        String::from_utf8(cooked.to_vec())
            .map(|value| Box::new(value) as Box<dyn Any + Send + Sync>)
            .map_err(|error| error.to_string())
    }

    #[cfg(not(feature = "runtime-subsystems"))]
    #[test]
    fn registered_extension_assets_share_the_typed_cache_and_reload_atomically() {
        use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

        let dir = cooked_case("extension_transaction");
        let id = AssetId::new("audio.custom");
        engine_asset::cook::write_cooked_artifact(
            &dir.join("audio.custom.cooked"),
            AssetType::Audio.kind_code(),
            b"first payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let mut builder = crate::EngineRuntime::builder(crate::EngineConfig::default());
        builder
            .asset_type_registry_mut()
            .register(AssetTypeExtension {
                meta: AssetTypeMeta {
                    type_id: "audio_clip",
                    source_extensions: vec!["custom"],
                    display_name: "Custom Audio",
                },
                cooker: None,
                loader: Some(test_extension_loader),
            })
            .unwrap();
        let mut runtime = builder.build();

        let report = runtime.load_cooked_assets(&dir).unwrap();

        assert_eq!(report.loaded_extension_assets(), 1);
        assert_eq!(report.loaded_assets(), 1);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("extension asset")
                .get(),
            "first payload"
        );
        assert_eq!(
            runtime
                .asset_registry_mut()
                .load(&id)
                .expect("raw payload")
                .get(),
            b"first payload"
        );

        engine_asset::cook::write_cooked_artifact(
            &dir.join("broken.cooked"),
            4_242,
            b"valid outer artifact with unknown kind",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("kind code 4242")));
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<String>("audio_clip", &id)
                .expect("previous batch remains installed")
                .get(),
            "first payload"
        );

        std::fs::remove_file(dir.join("broken.cooked")).unwrap();
        std::fs::remove_file(dir.join("audio.custom.cooked")).unwrap();
        let empty_report = runtime.load_cooked_assets(&dir).unwrap();
        assert_eq!(empty_report.loaded_assets(), 0);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 0);
        assert!(runtime.asset_registry().get::<String>(&id).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(feature = "runtime-subsystems")]
    fn write_registered_extension_source(
        runtime: &EngineRuntime,
        dir: &Path,
        id: &str,
        kind: AssetType,
        source: &[u8],
    ) {
        let type_id = registered_asset_type_id(&kind).expect("mapped extension kind");
        let extension = runtime
            .asset_type_registry()
            .get(type_id)
            .expect("registered runtime extension");
        let mut payload = Vec::new();
        extension.cooker.expect("registered extension cooker")(source, &mut payload).unwrap();
        engine_asset::cook::write_cooked_artifact(
            &dir.join(format!("{id}.cooked")),
            kind.kind_code(),
            &payload,
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
    }

    #[cfg(feature = "runtime-subsystems")]
    fn minimal_pcm_wav() -> Vec<u8> {
        let samples = [0i16; 80];
        let data_size = u32::try_from(samples.len() * 2).unwrap();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8_000u32.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[cfg(feature = "runtime-subsystems")]
    #[test]
    fn runtime_subsystem_cookers_and_loaders_roundtrip_all_mapped_asset_kinds() {
        use engine_animation::{AnimationClip, Joint, JointTransform, Skeleton};
        use engine_nav::NavMesh;
        use glam::Vec3;

        let dir = cooked_case("real_runtime_extensions");
        let mut runtime = EngineRuntime::new(crate::EngineConfig::default());
        write_registered_extension_source(
            &runtime,
            &dir,
            "audio.real",
            AssetType::Audio,
            &minimal_pcm_wav(),
        );
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "root".into(),
                parent_index: None,
                local_transform: JointTransform::IDENTITY,
            }],
            inverse_bind_matrices: vec![[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]],
        };
        write_registered_extension_source(
            &runtime,
            &dir,
            "skeleton.real",
            AssetType::Skeleton,
            &bincode::serialize(&skeleton).unwrap(),
        );
        let clip = AnimationClip {
            name: "idle".into(),
            duration: 1.0,
            channels: vec![],
            joint_indices: vec![],
        };
        write_registered_extension_source(
            &runtime,
            &dir,
            "animation.real",
            AssetType::Animation,
            &bincode::serialize(&clip).unwrap(),
        );
        let mut navmesh = NavMesh::new();
        let a = navmesh.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = navmesh.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = navmesh.add_vertex(Vec3::new(0.0, 0.0, 1.0));
        navmesh.add_polygon(&[a, b, c], 1.0);
        navmesh.rebuild_bvh();
        write_registered_extension_source(
            &runtime,
            &dir,
            "navmesh.real",
            AssetType::NavMesh,
            &bincode::serialize(&navmesh).unwrap(),
        );

        let report = runtime.load_cooked_assets(&dir).unwrap();

        assert_eq!(report.discovered_assets, 4);
        assert_eq!(report.loaded_extension_assets(), 4);
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(runtime.extension_asset_count("skeleton"), 1);
        assert_eq!(runtime.extension_asset_count("animation_clip"), 1);
        assert_eq!(runtime.extension_asset_count("navmesh"), 1);
        assert_eq!(
            runtime
                .extension_asset::<engine_audio::AudioClip>(
                    "audio_clip",
                    &AssetId::new("audio.real"),
                )
                .expect("audio clip")
                .get()
                .sample_rate(),
            8_000
        );
        assert_eq!(
            runtime
                .extension_asset::<Skeleton>("skeleton", &AssetId::new("skeleton.real"))
                .expect("skeleton")
                .get()
                .joint_count(),
            1
        );
        assert_eq!(
            runtime
                .extension_asset::<AnimationClip>(
                    "animation_clip",
                    &AssetId::new("animation.real"),
                )
                .expect("animation clip")
                .get()
                .name(),
            "idle"
        );
        assert!(runtime
            .extension_asset::<NavMesh>("navmesh", &AssetId::new("navmesh.real"))
            .is_some());

        engine_asset::cook::write_cooked_artifact(
            &dir.join("audio.real.cooked"),
            AssetType::Audio.kind_code(),
            b"not a valid cooked audio payload",
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .unwrap();
        let diagnostics = runtime.load_cooked_assets(&dir).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("extension loader 'audio_clip'") }));
        assert_eq!(runtime.extension_asset_count("audio_clip"), 1);
        assert_eq!(
            runtime
                .extension_asset::<engine_audio::AudioClip>(
                    "audio_clip",
                    &AssetId::new("audio.real"),
                )
                .expect("previous audio remains installed")
                .get()
                .sample_rate(),
            8_000
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
