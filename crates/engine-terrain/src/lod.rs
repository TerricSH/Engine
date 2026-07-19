use std::collections::BTreeSet;

use crate::{TerrainChunkId, TerrainChunkRequest, TerrainVolume};

#[derive(Clone, Debug, PartialEq)]
pub struct TerrainLodConfig {
    pub distances: Vec<f32>,
}

impl TerrainLodConfig {
    pub fn from_volume(volume: &TerrainVolume) -> Self {
        Self {
            distances: volume.lod_distances.clone(),
        }
    }

    pub fn lod_at_distance(&self, distance: f32) -> Option<u8> {
        self.distances
            .iter()
            .position(|cutoff| distance <= *cutoff)
            .map(|lod| lod.min(u8::MAX as usize) as u8)
    }
}

/// Select a non-overlapping CDLOD quadtree around `focus_xz`.
///
/// LOD 0 nodes cover `chunk_size`; every coarser level doubles the physical
/// span on both axes. `previous` supplies the last selection so split/merge
/// decisions use the authored dead band instead of oscillating at a cutoff.
pub fn desired_chunks_hysteretic(
    volume: &TerrainVolume,
    focus_xz: [f64; 2],
    previous: &BTreeSet<TerrainChunkId>,
) -> Vec<TerrainChunkRequest> {
    if !volume.enabled || volume.validate().is_err() {
        return Vec::new();
    }

    let max_lod = (volume.lod_distances.len() - 1) as u8;
    let root_span = chunk_span(volume, max_lod);
    let radius = f64::from(*volume.lod_distances.last().expect("validated distances"));
    let min_x = ((focus_xz[0] - radius) / root_span).floor() as i64;
    let max_x = ((focus_xz[0] + radius) / root_span).floor() as i64;
    let min_z = ((focus_xz[1] - radius) / root_span).floor() as i64;
    let max_z = ((focus_xz[1] + radius) / root_span).floor() as i64;
    let revision = volume.revision();
    let mut requests = Vec::new();

    for z in min_z..=max_z {
        for x in min_x..=max_x {
            let id = TerrainChunkId::new(x, z, max_lod);
            if distance_to_chunk(volume, id, focus_xz) <= radius {
                select_node(volume, focus_xz, previous, id, revision, &mut requests);
            }
        }
    }

    requests.sort_by_key(|request| (request.priority, request.id));
    requests
}

/// Stateless selection convenience for callers without a previous frame.
pub fn desired_chunks(volume: &TerrainVolume, focus_xz: [f64; 2]) -> Vec<TerrainChunkRequest> {
    desired_chunks_hysteretic(volume, focus_xz, &BTreeSet::new())
}

pub fn chunk_span(volume: &TerrainVolume, lod: u8) -> f64 {
    f64::from(volume.chunk_size) * 2.0f64.powi(i32::from(lod))
}

fn select_node(
    volume: &TerrainVolume,
    focus_xz: [f64; 2],
    previous: &BTreeSet<TerrainChunkId>,
    id: TerrainChunkId,
    revision: u64,
    requests: &mut Vec<TerrainChunkRequest>,
) {
    let distance = distance_to_chunk(volume, id, focus_xz);
    let split = if id.lod == 0 {
        false
    } else {
        let cutoff = f64::from(volume.lod_distances[usize::from(id.lod - 1)]);
        let hysteresis = f64::from(volume.lod_hysteresis);
        if was_split(previous, id) {
            distance <= cutoff + hysteresis
        } else if previous.contains(&id) {
            distance < (cutoff - hysteresis).max(0.0)
        } else {
            distance <= cutoff
        }
    };

    if split {
        let child_lod = id.lod - 1;
        let Some(base_x) = id.x.checked_mul(2) else {
            push_request(volume, id, revision, distance, requests);
            return;
        };
        let Some(base_z) = id.z.checked_mul(2) else {
            push_request(volume, id, revision, distance, requests);
            return;
        };
        for dz in 0..=1 {
            for dx in 0..=1 {
                let Some(x) = base_x.checked_add(dx) else {
                    continue;
                };
                let Some(z) = base_z.checked_add(dz) else {
                    continue;
                };
                select_node(
                    volume,
                    focus_xz,
                    previous,
                    TerrainChunkId::new(x, z, child_lod),
                    revision,
                    requests,
                );
            }
        }
    } else {
        push_request(volume, id, revision, distance, requests);
    }
}

