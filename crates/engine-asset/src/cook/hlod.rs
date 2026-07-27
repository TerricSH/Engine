//! Deterministic automatic HLOD clustering and proxy-mesh baking.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use engine_scene::{ComponentRecord, EntityRecord, Scene};
use engine_serialize::{AssetId, PersistentId, SchemaVersion, Value};
use glam::{Mat4, Vec2, Vec3};
use sha2::{Digest, Sha256};

use crate::mesh::MeshData;

use super::{write_cooked_artifact, AssetType, CookError, CookResult};

pub const HLOD_PROXY_PREFIX: &str = "hlod-proxy-";

#[derive(Clone, Debug)]
pub struct HlodBakeSource {
    pub entity_id: PersistentId,
    pub mesh_asset: AssetId,
    pub material_asset: AssetId,
    pub render_layer: String,
    pub world_transform: Mat4,
    pub mesh: MeshData,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HlodBakeSettings {
    pub cluster_cell_size: f32,
    pub max_sources_per_cluster: usize,
    pub minimum_sources_per_cluster: usize,
    pub target_vertex_ratio: f32,
    pub activation_distance: f32,
    pub cull_distance: f32,
}

impl Default for HlodBakeSettings {
    fn default() -> Self {
        Self {
            cluster_cell_size: 50.0,
            max_sources_per_cluster: 32,
            minimum_sources_per_cluster: 2,
            target_vertex_ratio: 0.35,
            activation_distance: 100.0,
            cull_distance: 2_000.0,
        }
    }
}

impl HlodBakeSettings {
    pub fn validate(self) -> Result<(), CookError> {
        if !self.cluster_cell_size.is_finite()
            || self.cluster_cell_size <= 0.0
            || self.max_sources_per_cluster == 0
            || self.minimum_sources_per_cluster < 2
            || self.minimum_sources_per_cluster > self.max_sources_per_cluster
            || !self.target_vertex_ratio.is_finite()
            || !(0.01..=1.0).contains(&self.target_vertex_ratio)
            || !self.activation_distance.is_finite()
            || self.activation_distance < 0.0
            || !self.cull_distance.is_finite()
            || self.cull_distance < 0.0
            || (self.cull_distance > 0.0 && self.cull_distance <= self.activation_distance)
        {
            return Err(CookError::InvalidAsset(
                "HLOD bake settings require a positive cell size, bounded source counts, a vertex ratio in [0.01, 1], and ordered finite distances"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct HlodProxyBake {
    pub cluster_id: String,
    pub proxy_entity_id: PersistentId,
    pub proxy_mesh_asset: AssetId,
    pub material_asset: AssetId,
    pub render_layer: String,
    pub source_entities: Vec<PersistentId>,
    pub origin: Vec3,
    pub mesh: MeshData,
    pub source_triangle_count: usize,
    pub proxy_triangle_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct HlodBakeOutput {
    pub proxies: Vec<HlodProxyBake>,
    pub skipped_source_entities: Vec<PersistentId>,
}

/// Extract eligible static renderables from a scene, resolve their cooked
/// meshes, and run the complete HLOD clustering stage.
pub fn bake_hlod_scene<F>(
    scene: &Scene,
    settings: HlodBakeSettings,
    mut resolve_mesh: F,
) -> Result<HlodBakeOutput, CookError>
where
    F: FnMut(&AssetId) -> Result<MeshData, CookError>,
{
    let transforms = scene_world_transforms(scene)?;
    let mut sources = Vec::new();
    for entity in &scene.entities {
        if !entity.enabled {
            continue;
        }
        if entity
            .components
            .get("engine.hlod_cluster")
            .and_then(|component| component.fields.get("role"))
            .is_some_and(
                |role| matches!(role, Value::Str(value) if value.eq_ignore_ascii_case("proxy")),
            )
        {
            continue;
        }
        let Some(renderable) = entity.components.get("engine.renderable") else {
            continue;
        };
        if !renderable.enabled
            || matches!(renderable.fields.get("visible"), Some(Value::Bool(false)))
        {
            continue;
        }
        let Some(Value::Asset(mesh_asset)) = renderable.fields.get("mesh") else {
            continue;
        };
        let Some(Value::Asset(material_asset)) = renderable.fields.get("material") else {
            continue;
        };
        let render_layer = match renderable.fields.get("render_layer") {
            Some(Value::Enum(value)) | Some(Value::Str(value)) => value.clone(),
            _ => scene.scene_settings.default_render_layer.clone(),
        };
        sources.push(HlodBakeSource {
            entity_id: entity.persistent_id.clone(),
            mesh_asset: mesh_asset.clone(),
            material_asset: material_asset.clone(),
            render_layer,
            world_transform: transforms
                .get(&entity.persistent_id)
                .copied()
                .unwrap_or(Mat4::IDENTITY),
            mesh: resolve_mesh(mesh_asset)?,
        });
    }
    bake_hlod_proxies(&sources, settings)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterKey {
    x: i64,
    y: i64,
    z: i64,
    material: String,
    render_layer: String,
}

/// Group static source meshes and build local-space proxy meshes.
///
/// Sources are first partitioned by spatial cell, material, and render layer;
/// each partition is then split to the configured maximum size. This avoids
/// silently merging draw-state boundaries into a proxy format that cannot
/// represent them.
pub fn bake_hlod_proxies(
    sources: &[HlodBakeSource],
    settings: HlodBakeSettings,
) -> Result<HlodBakeOutput, CookError> {
    settings.validate()?;
    let mut groups = BTreeMap::<ClusterKey, Vec<&HlodBakeSource>>::new();
    let mut skipped = Vec::new();
    for source in sources {
        if validate_source(source).is_err() {
            skipped.push(source.entity_id.clone());
            continue;
        }
        let center = transformed_bounds(source).map(|(min, max)| (min + max) * 0.5)?;
        let key = ClusterKey {
            x: grid_coordinate(center.x, settings.cluster_cell_size),
            y: grid_coordinate(center.y, settings.cluster_cell_size),
            z: grid_coordinate(center.z, settings.cluster_cell_size),
            material: source.material_asset.id.clone(),
            render_layer: source.render_layer.clone(),
        };
        groups.entry(key).or_default().push(source);
    }

    let mut proxies = Vec::new();
    for (key, mut group) in groups {
        group.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));
        for (chunk_index, chunk) in group.chunks(settings.max_sources_per_cluster).enumerate() {
            if chunk.len() < settings.minimum_sources_per_cluster {
                skipped.extend(chunk.iter().map(|source| source.entity_id.clone()));
                continue;
            }
            proxies.push(bake_cluster(&key, chunk_index, chunk, settings)?);
        }
    }
    proxies.sort_by(|left, right| left.cluster_id.cmp(&right.cluster_id));
    skipped.sort();
    skipped.dedup();
    Ok(HlodBakeOutput {
        proxies,
        skipped_source_entities: skipped,
    })
}

/// Apply a bake plan to scene authoring data.
///
/// Source entities receive `engine.hlod_cluster` source roles. Deterministic
/// proxy entities are inserted or replaced, making repeated bakes idempotent.
pub fn apply_hlod_bake_to_scene(
    scene: &mut Scene,
    output: &HlodBakeOutput,
    settings: HlodBakeSettings,
) -> Result<(), CookError> {
    settings.validate()?;
    let mut source_to_cluster = BTreeMap::new();
    for proxy in &output.proxies {
        for source in &proxy.source_entities {
            if source_to_cluster
                .insert(source.clone(), proxy.cluster_id.clone())
                .is_some()
            {
                return Err(CookError::InvalidAsset(format!(
                    "HLOD source entity '{source}' belongs to more than one proxy"
                )));
            }
        }
    }
    for entity in &mut scene.entities {
        let old_source_role = entity
            .components
            .get("engine.hlod_cluster")
            .and_then(|component| component.fields.get("role"))
            .is_some_and(
                |role| matches!(role, Value::Str(value) if value.eq_ignore_ascii_case("source")),
            );
        if old_source_role {
            entity.components.remove("engine.hlod_cluster");
        }
        if let Some(cluster_id) = source_to_cluster.get(&entity.persistent_id) {
            entity.components.insert(
                "engine.hlod_cluster".into(),
                hlod_component(cluster_id, "source", 0.0, 0.0),
            );
        }
    }

    scene.entities.retain(|entity| {
        !entity.persistent_id.starts_with(HLOD_PROXY_PREFIX)
            && !entity
                .components
                .get("engine.hlod_cluster")
                .and_then(|component| component.fields.get("role"))
                .is_some_and(
                    |role| matches!(role, Value::Str(value) if value.eq_ignore_ascii_case("proxy")),
                )
    });
    scene
        .dependencies
        .retain(|dependency| !dependency.id.starts_with("mesh-hlod-proxy-"));
    for proxy in &output.proxies {
        scene.entities.push(proxy_entity(proxy, settings));
        if !scene
            .dependencies
            .iter()
            .any(|dependency| dependency == &proxy.proxy_mesh_asset)
        {
            scene.dependencies.push(proxy.proxy_mesh_asset.clone());
        }
    }
    scene
        .entities
        .sort_by(|left, right| left.persistent_id.cmp(&right.persistent_id));
    scene
        .dependencies
        .sort_by(|left, right| left.id.cmp(&right.id));
    Ok(())
}

/// Write every baked proxy through the normal cooked-mesh artifact contract.
pub fn write_hlod_proxy_artifacts(
    output: &HlodBakeOutput,
    cooked_directory: &Path,
) -> Result<Vec<CookResult>, CookError> {
    let mut results = Vec::with_capacity(output.proxies.len());
    for proxy in &output.proxies {
        let payload = bincode::serialize(&proxy.mesh)
            .map_err(|error| CookError::InvalidAsset(error.to_string()))?;
        let path = cooked_directory.join(format!("{}.cooked", proxy.proxy_mesh_asset.id));
        let mut result = write_cooked_artifact(
            &path,
            AssetType::Mesh.kind_code(),
            &payload,
            SchemaVersion::new(0, 1, 0),
        )?;
        result.asset_id = proxy.proxy_mesh_asset.id.clone();
        result.source_path = PathBuf::from(format!("generated://hlod/{}", proxy.cluster_id));
        results.push(result);
    }
    Ok(results)
}

fn bake_cluster(
    key: &ClusterKey,
    chunk_index: usize,
    sources: &[&HlodBakeSource],
    settings: HlodBakeSettings,
) -> Result<HlodProxyBake, CookError> {
    let mut world_min = Vec3::splat(f32::INFINITY);
    let mut world_max = Vec3::splat(f32::NEG_INFINITY);
    for source in sources {
        let (min, max) = transformed_bounds(source)?;
        world_min = world_min.min(min);
        world_max = world_max.max(max);
    }
    let origin = (world_min + world_max) * 0.5;
    let mut merged = MeshData {
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        bounds: (Vec3::ZERO, Vec3::ZERO),
        joints: Vec::new(),
        weights: Vec::new(),
    };
    for source in sources {
        append_transformed_mesh(&mut merged, source, origin)?;
    }
    recompute_bounds(&mut merged)?;
    let source_triangle_count = merged.indices.len() / 3;
    let mesh = simplify_vertex_clusters(&merged, settings.target_vertex_ratio);
    let proxy_triangle_count = mesh.indices.len() / 3;
    let cluster_id = deterministic_cluster_id(key, chunk_index, sources);
    let proxy_entity_id = format!("{HLOD_PROXY_PREFIX}{cluster_id}");
    let proxy_mesh_asset = AssetId::new(format!("mesh-{proxy_entity_id}"));
    Ok(HlodProxyBake {
        cluster_id,
        proxy_entity_id,
        proxy_mesh_asset,
        material_asset: sources[0].material_asset.clone(),
        render_layer: sources[0].render_layer.clone(),
        source_entities: sources
            .iter()
            .map(|source| source.entity_id.clone())
            .collect(),
        origin,
        mesh,
        source_triangle_count,
        proxy_triangle_count,
    })
}

fn append_transformed_mesh(
    destination: &mut MeshData,
    source: &HlodBakeSource,
    origin: Vec3,
) -> Result<(), CookError> {
    let base = u32::try_from(destination.positions.len())
        .map_err(|_| CookError::InvalidAsset("HLOD proxy exceeds u32 vertex indexing".into()))?;
    let normal_matrix = source.world_transform.inverse().transpose();
    for (index, position) in source.mesh.positions.iter().copied().enumerate() {
        destination
            .positions
            .push(source.world_transform.transform_point3(position) - origin);
        let normal = source.mesh.normals.get(index).copied().unwrap_or(Vec3::Y);
        destination
            .normals
            .push(normal_matrix.transform_vector3(normal).normalize_or_zero());
        destination
            .uvs
            .push(source.mesh.uvs.get(index).copied().unwrap_or(Vec2::ZERO));
    }
    destination.indices.extend(
        source
            .mesh
            .indices
            .iter()
            .map(|index| base.saturating_add(*index)),
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct VertexAccumulator {
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
    count: u32,
}

fn simplify_vertex_clusters(mesh: &MeshData, ratio: f32) -> MeshData {
    let target =
        ((mesh.positions.len() as f32 * ratio).round() as usize).clamp(3, mesh.positions.len());
    if target >= mesh.positions.len() {
        return mesh.clone();
    }
    let (min, max) = mesh.bounds;
    let resolution = (target as f32).cbrt().ceil().max(1.0);
    let cell = (max - min).max(Vec3::splat(1.0e-5)) / resolution;
    let mut keys = Vec::with_capacity(mesh.positions.len());
    let mut accumulators = BTreeMap::<(i32, i32, i32), VertexAccumulator>::new();
    for (index, position) in mesh.positions.iter().copied().enumerate() {
        let coordinate = ((position - min) / cell).floor();
        let key = (
            coordinate.x as i32,
            coordinate.y as i32,
            coordinate.z as i32,
        );
        keys.push(key);
        let accumulator = accumulators.entry(key).or_default();
        accumulator.position += position;
        accumulator.normal += mesh.normals.get(index).copied().unwrap_or(Vec3::Y);
        accumulator.uv += mesh.uvs.get(index).copied().unwrap_or(Vec2::ZERO);
        accumulator.count += 1;
    }
    let mut key_to_index = BTreeMap::new();
    let mut positions = Vec::with_capacity(accumulators.len());
    let mut normals = Vec::with_capacity(accumulators.len());
    let mut uvs = Vec::with_capacity(accumulators.len());
    for (key, accumulator) in accumulators {
        key_to_index.insert(key, positions.len() as u32);
        let reciprocal = 1.0 / accumulator.count as f32;
        positions.push(accumulator.position * reciprocal);
        normals.push((accumulator.normal * reciprocal).normalize_or_zero());
        uvs.push(accumulator.uv * reciprocal);
    }
    let remap = keys.iter().map(|key| key_to_index[key]).collect::<Vec<_>>();
    let mut indices = Vec::new();
    let mut unique_triangles = BTreeSet::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let mapped = [
            remap[triangle[0] as usize],
            remap[triangle[1] as usize],
            remap[triangle[2] as usize],
        ];
        if mapped[0] == mapped[1] || mapped[1] == mapped[2] || mapped[0] == mapped[2] {
            continue;
        }
        let mut canonical = mapped;
        canonical.sort_unstable();
        if unique_triangles.insert(canonical) {
            indices.extend_from_slice(&mapped);
        }
    }
    if indices.is_empty() {
        return mesh.clone();
    }
    let mut simplified = MeshData {
        positions,
        normals,
        uvs,
        indices,
        bounds: (Vec3::ZERO, Vec3::ZERO),
        joints: Vec::new(),
        weights: Vec::new(),
    };
    if recompute_bounds(&mut simplified).is_err() {
        return mesh.clone();
    }
    simplified
}

fn validate_source(source: &HlodBakeSource) -> Result<(), CookError> {
    let mesh = &source.mesh;
    if source.entity_id.trim().is_empty()
        || source.mesh_asset.id.trim().is_empty()
        || source.material_asset.id.trim().is_empty()
        || source.render_layer.trim().is_empty()
        || !source.world_transform.is_finite()
        || source.world_transform.determinant().abs() <= 1.0e-8
        || mesh.positions.is_empty()
        || mesh.indices.is_empty()
        || !mesh.indices.len().is_multiple_of(3)
        || (!mesh.normals.is_empty() && mesh.normals.len() != mesh.positions.len())
        || (!mesh.uvs.is_empty() && mesh.uvs.len() != mesh.positions.len())
        || !mesh.joints.is_empty()
        || !mesh.weights.is_empty()
        || mesh
            .indices
            .iter()
            .any(|index| *index as usize >= mesh.positions.len())
        || !mesh.positions.iter().all(|position| position.is_finite())
        || !mesh.normals.iter().all(|normal| normal.is_finite())
        || !mesh.uvs.iter().all(|uv| uv.is_finite())
    {
        return Err(CookError::InvalidAsset(format!(
            "entity '{}' is not a finite, indexed, static HLOD mesh source",
            source.entity_id
        )));
    }
    Ok(())
}

fn transformed_bounds(source: &HlodBakeSource) -> Result<(Vec3, Vec3), CookError> {
    validate_source(source)?;
    let (min, max) = source.mesh.bounds;
    let mut world_min = Vec3::splat(f32::INFINITY);
    let mut world_max = Vec3::splat(f32::NEG_INFINITY);
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                let point = source.world_transform.transform_point3(Vec3::new(x, y, z));
                world_min = world_min.min(point);
                world_max = world_max.max(point);
            }
        }
    }
    if !world_min.is_finite() || !world_max.is_finite() {
        return Err(CookError::InvalidAsset(format!(
            "entity '{}' produced non-finite world bounds",
            source.entity_id
        )));
    }
    Ok((world_min, world_max))
}

fn recompute_bounds(mesh: &mut MeshData) -> Result<(), CookError> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for position in &mesh.positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    if !min.is_finite() || !max.is_finite() {
        return Err(CookError::InvalidAsset(
            "HLOD proxy has no finite positions".into(),
        ));
    }
    mesh.bounds = (min, max);
    Ok(())
}

fn deterministic_cluster_id(
    key: &ClusterKey,
    chunk_index: usize,
    sources: &[&HlodBakeSource],
) -> String {
    let mut digest = Sha256::new();
    digest.update(key.x.to_le_bytes());
    digest.update(key.y.to_le_bytes());
    digest.update(key.z.to_le_bytes());
    digest.update(key.material.as_bytes());
    digest.update([0]);
    digest.update(key.render_layer.as_bytes());
    digest.update((chunk_index as u64).to_le_bytes());
    for source in sources {
        digest.update(source.entity_id.as_bytes());
        digest.update([0]);
        digest.update(source.mesh_asset.id.as_bytes());
    }
    let digest = digest.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]
    )
}

fn grid_coordinate(value: f32, cell_size: f32) -> i64 {
    (value / cell_size).floor() as i64
}

fn scene_world_transforms(scene: &Scene) -> Result<BTreeMap<PersistentId, Mat4>, CookError> {
    let locals = scene
        .entities
        .iter()
        .map(|entity| {
            (
                entity.persistent_id.clone(),
                (entity.parent.clone(), scene_local_transform(entity)),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut resolved = BTreeMap::new();
    let ids = locals.keys().cloned().collect::<Vec<_>>();
    for id in ids {
        resolve_world_transform(&id, &locals, &mut resolved, &mut BTreeSet::new())?;
    }
    Ok(resolved)
}

fn resolve_world_transform(
    entity_id: &PersistentId,
    locals: &BTreeMap<PersistentId, (Option<PersistentId>, Mat4)>,
    resolved: &mut BTreeMap<PersistentId, Mat4>,
    visiting: &mut BTreeSet<PersistentId>,
) -> Result<Mat4, CookError> {
    if let Some(transform) = resolved.get(entity_id) {
        return Ok(*transform);
    }
    if !visiting.insert(entity_id.clone()) {
        return Err(CookError::InvalidAsset(format!(
            "HLOD scene transform hierarchy contains a cycle at '{entity_id}'"
        )));
    }
    let (parent, local) = locals.get(entity_id).cloned().ok_or_else(|| {
        CookError::InvalidAsset(format!(
            "HLOD scene transform hierarchy references missing entity '{entity_id}'"
        ))
    })?;
    let world = if let Some(parent) = parent {
        resolve_world_transform(&parent, locals, resolved, visiting)? * local
    } else {
        local
    };
    visiting.remove(entity_id);
    resolved.insert(entity_id.clone(), world);
    Ok(world)
}

fn scene_local_transform(entity: &EntityRecord) -> Mat4 {
    let Some(transform) = entity.components.get("engine.transform") else {
        return Mat4::IDENTITY;
    };
    let translation = match transform.fields.get("translation") {
        Some(Value::Vec3(value)) => Vec3::from_array(*value),
        _ => Vec3::ZERO,
    };
    let rotation = match transform.fields.get("rotation") {
        Some(Value::Quat(value)) => glam::Quat::from_array(*value),
        _ => glam::Quat::IDENTITY,
    };
    let scale = match transform.fields.get("scale") {
        Some(Value::Vec3(value)) => Vec3::from_array(*value),
        _ => Vec3::ONE,
    };
    Mat4::from_scale_rotation_translation(scale, rotation, translation)
}

fn component(fields: BTreeMap<String, Value>) -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields,
    }
}

fn hlod_component(
    cluster_id: &str,
    role: &str,
    activation_distance: f32,
    cull_distance: f32,
) -> ComponentRecord {
    component(BTreeMap::from([
        ("cluster_id".into(), Value::Str(cluster_id.into())),
        ("role".into(), Value::Str(role.into())),
        (
            "activation_distance".into(),
            Value::Float32(activation_distance),
        ),
        ("cull_distance".into(), Value::Float32(cull_distance)),
    ]))
}

fn proxy_entity(proxy: &HlodProxyBake, settings: HlodBakeSettings) -> EntityRecord {
    let (min, max) = proxy.mesh.bounds;
    let center = (min + max) * 0.5;
    let half_extents = (max - min) * 0.5;
    EntityRecord {
        persistent_id: proxy.proxy_entity_id.clone(),
        parent: None,
        name: Some(format!("HLOD Proxy {}", proxy.cluster_id)),
        enabled: true,
        components: BTreeMap::from([
            (
                "engine.transform".into(),
                component(BTreeMap::from([
                    ("translation".into(), Value::Vec3(proxy.origin.to_array())),
                    ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
                    ("scale".into(), Value::Vec3([1.0; 3])),
                ])),
            ),
            (
                "engine.renderable".into(),
                component(BTreeMap::from([
                    ("mesh".into(), Value::Asset(proxy.proxy_mesh_asset.clone())),
                    (
                        "material".into(),
                        Value::Asset(proxy.material_asset.clone()),
                    ),
                    ("visible".into(), Value::Bool(true)),
                    ("cast_shadows".into(), Value::Bool(true)),
                    (
                        "render_layer".into(),
                        Value::Enum(proxy.render_layer.clone()),
                    ),
                ])),
            ),
            (
                "engine.bounds".into(),
                component(BTreeMap::from([
                    ("center".into(), Value::Vec3(center.to_array())),
                    ("half_extents".into(), Value::Vec3(half_extents.to_array())),
                ])),
            ),
            (
                "engine.hlod_cluster".into(),
                hlod_component(
                    &proxy.cluster_id,
                    "proxy",
                    settings.activation_distance,
                    settings.cull_distance,
                ),
            ),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::create_test_cube;
    use engine_scene::{DiagnosticsPolicy, SceneSettings};
    use engine_serialize::EngineVersion;

    fn source(id: &str, translation: Vec3, material: &str) -> HlodBakeSource {
        HlodBakeSource {
            entity_id: id.into(),
            mesh_asset: AssetId::new(format!("mesh-{id}")),
            material_asset: AssetId::new(material),
            render_layer: "Default".into(),
            world_transform: Mat4::from_translation(translation),
            mesh: create_test_cube(),
        }
    }

    fn empty_scene(source_ids: &[&str]) -> Scene {
        Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: EngineVersion::from("0.1.0"),
            scene_id: "scene".into(),
            name: "HLOD".into(),
            entities: source_ids
                .iter()
                .map(|id| EntityRecord {
                    persistent_id: (*id).into(),
                    parent: None,
                    name: None,
                    enabled: true,
                    components: BTreeMap::new(),
                })
                .collect(),
            scene_settings: SceneSettings::default(),
            dependencies: Vec::new(),
            diagnostics_policy: DiagnosticsPolicy::Strict,
        }
    }

    #[test]
    fn clustering_is_deterministic_and_builds_a_reduced_proxy() {
        let sources = vec![
            source("a", Vec3::ZERO, "mat"),
            source("b", Vec3::new(2.0, 0.0, 0.0), "mat"),
            source("c", Vec3::new(4.0, 0.0, 0.0), "mat"),
        ];
        let settings = HlodBakeSettings {
            target_vertex_ratio: 0.2,
            ..HlodBakeSettings::default()
        };
        let first = bake_hlod_proxies(&sources, settings).unwrap();
        let second = bake_hlod_proxies(&sources, settings).unwrap();
        assert_eq!(first.proxies.len(), 1);
        assert_eq!(first.proxies[0].cluster_id, second.proxies[0].cluster_id);
        assert_eq!(first.proxies[0].source_entities, vec!["a", "b", "c"]);
        assert!(first.proxies[0].proxy_triangle_count > 0);
        assert!(first.proxies[0].proxy_triangle_count <= first.proxies[0].source_triangle_count);
    }

    #[test]
    fn material_and_spatial_boundaries_split_clusters() {
        let sources = vec![
            source("a", Vec3::ZERO, "mat-a"),
            source("b", Vec3::ONE, "mat-b"),
            source("c", Vec3::splat(100.0), "mat-a"),
        ];
        let output = bake_hlod_proxies(
            &sources,
            HlodBakeSettings {
                minimum_sources_per_cluster: 2,
                ..HlodBakeSettings::default()
            },
        )
        .unwrap();
        assert!(output.proxies.is_empty());
        assert_eq!(output.skipped_source_entities.len(), 3);
    }

    #[test]
    fn scene_bake_rejects_missing_transform_parents() {
        let mut scene = empty_scene(&["child"]);
        scene.entities[0].parent = Some("missing-parent".into());
        let error = bake_hlod_scene(&scene, HlodBakeSettings::default(), |_| {
            Ok(create_test_cube())
        })
        .unwrap_err();
        assert!(error.to_string().contains("missing-parent"));
    }

    #[test]
    fn scene_application_is_idempotent_and_artifacts_use_mesh_contract() {
        let sources = vec![
            source("a", Vec3::ZERO, "mat"),
            source("b", Vec3::ONE, "mat"),
        ];
        let settings = HlodBakeSettings::default();
        let output = bake_hlod_proxies(&sources, settings).unwrap();
        let mut scene = empty_scene(&["a", "b"]);
        apply_hlod_bake_to_scene(&mut scene, &output, settings).unwrap();
        apply_hlod_bake_to_scene(&mut scene, &output, settings).unwrap();
        assert_eq!(
            scene
                .entities
                .iter()
                .filter(|entity| entity.persistent_id.starts_with(HLOD_PROXY_PREFIX))
                .count(),
            1
        );
        assert!(scene.entities[0]
            .components
            .contains_key("engine.hlod_cluster"));

        let directory = tempfile::tempdir().unwrap();
        let results = write_hlod_proxy_artifacts(&output, directory.path()).unwrap();
        assert_eq!(results.len(), 1);
        let artifact = super::super::read_cooked_artifact(&results[0].output_path).unwrap();
        let mesh = super::super::decode_cooked_mesh(&artifact).unwrap();
        assert!(!mesh.indices.is_empty());

        apply_hlod_bake_to_scene(&mut scene, &HlodBakeOutput::default(), settings).unwrap();
        assert!(scene
            .entities
            .iter()
            .all(|entity| !entity.persistent_id.starts_with(HLOD_PROXY_PREFIX)));
        assert!(scene
            .entities
            .iter()
            .all(|entity| !entity.components.contains_key("engine.hlod_cluster")));
        assert!(scene
            .dependencies
            .iter()
            .all(|dependency| !dependency.id.starts_with("mesh-hlod-proxy-")));
    }
}
