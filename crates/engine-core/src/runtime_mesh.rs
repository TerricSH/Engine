//! Runtime dynamic mesh registration (ENG-20).
//!
//! Cooked meshes flow through the [`AssetRegistry`](engine_asset::AssetRegistry)
//! as typed [`MeshUpload`] assets and are uploaded to the GPU per frame by
//! `sync_render_assets`. This module lets Rust systems create, update, and
//! destroy meshes *at runtime* — the native foundation for terrain chunks
//! (ENG-T0) and game-side procedural geometry — while reusing that exact same
//! registry → sync → backend path.
//!
//! # Namespace and asset-ID safety
//!
//! Every runtime mesh is registered under a derived asset ID of the form
//! `runtime-mesh-{name}`. The prefix keeps the ID a single portable path
//! component (unlike a `runtime/` prefix, which
//! [`engine_asset::validate_asset_id`] rejects), so registry bookkeeping and
//! diagnostics behave exactly like cooked assets. Collision rules:
//!
//! - [`EngineRuntime::create_runtime_mesh`] refuses a name whose derived ID
//!   is already present in the registry as a foreign (e.g. cooked) asset.
//! - [`EngineRuntime::validate_cooked_batch`] rejects any cooked asset whose
//!   ID names a live runtime mesh, so neither replace-mode batch swaps nor
//!   additive installs can overwrite — and later unload — a runtime mesh.
//! - Additive/streamed installs already treat a differing payload under the
//!   same ID as a conflict, so they can never silently replace a runtime
//!   mesh either.
//!
//! # Handle lifecycle
//!
//! Calls return a [`RuntimeMeshHandle`] (slot + generation). Unknown slots
//! and generation mismatches (stale handles after destroy/re-create) are
//! reported as [`RuntimeMeshError`] values, never panics.
//!
//! # GPU frame safety
//!
//! [`EngineRuntime::destroy_runtime_mesh`] removes the typed registry entry
//! immediately (so the next frame can no longer reference it). The canonical
//! render-asset synchronizer notices the missing typed entry at the next frame
//! boundary and removes the backend object. The Vulkan backend then performs
//! its own `wait_idle` before freeing buffers, so a mesh referenced by an
//! in-flight frame is never freed mid-frame. Re-creating a mesh under the same
//! name before synchronization leaves one live registry entry, so the next
//! upload replaces the old GPU buffers without a conflicting removal.
//!
//! # Updates
//!
//! - [`EngineRuntime::update_runtime_mesh`] replaces the full vertex/index
//!   payload; bounds are recomputed (or taken from the descriptor).
//! - [`EngineRuntime::update_runtime_mesh_vertices`] edits a contiguous
//!   vertex range in place for LOD morphing / deformation. Index data is
//!   intentionally *not* partially updatable: index-count changes alter
//!   draw topology and must go through a full update. Bounds are not
//!   recomputed by partial updates.
//!
//! All uploads use the portable 32-byte PBR vertex format
//! ([`MeshVertexFormat::Pbr32`]); skinned runtime meshes are out of scope.

use std::collections::BTreeMap;
use std::fmt;

use engine_asset::mesh::{mesh_data_to_upload_bytes, MeshData};
use engine_renderer::{
    AssetId, AxisAlignedBox, IndexFormat, MeshUpload, MeshVertexFormat, Pbr32Vertex,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use glam::{Vec2, Vec3};

use crate::EngineRuntime;

/// Asset-ID prefix reserved for runtime-registered meshes.
pub const RUNTIME_MESH_ID_PREFIX: &str = "runtime-mesh-";

// ---------------------------------------------------------------------------
// RuntimeMeshHandle
// ---------------------------------------------------------------------------

/// Opaque handle to a registered runtime mesh.
///
/// The `generation` distinguishes a live mesh from a destroyed (and possibly
/// re-created) one occupying the same slot, so stale handles are detected
/// instead of silently addressing a different mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeMeshHandle {
    slot: u32,
    generation: u32,
}

impl RuntimeMeshHandle {
    /// Table slot identifying the mesh within one runtime.
    pub fn slot(self) -> u32 {
        self.slot
    }

    /// Generation of the slot when the handle was issued.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

// ---------------------------------------------------------------------------
// RuntimeMeshDescriptor
// ---------------------------------------------------------------------------

/// Vertex/index payload for creating or fully replacing a runtime mesh.
///
/// Normals and UVs may be empty (defaults are substituted during conversion);
/// when present their lengths must match `positions`. When `bounds` is
/// `None` a tight bounding box is computed from the positions.
#[derive(Clone, Debug, Default)]
pub struct RuntimeMeshDescriptor {
    /// Vertex positions.
    pub positions: Vec<Vec3>,
    /// Per-vertex normals (empty or one per position).
    pub normals: Vec<Vec3>,
    /// Per-vertex texture coordinates (empty or one per position).
    pub uvs: Vec<Vec2>,
    /// Triangle-list indices; non-empty and a multiple of three.
    pub indices: Vec<u32>,
    /// Optional explicit bounds (min, max); computed from positions when `None`.
    pub bounds: Option<(Vec3, Vec3)>,
}

impl RuntimeMeshDescriptor {
    /// A descriptor with only positions and indices; normals/UVs default and
    /// bounds are computed.
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        Self {
            positions,
            indices,
            ..Self::default()
        }
    }

