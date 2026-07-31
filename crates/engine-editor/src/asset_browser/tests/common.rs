use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
pub(super) use engine_asset::cook::{AssetType, CookRules, SourceAssetEntry, SourceManifest};
pub(super) use engine_asset::AssetRegistry;
use engine_renderer::{
    AxisAlignedBox, ColorSpace, IndexFormat, MaterialUpload, MeshUpload, MeshVertexFormat,
    SamplerDescriptor, TextureMipLevel, TextureUpload, TextureUploadFormat, Transparency,
};
pub(super) use engine_serialize::{AssetId, SchemaVersion};

use super::super::*;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

pub(super) struct AssetCatalogFixture {
    root: PathBuf,
    pub(super) source_root: PathBuf,
    pub(super) cooked_root: PathBuf,
}

impl AssetCatalogFixture {
    pub(super) fn new(name: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "engine_editor_asset_browser_{name}_{}_{sequence}",
            std::process::id()
        ));
        let source_root = root.join("assets/source");
        let cooked_root = root.join("assets/cooked");
        std::fs::create_dir_all(&source_root).expect("create source root");
        std::fs::create_dir_all(&cooked_root).expect("create cooked root");
        Self {
            root,
            source_root,
            cooked_root,
        }
    }

    pub(super) fn write_manifest(&self, name: &str, manifest: &SourceManifest) {
        let bytes = serde_json::to_vec_pretty(manifest).expect("serialize source manifest");
        std::fs::write(self.source_root.join(name), bytes).expect("write source manifest");
    }

    pub(super) fn write_cooked_marker(&self, id: &str) {
        std::fs::write(self.cooked_root.join(format!("{id}.cooked")), b"cooked")
            .expect("write cooked marker");
    }
}

impl Drop for AssetCatalogFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) fn source_entry(id: &str, asset_type: AssetType, source_path: &str) -> SourceAssetEntry {
    SourceAssetEntry {
        id: AssetId::new(id),
        asset_type,
        source_path: source_path.to_string(),
        cook_rules: CookRules::default(),
    }
}

pub(super) fn empty_catalog_refresh(
    panel: &mut AssetBrowserPanel,
    registry: &AssetRegistry,
    fixture: &AssetCatalogFixture,
) {
    refresh_project_asset_list(panel, registry, &fixture.source_root)
        .expect("refresh project asset catalog");
}

pub(super) fn mesh(id: AssetId) -> MeshUpload {
    MeshUpload {
        mesh_id: id,
        vertex_format: MeshVertexFormat::Pbr32,
        vertex_count: 3,
        vertex_bytes: vec![0; 96],
        index_format: IndexFormat::U16,
        index_count: 3,
        index_bytes: vec![0, 0, 1, 0, 2, 0],
        bounds: AxisAlignedBox::UNIT,
        content_hash: [1; 32],
    }
}

pub(super) fn texture(id: AssetId) -> TextureUpload {
    TextureUpload {
        texture_id: id,
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
        content_hash: [2; 32],
    }
}

pub(super) fn material(id: AssetId) -> MaterialUpload {
    MaterialUpload {
        material_id: id,
        base_color: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        emissive: [0.0; 3],
        base_color_texture: None,
        normal_texture: None,
        metallic_roughness_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        advanced: engine_renderer::AdvancedMaterialParameters::default(),
        transparency: Transparency::Opaque,
        double_sided: false,
        content_hash: [3; 32],
    }
}

pub(super) fn registry_with_typed_assets() -> AssetRegistry {
    let mut registry = AssetRegistry::new();
    let mesh_id = AssetId::with_path("plain-name", "models/plain.mesh");
    let texture_id = AssetId::with_path("not-a-prefix", "textures/albedo.png");
    let material_id = AssetId::with_path("also-plain", "materials/default.mat");
    registry.insert_typed(mesh_id.clone(), mesh(mesh_id));
    registry.insert_typed(texture_id.clone(), texture(texture_id));
    registry.insert_typed(material_id.clone(), material(material_id));
    registry.insert_typed(AssetId::new("mesh-lie"), 42_u32);
    registry
}