fn push_request(
    volume: &TerrainVolume,
    id: TerrainChunkId,
    revision: u64,
    distance: f64,
    requests: &mut Vec<TerrainChunkRequest>,
) {
    requests.push(TerrainChunkRequest {
        id,
        revision,
        priority: (distance.max(0.0) * 1024.0).min(f64::from(u32::MAX)) as u32,
        volume: volume.clone(),
    });
}

fn was_split(previous: &BTreeSet<TerrainChunkId>, node: TerrainChunkId) -> bool {
    previous.iter().any(|candidate| {
        if candidate.lod >= node.lod {
            return false;
        }
        let level_delta = u32::from(node.lod - candidate.lod);
        let Some(scale) = 1i64.checked_shl(level_delta) else {
            return false;
        };
        candidate.x.div_euclid(scale) == node.x && candidate.z.div_euclid(scale) == node.z
    })
}

fn distance_to_chunk(volume: &TerrainVolume, id: TerrainChunkId, focus_xz: [f64; 2]) -> f64 {
    let span = chunk_span(volume, id.lod);
    let min_x = id.x as f64 * span;
    let min_z = id.z as f64 * span;
    let max_x = min_x + span;
    let max_z = min_z + span;
    let dx = if focus_xz[0] < min_x {
        min_x - focus_xz[0]
    } else if focus_xz[0] > max_x {
        focus_xz[0] - max_x
    } else {
        0.0
    };
    let dz = if focus_xz[1] < min_z {
        min_z - focus_xz[1]
    } else if focus_xz[1] > max_z {
        focus_xz[1] - max_z
    } else {
        0.0
    };
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_sorted_non_overlapping_and_uses_multiple_lods() {
        let volume = TerrainVolume {
            chunk_size: 32.0,
            lod_distances: vec![48.0, 96.0, 192.0],
            ..Default::default()
        };
        let requests = desired_chunks(&volume, [0.0, 0.0]);
        assert!(!requests.is_empty());
        assert!(requests
            .windows(2)
            .all(|pair| pair[0].priority <= pair[1].priority));
        assert!(requests.iter().any(|request| request.id.lod == 0));
        assert!(requests.iter().any(|request| request.id.lod > 0));
        for (index, left) in requests.iter().enumerate() {
            for right in requests.iter().skip(index + 1) {
                assert!(!overlaps(&volume, left.id, right.id));
            }
        }
    }

    #[test]
    fn hysteresis_retains_a_previous_split_inside_dead_band() {
        let volume = TerrainVolume {
            chunk_size: 64.0,
            lod_distances: vec![160.0, 320.0],
            lod_hysteresis: 16.0,
            ..Default::default()
        };
        let initial = desired_chunks(&volume, [0.0, 64.0]);
        let previous = initial.iter().map(|request| request.id).collect();
        let stateless = desired_chunks(&volume, [-40.0, 64.0]);
        let stable = desired_chunks_hysteretic(&volume, [-40.0, 64.0], &previous);
        let parent = TerrainChunkId::new(1, 0, 1);
        assert!(stateless.iter().any(|request| request.id == parent));
        assert!(!stable.iter().any(|request| request.id == parent));
        assert!(stable
            .iter()
            .any(|request| request.id.lod == 0 && request.id.x.div_euclid(2) == 1));
    }

    #[test]
    fn selection_retains_chunk_identity_at_large_logical_coordinates() {
        let volume = TerrainVolume {
            chunk_size: 64.0,
            lod_distances: vec![96.0],
            ..Default::default()
        };
        let focus = 1_000_000_000_000.0;
        let requests = desired_chunks(&volume, [focus, focus]);
        let expected = (focus / 64.0).floor() as i64;
        assert!(requests.iter().any(|request| request.id.x == expected));
        assert!(requests.iter().any(|request| request.id.x == expected - 1));
    }

    fn overlaps(volume: &TerrainVolume, left: TerrainChunkId, right: TerrainChunkId) -> bool {
        let left_span = chunk_span(volume, left.lod);
        let right_span = chunk_span(volume, right.lod);
        let left_min = [left.x as f64 * left_span, left.z as f64 * left_span];
        let right_min = [right.x as f64 * right_span, right.z as f64 * right_span];
        left_min[0] < right_min[0] + right_span
            && right_min[0] < left_min[0] + left_span
            && left_min[1] < right_min[1] + right_span
            && right_min[1] < left_min[1] + left_span
    }
}