    /// Validate the payload and convert it to the GPU-ready upload layout.
    ///
    /// The conversion reuses
    /// [`mesh_data_to_upload_bytes`], the exact layout used by cooked mesh
    /// decode, so runtime and cooked meshes are indistinguishable downstream.
    fn to_upload(&self, mesh_id: AssetId) -> Result<MeshUpload, RuntimeMeshError> {
        if self.positions.is_empty() {
            return Err(RuntimeMeshError::InvalidGeometry(
                "mesh must contain at least one vertex".to_string(),
            ));
        }
        if self.positions.len() > u32::MAX as usize {
            return Err(RuntimeMeshError::InvalidGeometry(format!(
                "vertex count {} exceeds the u32 upload contract",
                self.positions.len()
            )));
        }
        if !self.positions.iter().all(|position| position.is_finite()) {
            return Err(RuntimeMeshError::InvalidGeometry(
                "vertex positions must be finite".to_string(),
            ));
        }
        for (field, len) in [("normals", self.normals.len()), ("uvs", self.uvs.len())] {
            if len != 0 && len != self.positions.len() {
                return Err(RuntimeMeshError::InvalidGeometry(format!(
                    "{field} must be empty or match the position count ({}), got {len}",
                    self.positions.len()
                )));
            }
        }
        if self.indices.is_empty() || !self.indices.len().is_multiple_of(3) {
            return Err(RuntimeMeshError::InvalidGeometry(format!(
                "triangle-list indices must be non-empty and a multiple of three, got {}",
                self.indices.len()
            )));
        }
        let vertex_count = self.positions.len() as u32;
        if let Some(index) = self.indices.iter().find(|index| **index >= vertex_count) {
            return Err(RuntimeMeshError::InvalidGeometry(format!(
                "index {index} is out of range for {vertex_count} vertices"
            )));
        }

        let bounds = match self.bounds {
            Some((min, max)) => {
                if !min.is_finite() || !max.is_finite() || min.cmpgt(max).any() {
                    return Err(RuntimeMeshError::InvalidGeometry(
                        "bounds must be finite and min must not exceed max".to_string(),
                    ));
                }
                (min, max)
            }
            None => compute_bounds(&self.positions),
        };

        let mesh = MeshData {
            positions: self.positions.clone(),
            normals: self.normals.clone(),
            uvs: self.uvs.clone(),
            indices: self.indices.clone(),
            bounds,
            joints: Vec::new(),
            weights: Vec::new(),
        };
        let (vertex_bytes, index_bytes, index_count, _) = mesh_data_to_upload_bytes(&mesh);
        let content_hash =
            engine_asset::compute_content_hash(&[vertex_bytes.as_slice(), index_bytes.as_slice()]);
        Ok(MeshUpload {
            mesh_id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count,
            vertex_bytes,
            index_format: IndexFormat::U32,
            index_count,
            index_bytes,
            bounds: AxisAlignedBox {
                min: bounds.0.to_array(),
                max: bounds.1.to_array(),
            },
            content_hash,
        })
    }
}

fn compute_bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for position in positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    (min, max)
}

// ---------------------------------------------------------------------------
// RuntimeMeshError
// ---------------------------------------------------------------------------

/// Errors returned by the runtime mesh API. Every failure is a value, never
/// a panic, so game systems can surface them through their own diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeMeshError {
    /// The name is empty or produces an ID that violates portable asset-ID
    /// rules.
    InvalidName(String),
    /// A live runtime mesh already uses this name.
    DuplicateName(String),
    /// The derived asset ID is occupied by a foreign asset (e.g. a cooked
    /// mesh) in the shared registry.
    AssetIdConflict(String),
    /// The handle's slot was never issued by this runtime.
    UnknownHandle {
        /// The slot carried by the rejected handle.
        slot: u32,
    },
    /// The handle's slot exists but its generation moved on: the mesh it
    /// referred to was destroyed (and possibly re-created).
    StaleHandle {
        /// The slot carried by the rejected handle.
        slot: u32,
    },
    /// The mesh payload violates the renderer upload contract.
    InvalidGeometry(String),
    /// A partial vertex update is empty or exceeds the mesh's vertex range.
    InvalidUpdateRange(String),
    /// The registry entry backing a live handle is missing or has the wrong
    /// type — only possible when the registry was mutated behind the API.
    RegistryMismatch(String),
}

impl fmt::Display for RuntimeMeshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeMeshError::InvalidName(detail)
            | RuntimeMeshError::DuplicateName(detail)
            | RuntimeMeshError::AssetIdConflict(detail)
            | RuntimeMeshError::InvalidGeometry(detail)
            | RuntimeMeshError::InvalidUpdateRange(detail)
            | RuntimeMeshError::RegistryMismatch(detail) => formatter.write_str(detail),
            RuntimeMeshError::UnknownHandle { slot } => {
                write!(formatter, "unknown runtime mesh handle (slot {slot})")
            }
            RuntimeMeshError::StaleHandle { slot } => write!(
                formatter,
                "stale runtime mesh handle (slot {slot}): the mesh was destroyed"
            ),
        }
    }
}

impl std::error::Error for RuntimeMeshError {}

// ---------------------------------------------------------------------------
// RuntimeMeshMemory
// ---------------------------------------------------------------------------

/// Aggregate CPU-side memory accounting for live runtime meshes.
///
/// Reported through [`EngineRuntime::runtime_mesh_memory`] and the
/// [`crate::RuntimeDiagnostics`] snapshot. GPU buffers mirror these payloads
/// one-for-one on the next frame sync, so the totals also bound runtime-mesh
/// GPU memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeMeshMemory {
    /// Number of live runtime meshes.
    pub mesh_count: usize,
    /// Total vertices across all live runtime meshes.
    pub vertex_count: u64,
    /// Total indices across all live runtime meshes.
    pub index_count: u64,
    /// Total interleaved vertex payload bytes.
    pub vertex_bytes: u64,
    /// Total packed index payload bytes.
    pub index_bytes: u64,
}

impl RuntimeMeshMemory {
    /// `vertex_bytes + index_bytes`.
    pub fn total_bytes(&self) -> u64 {
        self.vertex_bytes + self.index_bytes
    }
}

// ---------------------------------------------------------------------------
// RuntimeMeshTable
// ---------------------------------------------------------------------------

pub(crate) struct RuntimeMeshEntry {
    asset_id: AssetId,
    vertex_count: u32,
    index_count: u32,
    vertex_bytes: usize,
    index_bytes: usize,
}

struct RuntimeMeshSlot {
    generation: u32,
    entry: Option<RuntimeMeshEntry>,
}

/// Handle table tracking live runtime meshes.
///
/// Registry bookkeeping stays in the shared [`AssetRegistry`]; this table
/// only owns handle validation, name → slot lookup, and per-mesh stats.
#[derive(Default)]
pub(crate) struct RuntimeMeshTable {
    slots: Vec<RuntimeMeshSlot>,
    free: Vec<u32>,
    by_name: BTreeMap<String, u32>,
}

