use super::{DensityChunkKey, EditableTerrain, EditableTerrainMesh, TerrainEditError};

/// Host integration point for coordinated render/collision/navigation updates.
/// Implementations may upload immediately or enqueue work on their respective
/// subsystem threads; a revision lets them discard stale results.
pub trait EditableTerrainRebuildSink {
    fn replace_chunk(&mut self, chunk: &EditableTerrainMesh) -> Result<(), String>;
    fn remove_chunk(&mut self, key: DensityChunkKey, revision: u64) -> Result<(), String>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainRebuildReport {
    pub rebuilt: Vec<DensityChunkKey>,
    pub removed: Vec<DensityChunkKey>,
    pub remaining: usize,
}

impl EditableTerrain {
    /// Rebuild a bounded set of dirty chunks and publish the same revision to
    /// every host sink. Empty meshes are explicit removals. If a sink rejects
    /// a chunk it is re-queued for retry; sinks should replace revisions
    /// idempotently because an earlier sink may already have accepted it.
    pub fn rebuild_dirty_chunks(
        &mut self,
        limit: usize,
        base_density: impl Fn([f64; 3]) -> f32,
        sinks: &mut [&mut dyn EditableTerrainRebuildSink],
    ) -> Result<TerrainRebuildReport, TerrainEditError> {
        let selected = self.take_dirty_mesh_chunks(limit);
        let mut report = TerrainRebuildReport::default();
        for key in selected {
            let chunk = self.build_chunk_mesh(key, &base_density);
            let empty = chunk.mesh.indices.is_empty();
            for sink in sinks.iter_mut() {
                let result = if empty {
                    sink.remove_chunk(key, chunk.revision)
                } else {
                    sink.replace_chunk(&chunk)
                };
                if let Err(error) = result {
                    self.dirty_meshes.insert(key);
                    return Err(TerrainEditError::Storage(format!(
                        "terrain rebuild sink rejected {key:?}: {error}"
                    )));
                }
            }
            if empty {
                report.removed.push(key);
            } else {
                report.rebuilt.push(key);
            }
        }
        report.remaining = self.pending_mesh_rebuilds();
        Ok(report)
    }
}
