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
#[path = "runtime_mesh_tests.rs"]
mod tests;