impl RuntimeMeshTable {
    fn contains_asset_id(&self, id: &AssetId) -> bool {
        self.slots
            .iter()
            .filter_map(|slot| slot.entry.as_ref())
            .any(|entry| &entry.asset_id == id)
    }

    fn resolve(
        &self,
        handle: RuntimeMeshHandle,
    ) -> Result<(u32, &RuntimeMeshEntry), RuntimeMeshError> {
        let Some(slot) = self.slots.get(handle.slot as usize) else {
            return Err(RuntimeMeshError::UnknownHandle { slot: handle.slot });
        };
        // The generation moves on destroy, so a stale handle is detected
        // before the (now empty) slot contents are inspected.
        if slot.generation != handle.generation {
            return Err(RuntimeMeshError::StaleHandle { slot: handle.slot });
        }
        let Some(entry) = slot.entry.as_ref() else {
            return Err(RuntimeMeshError::UnknownHandle { slot: handle.slot });
        };
        Ok((handle.slot, entry))
    }

    fn insert(&mut self, name: &str, entry: RuntimeMeshEntry) -> RuntimeMeshHandle {
        let (slot_index, generation) = if let Some(reused) = self.free.pop() {
            let slot = &mut self.slots[reused as usize];
            debug_assert!(slot.entry.is_none());
            slot.entry = Some(entry);
            (reused, slot.generation)
        } else {
            let slot_index = u32::try_from(self.slots.len())
                .expect("runtime mesh slot count is bounded by caller budgets");
            self.slots.push(RuntimeMeshSlot {
                generation: 1,
                entry: Some(entry),
            });
            (slot_index, 1)
        };
        self.by_name.insert(name.to_string(), slot_index);
        RuntimeMeshHandle {
            slot: slot_index,
            generation,
        }
    }

    fn remove(&mut self, slot_index: u32) -> Option<RuntimeMeshEntry> {
        let slot = self.slots.get_mut(slot_index as usize)?;
        let entry = slot.entry.take()?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(slot_index);
        Some(entry)
    }

    fn memory(&self) -> RuntimeMeshMemory {
        let mut memory = RuntimeMeshMemory::default();
        for entry in self.slots.iter().filter_map(|slot| slot.entry.as_ref()) {
            memory.mesh_count += 1;
            memory.vertex_count += u64::from(entry.vertex_count);
            memory.index_count += u64::from(entry.index_count);
            memory.vertex_bytes += entry.vertex_bytes as u64;
            memory.index_bytes += entry.index_bytes as u64;
        }
        memory
    }
}

// ---------------------------------------------------------------------------
// EngineRuntime API
// ---------------------------------------------------------------------------

impl EngineRuntime {
    /// Register a new runtime mesh under `runtime-mesh-{name}`.
    ///
    /// The mesh is uploaded to the GPU on the next frame that references it,
    /// exactly like a cooked mesh. Returns a handle used by every subsequent
    /// operation; the matching [`AssetId`] for scene renderables is available
    /// through [`runtime_mesh_asset_id`](Self::runtime_mesh_asset_id).
    ///
    /// Fails with [`RuntimeMeshError::DuplicateName`] when a live runtime
    /// mesh already uses `name`, and with [`RuntimeMeshError::AssetIdConflict`]
    /// when the derived ID belongs to a foreign (e.g. cooked) asset.
    pub fn create_runtime_mesh(
        &mut self,
        name: &str,
        mesh: RuntimeMeshDescriptor,
    ) -> Result<RuntimeMeshHandle, RuntimeMeshError> {
        let asset_id = runtime_mesh_asset_id_from_name(name)?;
        if self.runtime_mesh_table.by_name.contains_key(name) {
            return Err(RuntimeMeshError::DuplicateName(format!(
                "a live runtime mesh already uses name '{name}'"
            )));
        }
        if self.asset_registry.contains(&asset_id) {
            return Err(RuntimeMeshError::AssetIdConflict(format!(
                "asset ID '{}' is already registered by a non-runtime asset; choose a different mesh name",
                asset_id.id
            )));
        }
        let upload = mesh.to_upload(asset_id.clone())?;

        let entry = RuntimeMeshEntry {
            asset_id,
            vertex_count: upload.vertex_count,
            index_count: upload.index_count,
            vertex_bytes: upload.vertex_bytes.len(),
            index_bytes: upload.index_bytes.len(),
        };
        self.asset_registry
            .insert_typed(entry.asset_id.clone(), upload);
        Ok(self.runtime_mesh_table.insert(name, entry))
    }

    /// Fully replace a runtime mesh's vertex/index payload.
    ///
    /// The handle and asset ID stay valid; the GPU sees the new payload on
    /// the next frame sync (backends dedupe unchanged content by hash, and
    /// this replacement always changes it unless the payload is identical).
    pub fn update_runtime_mesh(
        &mut self,
        handle: RuntimeMeshHandle,
        mesh: RuntimeMeshDescriptor,
    ) -> Result<(), RuntimeMeshError> {
        let (slot_index, asset_id) = {
            let (slot_index, entry) = self.runtime_mesh_table.resolve(handle)?;
            (slot_index, entry.asset_id.clone())
        };
        let upload = mesh.to_upload(asset_id)?;
        let slot = self
            .runtime_mesh_table
            .slots
            .get_mut(slot_index as usize)
            .and_then(|slot| slot.entry.as_mut())
            .ok_or(RuntimeMeshError::UnknownHandle { slot: slot_index })?;
        slot.vertex_count = upload.vertex_count;
        slot.index_count = upload.index_count;
        slot.vertex_bytes = upload.vertex_bytes.len();
        slot.index_bytes = upload.index_bytes.len();
        self.asset_registry
            .insert_typed(slot.asset_id.clone(), upload);
        Ok(())
    }

