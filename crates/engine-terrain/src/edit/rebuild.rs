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
        for (offset, key) in selected.iter().copied().enumerate() {
            let chunk = self.build_chunk_mesh(key, &base_density);
            let empty = chunk.mesh.indices.is_empty();
            for sink in sinks.iter_mut() {
                let result = if empty {
                    sink.remove_chunk(key, chunk.revision)
                } else {
                    sink.replace_chunk(&chunk)
                };
                if let Err(error) = result {
                    self.dirty_meshes.extend(selected[offset..].iter().copied());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DensityTerrainConfig;

    #[derive(Default)]
    struct RejectSecondChunk {
        calls: usize,
    }

    impl EditableTerrainRebuildSink for RejectSecondChunk {
        fn replace_chunk(&mut self, _chunk: &EditableTerrainMesh) -> Result<(), String> {
            self.accept_or_reject()
        }

        fn remove_chunk(&mut self, _key: DensityChunkKey, _revision: u64) -> Result<(), String> {
            self.accept_or_reject()
        }
    }

    impl RejectSecondChunk {
        fn accept_or_reject(&mut self) -> Result<(), String> {
            self.calls += 1;
            if self.calls == 2 {
                Err("injected failure".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn sink_failure_requeues_failed_and_unprocessed_chunks() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig {
            chunk_cells: 2,
            ..DensityTerrainConfig::default()
        })
        .unwrap();
        let keys = [
            DensityChunkKey::new(0, 0, 0),
            DensityChunkKey::new(1, 0, 0),
            DensityChunkKey::new(2, 0, 0),
        ];
        for key in keys {
            terrain.request_mesh_rebuild(key);
        }

        let mut sink = RejectSecondChunk::default();
        let error = terrain
            .rebuild_dirty_chunks(3, |_| -1.0, &mut [&mut sink])
            .unwrap_err();

        assert!(error.to_string().contains("injected failure"));
        assert_eq!(terrain.take_dirty_mesh_chunks(usize::MAX), keys[1..]);
    }
}
