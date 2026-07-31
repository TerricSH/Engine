use crate::*;

pub(crate) fn scene_load_diagnostic(diagnostic: SceneLoadDiagnostic) -> Diagnostic {
    let (code, entity_id, component_type_id, storage_type_id) = match &diagnostic {
        SceneLoadDiagnostic::UnknownComponent {
            entity_id,
            component_type_id,
        } => ("SC0030", entity_id, component_type_id, None),
        SceneLoadDiagnostic::MissingDeserializeHook {
            entity_id,
            component_type_id,
        } => ("SC0031", entity_id, component_type_id, None),
        SceneLoadDiagnostic::StorageFactoryTypeMismatch {
            entity_id,
            component_type_id,
            storage_type_id,
        } => (
            "SC0032",
            entity_id,
            component_type_id,
            Some(storage_type_id),
        ),
        SceneLoadDiagnostic::StorageInsertTypeMismatch {
            entity_id,
            component_type_id,
        } => ("SC0033", entity_id, component_type_id, None),
        SceneLoadDiagnostic::InvalidComponentFields {
            entity_id,
            component_type_id,
            ..
        } => ("SC0034", entity_id, component_type_id, None),
        SceneLoadDiagnostic::DuplicateSingletonComponent {
            entity_id,
            component_type_id,
            ..
        } => ("SC0035", entity_id, component_type_id, None),
    };

    let mut mapped = Diagnostic::new(
        code,
        DiagnosticSeverity::Error,
        "engine-core.scene-loader",
        diagnostic.to_string(),
    )
    .entity(entity_id.clone())
    .path(format!(
        "entities[{entity_id}].components[{component_type_id}]"
    ));
    mapped
        .fields
        .insert("component_type_id".to_string(), component_type_id.clone());
    if let Some(storage_type_id) = storage_type_id {
        mapped
            .fields
            .insert("storage_type_id".to_string(), storage_type_id.clone());
    }
    mapped
}

pub(crate) fn missing_registered_render_asset(kind: &str, requested: &AssetId) -> Vec<Diagnostic> {
    let mut diagnostic = Diagnostic::new(
        "AS0002",
        DiagnosticSeverity::Error,
        "engine-core.assets",
        format!(
            "{kind} asset '{}' is referenced by the frame but is not registered in AssetRegistry",
            requested.id
        ),
    );
    diagnostic.asset = Some(requested.clone());
    vec![diagnostic]
}

pub(crate) fn validate_registered_asset_id(
    kind: &str,
    requested: &AssetId,
    embedded: &AssetId,
) -> Result<(), Vec<Diagnostic>> {
    if requested == embedded {
        return Ok(());
    }
    let mut diagnostic = Diagnostic::new(
        "AS0001",
        DiagnosticSeverity::Error,
        "engine-core.assets",
        format!(
            "registered {kind} asset '{}' embeds mismatched id '{}'",
            requested.id, embedded.id
        ),
    );
    diagnostic.asset = Some(requested.clone());
    Err(vec![diagnostic])
}

pub(crate) fn install_builtin_render_assets(registry: &mut AssetRegistry) {
    let mesh = engine_asset::mesh::create_test_cube();
    let (vertex_bytes, index_bytes, index_count, _) =
        engine_asset::mesh::mesh_data_to_upload_bytes(&mesh);
    let content_hash =
        engine_asset::compute_content_hash(&[vertex_bytes.as_slice(), index_bytes.as_slice()]);
    let mesh_id = AssetId::new("mesh-cube");
    registry.insert_typed(
        mesh_id.clone(),
        MeshUpload {
            mesh_id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX),
            vertex_bytes,
            index_format: engine_renderer::IndexFormat::U32,
            index_count,
            index_bytes,
            bounds: engine_renderer::AxisAlignedBox {
                min: mesh.bounds.0.to_array(),
                max: mesh.bounds.1.to_array(),
            },
            content_hash,
        },
    );

    let material_id = AssetId::new("mat-default");
    registry.insert_typed(
        material_id.clone(),
        MaterialUpload {
            material_id,
            base_color: [1.0, 1.0, 1.0, 1.0],
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
            transparency: engine_renderer::Transparency::Opaque,
            double_sided: false,
            content_hash: engine_asset::compute_content_hash(&[b"mat-default-v1"]),
        },
    );

    let quad = engine_asset::mesh::MeshData {
        positions: vec![
            glam::Vec3::new(-0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, 0.5, 0.0),
            glam::Vec3::new(-0.5, 0.5, 0.0),
        ],
        normals: vec![glam::Vec3::Z; 4],
        uvs: vec![
            glam::Vec2::new(0.0, 1.0),
            glam::Vec2::new(1.0, 1.0),
            glam::Vec2::new(1.0, 0.0),
            glam::Vec2::new(0.0, 0.0),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        bounds: (
            glam::Vec3::new(-0.5, -0.5, 0.0),
            glam::Vec3::new(0.5, 0.5, 0.0),
        ),
        joints: Vec::new(),
        weights: Vec::new(),
    };
    let (vertex_bytes, index_bytes, index_count, _) =
        engine_asset::mesh::mesh_data_to_upload_bytes(&quad);
    let content_hash =
        engine_asset::compute_content_hash(&[vertex_bytes.as_slice(), index_bytes.as_slice()]);
    let mesh_id = AssetId::new(engine_vfx::BUILTIN_VFX_QUAD_MESH_ID);
    registry.insert_typed(
        mesh_id.clone(),
        MeshUpload {
            mesh_id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: 4,
            vertex_bytes,
            index_format: engine_renderer::IndexFormat::U32,
            index_count,
            index_bytes,
            bounds: engine_renderer::AxisAlignedBox {
                min: quad.bounds.0.to_array(),
                max: quad.bounds.1.to_array(),
            },
            content_hash,
        },
    );

    let material_id = AssetId::new(engine_vfx::BUILTIN_VFX_MATERIAL_ID);
    registry.insert_typed(
        material_id.clone(),
        MaterialUpload {
            material_id,
            base_color: [1.0, 1.0, 1.0, 0.75],
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
            transparency: engine_renderer::Transparency::Blend,
            double_sided: true,
            content_hash: engine_asset::compute_content_hash(&[b"mat-vfx-default-v1"]),
        },
    );
}