    /// Overwrite a contiguous range of vertices in place.
    ///
    /// `first_vertex` is a vertex index (not a byte offset) and `vertices`
    /// replaces `vertices.len()` consecutive vertices starting there. The
    /// mesh's vertex count, index data, and bounds are unchanged — deform the
    /// surface within its existing bounds, or use
    /// [`update_runtime_mesh`](Self::update_runtime_mesh) when topology or
    /// bounds change. Index data is deliberately not partially updatable.
    pub fn update_runtime_mesh_vertices(
        &mut self,
        handle: RuntimeMeshHandle,
        first_vertex: u32,
        vertices: &[Pbr32Vertex],
    ) -> Result<(), RuntimeMeshError> {
        let asset_id = self.runtime_mesh_table.resolve(handle)?.1.asset_id.clone();
        let vertex_count = self
            .asset_registry
            .get::<MeshUpload>(&asset_id)
            .ok_or_else(|| {
                RuntimeMeshError::RegistryMismatch(format!(
                    "runtime mesh '{}' is missing from the asset registry",
                    asset_id.id
                ))
            })?
            .get()
            .vertex_count;

        if vertices.is_empty() {
            return Err(RuntimeMeshError::InvalidUpdateRange(
                "partial vertex update must contain at least one vertex".to_string(),
            ));
        }
        let end_vertex = u64::from(first_vertex) + vertices.len() as u64;
        if end_vertex > u64::from(vertex_count) {
            return Err(RuntimeMeshError::InvalidUpdateRange(format!(
                "vertex range {first_vertex}..{end_vertex} exceeds the mesh's {vertex_count} vertices"
            )));
        }

        let mut upload = self
            .asset_registry
            .get::<MeshUpload>(&asset_id)
            .expect("checked above")
            .get()
            .clone();
        debug_assert_eq!(upload.vertex_format, MeshVertexFormat::Pbr32);
        let stride = MeshVertexFormat::Pbr32.stride_bytes() as usize;
        let byte_offset = first_vertex as usize * stride;
        for (index, vertex) in vertices.iter().enumerate() {
            let offset = byte_offset + index * stride;
            let target = &mut upload.vertex_bytes[offset..offset + stride];
            debug_assert_eq!(target.len(), std::mem::size_of::<Pbr32Vertex>());
            write_pbr32_vertex(target, vertex);
        }
        upload.content_hash = engine_asset::compute_content_hash(&[
            upload.vertex_bytes.as_slice(),
            upload.index_bytes.as_slice(),
        ]);
        self.asset_registry.insert_typed(asset_id, upload);
        Ok(())
    }

    /// Destroy a runtime mesh.
    ///
    /// The typed registry entry is removed immediately, so subsequent frames
    /// can no longer resolve the ID (a renderable still referencing it
    /// surfaces the usual missing-asset diagnostic). The backend GPU resource
    /// destruction is reconciled by the canonical render-asset sync at the
    /// next rendered frame boundary; see the module-level GPU frame-safety
    /// note.
    pub fn destroy_runtime_mesh(
        &mut self,
        handle: RuntimeMeshHandle,
    ) -> Result<(), RuntimeMeshError> {
        let slot_index = self.runtime_mesh_table.resolve(handle)?.0;
        let entry = self
            .runtime_mesh_table
            .remove(slot_index)
            .expect("resolve proved the entry exists");
        let name = entry
            .asset_id
            .id
            .strip_prefix(RUNTIME_MESH_ID_PREFIX)
            .unwrap_or_default()
            .to_string();
        self.runtime_mesh_table.by_name.remove(&name);
        self.asset_registry.unload(&entry.asset_id);
        Ok(())
    }

    /// The asset ID a scene renderable must reference to draw this mesh.
    ///
    /// Returns `None` for unknown or stale handles.
    pub fn runtime_mesh_asset_id(&self, handle: RuntimeMeshHandle) -> Option<AssetId> {
        self.runtime_mesh_table
            .resolve(handle)
            .ok()
            .map(|(_, entry)| entry.asset_id.clone())
    }

    /// CPU-side memory accounting for live runtime meshes; see
    /// [`RuntimeMeshMemory`].
    pub fn runtime_mesh_memory(&self) -> RuntimeMeshMemory {
        self.runtime_mesh_table.memory()
    }

    /// Number of live runtime meshes whose IDs `id`-colliding cooked assets
    /// must not overwrite. Used by cooked-batch validation.
    pub(crate) fn is_runtime_mesh_asset_id(&self, id: &AssetId) -> bool {
        self.runtime_mesh_table.contains_asset_id(id)
    }
}

/// Derive and validate the registry asset ID for a runtime mesh name.
fn runtime_mesh_asset_id_from_name(name: &str) -> Result<AssetId, RuntimeMeshError> {
    if name.is_empty() {
        return Err(RuntimeMeshError::InvalidName(
            "runtime mesh name must not be empty".to_string(),
        ));
    }
    let id = AssetId::new(format!("{RUNTIME_MESH_ID_PREFIX}{name}"));
    engine_asset::validate_asset_id(&id).map_err(|error| {
        RuntimeMeshError::InvalidName(format!(
            "runtime mesh name '{name}' produces an invalid asset ID: {error}"
        ))
    })?;
    Ok(id)
}

/// Serialize one [`Pbr32Vertex`] into the exact 32-byte interleaved layout
/// produced by [`mesh_data_to_upload_bytes`].
fn write_pbr32_vertex(target: &mut [u8], vertex: &Pbr32Vertex) {
    let mut cursor = 0;
    for value in vertex
        .position
        .iter()
        .chain(vertex.normal.iter())
        .chain(vertex.uv0.iter())
    {
        target[cursor..cursor + 4].copy_from_slice(&value.to_ne_bytes());
        cursor += 4;
    }
}

/// Diagnostic emitted when a cooked batch names a live runtime mesh ID.
pub(crate) fn runtime_mesh_conflict_diagnostic(id: &AssetId, kind: &str) -> Diagnostic {
    Diagnostic::new(
        "AS0003",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        format!(
            "cooked {kind} asset '{}' conflicts with a live runtime mesh; \
             runtime mesh IDs are reserved until the mesh is destroyed",
            id.id
        ),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_renderer::{BackendRenderer, FrameStats, ResourceRemoval};
    use std::sync::{Arc, Mutex};

    fn triangle() -> RuntimeMeshDescriptor {
        RuntimeMeshDescriptor {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            normals: vec![Vec3::Z; 3],
            uvs: vec![Vec2::ZERO; 3],
            indices: vec![0, 1, 2],
            bounds: None,
        }
    }

    fn quad() -> RuntimeMeshDescriptor {
        RuntimeMeshDescriptor {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            normals: vec![Vec3::Z; 4],
            uvs: vec![Vec2::ZERO; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            bounds: Some((Vec3::ZERO, Vec3::ONE)),
        }
    }

    fn runtime() -> EngineRuntime {
        EngineRuntime::new(crate::EngineConfig::default())
    }

    fn registered_upload(runtime: &EngineRuntime, handle: RuntimeMeshHandle) -> MeshUpload {
        let id = runtime
            .runtime_mesh_asset_id(handle)
            .expect("live handle resolves an asset ID");
        runtime
            .asset_registry()
            .get::<MeshUpload>(&id)
            .expect("runtime mesh is registered")
            .get()
            .clone()
    }

    // ── Create / lifecycle ──────────────────────────────────────────────

    #[test]
    fn create_registers_mesh_and_reports_memory() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("terrain-chunk-0", triangle())
            .expect("create");

        let id = runtime
            .runtime_mesh_asset_id(handle)
            .expect("handle resolves");
        assert_eq!(id.id, "runtime-mesh-terrain-chunk-0");

        let upload = registered_upload(&runtime, handle);
        assert_eq!(upload.mesh_id, id);
        assert_eq!(upload.vertex_format, MeshVertexFormat::Pbr32);
        assert_eq!(upload.vertex_count, 3);
        assert_eq!(upload.index_count, 3);
        assert_eq!(upload.vertex_bytes.len(), 3 * 32);
        assert_eq!(upload.index_bytes.len(), 3 * 4);
        // Bounds were computed from the positions.
        assert_eq!(upload.bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(upload.bounds.max, [1.0, 1.0, 0.0]);

        let memory = runtime.runtime_mesh_memory();
        assert_eq!(memory.mesh_count, 1);
        assert_eq!(memory.vertex_count, 3);
        assert_eq!(memory.index_count, 3);
        assert_eq!(memory.vertex_bytes, 3 * 32);
        assert_eq!(memory.index_bytes, 3 * 4);
        assert_eq!(memory.total_bytes(), 3 * 32 + 3 * 4);
    }

    #[test]
    fn explicit_bounds_are_preserved() {
        let mut runtime = runtime();
        let handle = runtime.create_runtime_mesh("quad", quad()).expect("create");
        let upload = registered_upload(&runtime, handle);
        assert_eq!(upload.bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(upload.bounds.max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn create_rejects_invalid_names() {
        let mut runtime = runtime();
        for name in ["", "has/slash", "has\\backslash", ":", "..", "trail."] {
            let result = runtime.create_runtime_mesh(name, triangle());
            assert!(
                matches!(result, Err(RuntimeMeshError::InvalidName(_))),
                "name '{name}' must be rejected, got {result:?}"
            );
        }
    }

    #[test]
    fn create_rejects_duplicate_name() {
        let mut runtime = runtime();
        runtime
            .create_runtime_mesh("dup", triangle())
            .expect("create");
        let result = runtime.create_runtime_mesh("dup", triangle());
        assert_eq!(
            result,
            Err(RuntimeMeshError::DuplicateName(
                "a live runtime mesh already uses name 'dup'".to_string()
            ))
        );
    }

    #[test]
    fn create_rejects_id_occupied_by_foreign_asset() {
        let mut runtime = runtime();
        let foreign = triangle()
            .to_upload(AssetId::new("runtime-mesh-taken"))
            .unwrap();
        runtime.register_mesh_asset(foreign);

        let result = runtime.create_runtime_mesh("taken", triangle());
        assert!(
            matches!(result, Err(RuntimeMeshError::AssetIdConflict(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn create_rejects_invalid_geometry() {
        let mut runtime = runtime();

        let mut empty = triangle();
        empty.positions.clear();
        empty.indices.clear();
        assert!(matches!(
            runtime.create_runtime_mesh("empty", empty),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        let mut bad_indices = triangle();
        bad_indices.indices = vec![0, 1];
        assert!(matches!(
            runtime.create_runtime_mesh("bad-indices", bad_indices),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        let mut out_of_range = triangle();
        out_of_range.indices = vec![0, 1, 7];
        assert!(matches!(
            runtime.create_runtime_mesh("oob", out_of_range),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        let mut mismatched_normals = triangle();
        mismatched_normals.normals = vec![Vec3::Z];
        assert!(matches!(
            runtime.create_runtime_mesh("normals", mismatched_normals),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        let mut non_finite = triangle();
        non_finite.positions[1] = Vec3::new(f32::NAN, 0.0, 0.0);
        assert!(matches!(
            runtime.create_runtime_mesh("nan", non_finite),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        let mut bad_bounds = triangle();
        bad_bounds.bounds = Some((Vec3::ONE, Vec3::ZERO));
        assert!(matches!(
            runtime.create_runtime_mesh("bounds", bad_bounds),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));

        // None of the failed creates registered anything.
        assert_eq!(runtime.runtime_mesh_memory().mesh_count, 0);
    }

    // ── Full update ─────────────────────────────────────────────────────

    #[test]
    fn update_replaces_payload_and_memory() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("morph", triangle())
            .expect("create");
        let before = registered_upload(&runtime, handle);

        runtime
            .update_runtime_mesh(handle, quad())
            .expect("full update");
        let after = registered_upload(&runtime, handle);

        assert_eq!(after.mesh_id, before.mesh_id, "asset ID is stable");
        assert_eq!(after.vertex_count, 4);
        assert_eq!(after.index_count, 6);
        assert_eq!(after.vertex_bytes.len(), 4 * 32);
        assert_ne!(after.content_hash, before.content_hash);

        let memory = runtime.runtime_mesh_memory();
        assert_eq!(memory.mesh_count, 1);
        assert_eq!(memory.vertex_count, 4);
        assert_eq!(memory.index_count, 6);
        assert_eq!(memory.vertex_bytes, 4 * 32);
    }

    #[test]
    fn update_validates_geometry() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("morph", triangle())
            .expect("create");
        let mut invalid = triangle();
        invalid.indices = vec![0, 1, 9];
        assert!(matches!(
            runtime.update_runtime_mesh(handle, invalid),
            Err(RuntimeMeshError::InvalidGeometry(_))
        ));
        // The original payload is untouched by the failed update.
        assert_eq!(registered_upload(&runtime, handle).vertex_count, 3);
    }

    // ── Partial vertex update ───────────────────────────────────────────

    fn vertex(x: f32, y: f32, z: f32) -> Pbr32Vertex {
        Pbr32Vertex {
            position: [x, y, z],
            normal: [0.0, 0.0, 1.0],
            uv0: [0.0, 0.0],
        }
    }

    fn read_position(upload: &MeshUpload, vertex_index: usize) -> [f32; 3] {
        let stride = MeshVertexFormat::Pbr32.stride_bytes() as usize;
        let offset = vertex_index * stride;
        let mut position = [0.0_f32; 3];
        for (axis, slot) in position.iter_mut().enumerate() {
            let start = offset + axis * 4;
            *slot = f32::from_ne_bytes(
                upload.vertex_bytes[start..start + 4]
                    .try_into()
                    .expect("four bytes per float"),
            );
        }
        position
    }

    #[test]
    fn partial_vertex_update_rewrites_only_the_target_range() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("deform", quad())
            .expect("create");
        let before = registered_upload(&runtime, handle);

        runtime
            .update_runtime_mesh_vertices(
                handle,
                1,
                &[vertex(5.0, 0.0, 0.0), vertex(6.0, 1.0, 0.0)],
            )
            .expect("partial update");
        let after = registered_upload(&runtime, handle);

        assert_eq!(read_position(&after, 0), read_position(&before, 0));
        assert_eq!(read_position(&after, 1), [5.0, 0.0, 0.0]);
        assert_eq!(read_position(&after, 2), [6.0, 1.0, 0.0]);
        assert_eq!(read_position(&after, 3), read_position(&before, 3));
        assert_eq!(after.index_bytes, before.index_bytes);
        assert_eq!(after.bounds, before.bounds, "partial edits keep bounds");
        assert_ne!(after.content_hash, before.content_hash);
        // Counts and memory are unchanged by in-place edits.
        assert_eq!(runtime.runtime_mesh_memory().vertex_count, 4);
    }

    #[test]
    fn partial_vertex_update_rejects_invalid_ranges() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("deform", quad())
            .expect("create");

        assert!(matches!(
            runtime.update_runtime_mesh_vertices(handle, 0, &[]),
            Err(RuntimeMeshError::InvalidUpdateRange(_))
        ));
        assert!(matches!(
            runtime.update_runtime_mesh_vertices(handle, 3, &[vertex(0.0, 0.0, 0.0); 2]),
            Err(RuntimeMeshError::InvalidUpdateRange(_))
        ));
        assert!(matches!(
            runtime.update_runtime_mesh_vertices(handle, 4, &[vertex(0.0, 0.0, 0.0)]),
            Err(RuntimeMeshError::InvalidUpdateRange(_))
        ));
    }

    // ── Destroy / handle lifecycle ──────────────────────────────────────

    #[test]
    fn destroy_removes_mesh_and_zeroes_memory() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("temp", triangle())
            .expect("create");
        let id = runtime.runtime_mesh_asset_id(handle).unwrap();

        runtime.destroy_runtime_mesh(handle).expect("destroy");

        assert!(runtime.asset_registry().get::<MeshUpload>(&id).is_none());
        assert!(!runtime.asset_registry().contains(&id));
        assert!(runtime.runtime_mesh_asset_id(handle).is_none());
        let memory = runtime.runtime_mesh_memory();
        assert_eq!(memory.mesh_count, 0);
        assert_eq!(memory.total_bytes(), 0);
    }

    #[test]
    fn stale_and_unknown_handles_are_errors_not_panics() {
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("temp", triangle())
            .expect("create");
        runtime.destroy_runtime_mesh(handle).expect("destroy");

        assert_eq!(
            runtime.update_runtime_mesh(handle, triangle()),
            Err(RuntimeMeshError::StaleHandle {
                slot: handle.slot()
            })
        );
        assert_eq!(
            runtime.destroy_runtime_mesh(handle),
            Err(RuntimeMeshError::StaleHandle {
                slot: handle.slot()
            })
        );
        assert_eq!(
            runtime.update_runtime_mesh_vertices(handle, 0, &[vertex(0.0, 0.0, 0.0)]),
            Err(RuntimeMeshError::StaleHandle {
                slot: handle.slot()
            })
        );

        let bogus = RuntimeMeshHandle {
            slot: 99,
            generation: 1,
        };
        assert_eq!(
            runtime.destroy_runtime_mesh(bogus),
            Err(RuntimeMeshError::UnknownHandle { slot: 99 })
        );
        assert!(runtime.runtime_mesh_asset_id(bogus).is_none());
    }

    #[test]
    fn recreate_after_destroy_issues_a_fresh_generation() {
        let mut runtime = runtime();
        let old = runtime
            .create_runtime_mesh("cycle", triangle())
            .expect("create");
        runtime.destroy_runtime_mesh(old).expect("destroy");

        let new = runtime
            .create_runtime_mesh("cycle", quad())
            .expect("re-create under the same name");
        assert_eq!(new.slot(), old.slot(), "slots are reused");
        assert_ne!(new.generation(), old.generation());
        assert_eq!(
            runtime.update_runtime_mesh(old, triangle()),
            Err(RuntimeMeshError::StaleHandle { slot: old.slot() })
        );
        assert_eq!(registered_upload(&runtime, new).vertex_count, 4);
        // Registry reconciliation sees one live entry, so the next upload
        // replaces the buffers without issuing a stale removal first.
    }

    #[test]
    fn multiple_meshes_accumulate_memory() {
        let mut runtime = runtime();
        let a = runtime
            .create_runtime_mesh("a", triangle())
            .expect("create");
        let _b = runtime.create_runtime_mesh("b", quad()).expect("create");
        let memory = runtime.runtime_mesh_memory();
        assert_eq!(memory.mesh_count, 2);
        assert_eq!(memory.vertex_count, 3 + 4);
        assert_eq!(memory.index_count, 3 + 6);
        assert_eq!(memory.vertex_bytes, (3 + 4) * 32);
        assert_eq!(memory.index_bytes, (3 + 6) * 4);

        runtime.destroy_runtime_mesh(a).expect("destroy");
        let memory = runtime.runtime_mesh_memory();
        assert_eq!(memory.mesh_count, 1);
        assert_eq!(memory.vertex_count, 4);
    }

    // ── Cooked-batch interaction ────────────────────────────────────────

    fn cook_test_mesh(dir: &std::path::Path, id: &str) {
        let mesh = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![Vec3::Z; 3],
            uvs: vec![],
            indices: vec![0, 1, 2],
            bounds: (Vec3::ZERO, Vec3::ONE),
            joints: vec![],
            weights: vec![],
        };
        let payload = bincode::serialize(&mesh).expect("serialize mesh");
        engine_asset::cook::write_cooked_artifact(
            &dir.join(format!("{id}.cooked")),
            engine_asset::cook::AssetType::Mesh.kind_code(),
            &payload,
            engine_serialize::SchemaVersion::new(0, 1, 0),
        )
        .expect("write cooked mesh");
    }

    fn cooked_case(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engine_core_runtime_mesh_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cooked_replace_load_preserves_runtime_meshes() {
        let dir = cooked_case("replace_preserves");
        cook_test_mesh(&dir, "mesh-cooked-one");
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("terrain-chunk-0", triangle())
            .expect("create");
        let before = registered_upload(&runtime, handle);

        let report = runtime
            .load_cooked_assets(&dir)
            .expect("cooked load succeeds");
        assert_eq!(report.loaded_meshes, 1);

        assert_eq!(
            registered_upload(&runtime, handle),
            before,
            "cooked replace must not touch runtime meshes"
        );
        assert!(runtime
            .asset_registry()
            .get::<MeshUpload>(&AssetId::new("mesh-cooked-one"))
            .is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cooked_batch_naming_a_runtime_mesh_id_is_rejected() {
        let dir = cooked_case("replace_conflict");
        cook_test_mesh(&dir, "runtime-mesh-terrain-chunk-0");
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("terrain-chunk-0", triangle())
            .expect("create");
        let before = registered_upload(&runtime, handle);

        let diagnostics = runtime
            .load_cooked_assets(&dir)
            .expect_err("cooked asset colliding with a runtime mesh ID is rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "AS0003"
                    && diagnostic.message.contains("runtime-mesh-terrain-chunk-0")),
            "expected an AS0003 runtime-mesh conflict, got {diagnostics:?}"
        );
        assert_eq!(registered_upload(&runtime, handle), before);
        assert_eq!(runtime.runtime_mesh_memory().mesh_count, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn additive_install_coexists_and_conflicts_cleanly() {
        let dir = cooked_case("additive_coexists");
        cook_test_mesh(&dir, "mesh-streamed-one");
        let mut runtime = runtime();
        let handle = runtime
            .create_runtime_mesh("chunk", triangle())
            .expect("create");

        let report = runtime
            .install_cooked_assets_additive(&[dir.join("mesh-streamed-one.cooked")])
            .expect("additive install of an unrelated mesh succeeds");
        assert_eq!(report.loaded_meshes, 1);
        assert_eq!(runtime.runtime_mesh_memory().mesh_count, 1);

        // An additive install whose ID matches the live runtime mesh is a
        // conflict, not an overwrite.
        cook_test_mesh(&dir, "runtime-mesh-chunk");
        let diagnostics = runtime
            .install_cooked_assets_additive(&[dir.join("runtime-mesh-chunk.cooked")])
            .expect_err("additive conflict");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("runtime-mesh-chunk")),
            "got {diagnostics:?}"
        );
        assert_eq!(registered_upload(&runtime, handle).vertex_count, 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    // ── Rendering integration / GPU deferral ────────────────────────────

    #[derive(Default)]
    struct MeshTrace {
        uploads: Vec<(String, [u8; 32])>,
        removals: Vec<String>,
        removal_failures_remaining: usize,
    }

    /// Headless backend that records mesh uploads/removals and reports one
    /// draw call per extracted drawable, mirroring the sandbox contract
    /// backend.
    struct TraceBackend {
        trace: Arc<Mutex<MeshTrace>>,
    }

    impl BackendRenderer for TraceBackend {
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
            pass: &engine_renderer::render_graph2::PassNode,
            stats: &mut FrameStats,
        ) -> Result<(), Vec<Diagnostic>> {
            if pass.kind == engine_renderer::render_graph2::PassKind::OpaquePbrForward {
                stats.draw_calls += input.drawables.len() as u32;
                stats.visible_drawables = input.drawables.len() as u32;
            }
            Ok(())
        }

        fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn upload_mesh(
            &mut self,
            upload: MeshUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            self.trace
                .lock()
                .unwrap()
                .uploads
                .push((upload.mesh_id.id.clone(), upload.content_hash));
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_texture(
            &mut self,
            _upload: engine_renderer::TextureUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_material(
            &mut self,
            _upload: engine_renderer::MaterialUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
            let mut trace = self.trace.lock().unwrap();
            if trace.removal_failures_remaining > 0 {
                trace.removal_failures_remaining -= 1;
                return Err(vec![Diagnostic::new(
                    "TEST_RESOURCE_REMOVAL_FAILED",
                    DiagnosticSeverity::Error,
                    "runtime-mesh-test",
                    "injected backend removal failure",
                )]);
            }
            trace.removals.push(removal.resource_id.id.clone());
            Ok(())
        }
    }

    fn sample_scene_with_mesh(mesh_id: &str) -> engine_scene::Scene {
        let mut scene = engine_scene::sample_scene();
        let entity = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .expect("sample scene cube entity");
        let renderable = entity
            .components
            .get_mut("engine.renderable")
            .expect("renderable component");
        renderable.fields.insert(
            "mesh".to_string(),
            engine_serialize::Value::Asset(AssetId::new(mesh_id)),
        );
        scene
    }

    #[test]
    fn renderable_referencing_runtime_mesh_produces_draw_calls() {
        let _guard = crate::tests::serial_ffi_world_test();
        let trace = Arc::new(Mutex::new(MeshTrace::default()));
        let mut runtime = runtime();
        runtime.set_renderer_backend(Box::new(TraceBackend {
            trace: Arc::clone(&trace),
        }));
        let handle = runtime
            .create_runtime_mesh("terrain-chunk-0", quad())
            .expect("create");
        let id = runtime.runtime_mesh_asset_id(handle).unwrap();
        runtime
            .load_scene(sample_scene_with_mesh(&id.id))
            .expect("scene loads");

        let stats = runtime.render_frame(0).expect("frame renders");

        assert_eq!(stats.visible_drawables, 1);
        assert_eq!(stats.draw_calls, 1);
        let trace = trace.lock().unwrap();
        assert!(
            trace
                .uploads
                .iter()
                .any(|(id, _)| id == "runtime-mesh-terrain-chunk-0"),
            "runtime mesh must be uploaded through the standard sync path, got {:?}",
            trace.uploads
        );
    }

    #[test]
    fn updated_runtime_mesh_is_reuploaded_with_new_content() {
        let _guard = crate::tests::serial_ffi_world_test();
        let trace = Arc::new(Mutex::new(MeshTrace::default()));
        let mut runtime = runtime();
        runtime.set_renderer_backend(Box::new(TraceBackend {
            trace: Arc::clone(&trace),
        }));
        let handle = runtime
            .create_runtime_mesh("morph", triangle())
            .expect("create");
        let id = runtime.runtime_mesh_asset_id(handle).unwrap();
        runtime
            .load_scene(sample_scene_with_mesh(&id.id))
            .expect("scene loads");

        runtime.render_frame(0).expect("first frame");
        runtime
            .update_runtime_mesh(handle, quad())
            .expect("full update");
        runtime.render_frame(1).expect("second frame");

        let trace = trace.lock().unwrap();
        let hashes: Vec<[u8; 32]> = trace
            .uploads
            .iter()
            .filter(|(id, _)| id == "runtime-mesh-morph")
            .map(|(_, hash)| *hash)
            .collect();
        assert_eq!(hashes.len(), 2, "one upload per frame, got {hashes:?}");
        assert_ne!(hashes[0], hashes[1], "update changed the uploaded content");
    }

    #[test]
    fn destroy_defers_gpu_removal_to_the_next_frame_boundary() {
        let _guard = crate::tests::serial_ffi_world_test();
        let trace = Arc::new(Mutex::new(MeshTrace::default()));
        let mut runtime = runtime();
        runtime.set_renderer_backend(Box::new(TraceBackend {
            trace: Arc::clone(&trace),
        }));
        let handle = runtime
            .create_runtime_mesh("temp", triangle())
            .expect("create");
        let id = runtime.runtime_mesh_asset_id(handle).unwrap();
        runtime
            .load_scene(sample_scene_with_mesh(&id.id))
            .expect("scene loads");
        runtime.render_frame(0).expect("frame renders");

        runtime.destroy_runtime_mesh(handle).expect("destroy");
        assert!(
            trace.lock().unwrap().removals.is_empty(),
            "GPU destruction must not happen mid-frame"
        );

        // The renderable still references the destroyed mesh; extraction no
        // longer resolves it, so the frame fails asset sync — but registry
        // reconciliation removes the stale backend resource first.
        let _ = runtime.render_frame(1);
        {
            let trace = trace.lock().unwrap();
            assert_eq!(
                trace.removals,
                vec!["runtime-mesh-temp".to_string()],
                "registry reconciliation removes the resource exactly once"
            );
        }
        // Once the renderable points at the built-in cube again, rendering
        // recovers; no further removals are queued.
        runtime
            .load_scene(sample_scene_with_mesh("mesh-cube"))
            .expect("scene reloads");
        runtime.render_frame(2).expect("frame renders");
        assert_eq!(trace.lock().unwrap().removals.len(), 1);
    }

    #[test]
    fn failed_registry_removal_is_reported_and_retried() {
        let _guard = crate::tests::serial_ffi_world_test();
        let trace = Arc::new(Mutex::new(MeshTrace::default()));
        let mut runtime = runtime();
        runtime.set_renderer_backend(Box::new(TraceBackend {
            trace: Arc::clone(&trace),
        }));
        let handle = runtime
            .create_runtime_mesh("retry", triangle())
            .expect("create");
        let id = runtime.runtime_mesh_asset_id(handle).unwrap();
        runtime
            .load_scene(sample_scene_with_mesh(&id.id))
            .expect("scene loads");
        runtime.render_frame(0).expect("frame renders");

        runtime.destroy_runtime_mesh(handle).expect("destroy");
        runtime
            .load_scene(sample_scene_with_mesh("mesh-cube"))
            .expect("scene reloads");
        trace.lock().unwrap().removal_failures_remaining = 1;

        let diagnostics = runtime.render_frame(1).expect_err("failure is reported");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "TEST_RESOURCE_REMOVAL_FAILED"));
        assert!(trace.lock().unwrap().removals.is_empty());

        runtime.render_frame(2).expect("next frame retries removal");
        assert_eq!(
            trace.lock().unwrap().removals,
            vec!["runtime-mesh-retry".to_string()]
        );
    }

    #[test]
    fn runtime_diagnostics_reports_runtime_mesh_memory() {
        let mut runtime = runtime();
        assert_eq!(runtime.runtime_diagnostics().runtime_meshes.mesh_count, 0);
        let _handle = runtime.create_runtime_mesh("diag", quad()).expect("create");
        let snapshot = runtime.runtime_diagnostics();
        assert_eq!(snapshot.runtime_meshes.mesh_count, 1);
        assert_eq!(snapshot.runtime_meshes.vertex_bytes, 4 * 32);
        assert_eq!(snapshot.runtime_meshes.total_bytes(), 4 * 32 + 6 * 4);
    }
}
