use std::collections::BTreeSet;

use glam::DVec3;

use crate::{
    TerrainChunkId, TerrainChunkRequest, TerrainFace, TerrainTopology, TerrainVolume,
    TerrainVolumeId,
};

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
    desired_chunks_for_volume_hysteretic(TerrainVolumeId::LEGACY, volume, focus_xz, previous)
}

/// Multi-volume variant of [`desired_chunks_hysteretic`].
pub fn desired_chunks_for_volume_hysteretic(
    volume_id: TerrainVolumeId,
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
            let id = TerrainChunkId::for_volume(volume_id, x, z, max_lod);
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

/// Stateless multi-volume selection convenience.
pub fn desired_chunks_for_volume(
    volume_id: TerrainVolumeId,
    volume: &TerrainVolume,
    focus_xz: [f64; 2],
) -> Vec<TerrainChunkRequest> {
    desired_chunks_for_volume_hysteretic(volume_id, volume, focus_xz, &BTreeSet::new())
}

/// Topology-aware selection used by the engine host. Planar terrain preserves
/// the existing X/Z CDLOD behavior; cube-sphere terrain streams six face
/// quadtrees against the full logical-space focus.
pub fn desired_terrain_chunks_hysteretic(
    volume: &TerrainVolume,
    focus: [f64; 3],
    previous: &BTreeSet<TerrainChunkId>,
) -> Vec<TerrainChunkRequest> {
    desired_terrain_chunks_for_volume_hysteretic(TerrainVolumeId::LEGACY, volume, focus, previous)
}

/// Topology-aware multi-volume selection. Every emitted chunk carries the
/// supplied identity, including descendants created while refining a node.
pub fn desired_terrain_chunks_for_volume_hysteretic(
    volume_id: TerrainVolumeId,
    volume: &TerrainVolume,
    focus: [f64; 3],
    previous: &BTreeSet<TerrainChunkId>,
) -> Vec<TerrainChunkRequest> {
    match volume.topology {
        TerrainTopology::Planar => {
            desired_chunks_for_volume_hysteretic(volume_id, volume, [focus[0], focus[2]], previous)
        }
        TerrainTopology::CubeSphere => {
            desired_planet_chunks_hysteretic(volume_id, volume, focus, previous)
        }
    }
}

pub fn desired_terrain_chunks(volume: &TerrainVolume, focus: [f64; 3]) -> Vec<TerrainChunkRequest> {
    desired_terrain_chunks_hysteretic(volume, focus, &BTreeSet::new())
}

pub fn desired_terrain_chunks_for_volume(
    volume_id: TerrainVolumeId,
    volume: &TerrainVolume,
    focus: [f64; 3],
) -> Vec<TerrainChunkRequest> {
    desired_terrain_chunks_for_volume_hysteretic(volume_id, volume, focus, &BTreeSet::new())
}

pub fn chunk_span(volume: &TerrainVolume, lod: u8) -> f64 {
    f64::from(volume.chunk_size) * 2.0f64.powi(i32::from(lod))
}

/// Conservative logical-space bounds for streaming transition coverage.
pub fn terrain_chunk_bounds(volume: &TerrainVolume, id: TerrainChunkId) -> ([f64; 3], [f64; 3]) {
    if id.face == TerrainFace::Planar {
        let span = chunk_span(volume, id.lod);
        let min_x = id.x as f64 * span;
        let min_z = id.z as f64 * span;
        return (
            [
                min_x,
                -f64::from(volume.height_scale + volume.skirt_depth),
                min_z,
            ],
            [min_x + span, f64::from(volume.height_scale), min_z + span],
        );
    }
    let (center, radius) = planet_patch_bound(volume, id);
    (
        (center - DVec3::splat(radius)).to_array(),
        (center + DVec3::splat(radius)).to_array(),
    )
}

/// Distance used by terrain LOD selection, exposed so render hosts can drive
/// the exact same transition interval for continuous geomorphing.
pub fn terrain_chunk_distance(volume: &TerrainVolume, id: TerrainChunkId, focus: [f64; 3]) -> f64 {
    if id.face == TerrainFace::Planar {
        distance_to_chunk(volume, id, [focus[0], focus[2]])
    } else {
        distance_to_planet_chunk(volume, id, DVec3::from_array(focus))
    }
}

fn desired_planet_chunks_hysteretic(
    volume_id: TerrainVolumeId,
    volume: &TerrainVolume,
    focus: [f64; 3],
    previous: &BTreeSet<TerrainChunkId>,
) -> Vec<TerrainChunkRequest> {
    if !volume.enabled || volume.validate().is_err() {
        return Vec::new();
    }
    let revision = volume.revision();
    let focus = DVec3::from_array(focus);
    let mut requests = Vec::new();
    for face in TerrainFace::CUBE_FACES {
        select_planet_node(
            volume,
            focus,
            previous,
            TerrainChunkId::on_volume_face(volume_id, face, 0, 0, volume.planet_max_lod),
            revision,
            &mut requests,
        );
    }
    requests.sort_by_key(|request| (request.priority, request.id));
    requests
}

fn select_planet_node(
    volume: &TerrainVolume,
    focus: DVec3,
    previous: &BTreeSet<TerrainChunkId>,
    id: TerrainChunkId,
    revision: u64,
    requests: &mut Vec<TerrainChunkRequest>,
) {
    if !planet_chunk_visible_from(volume, id, focus.to_array()) {
        return;
    }
    let distance = distance_to_planet_chunk(volume, id, focus);
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
        let base_x = id.x * 2;
        let base_z = id.z * 2;
        for dz in 0..=1 {
            for dx in 0..=1 {
                select_planet_node(
                    volume,
                    focus,
                    previous,
                    TerrainChunkId::on_volume_face(
                        id.volume_id,
                        id.face,
                        base_x + dx,
                        base_z + dz,
                        child_lod,
                    ),
                    revision,
                    requests,
                );
            }
        }
    } else {
        push_request(volume, id, revision, distance, requests);
    }
}

/// Conservatively rejects cube-sphere patches fully hidden behind the planet.
///
/// The test uses an inner sphere guaranteed to remain below the authored
/// displacement range, then expands the visible cap by both the patch angular
/// radius and the maximum terrain elevation. It therefore reduces back-side
/// generation and extraction without clipping elevated silhouettes.
pub fn planet_chunk_visible_from(
    volume: &TerrainVolume,
    id: TerrainChunkId,
    focus: [f64; 3],
) -> bool {
    if !volume.horizon_culling
        || volume.topology != TerrainTopology::CubeSphere
        || id.face == TerrainFace::Planar
    {
        return true;
    }

    let radial = DVec3::from_array(focus) - DVec3::from_array(volume.planet_center);
    let camera_distance = radial.length();
    let displacement = f64::from(volume.height_scale + volume.skirt_depth);
    let inner_radius = (volume.planet_radius - displacement).max(f64::EPSILON);
    if camera_distance <= inner_radius {
        return true;
    }

    let (patch_direction, patch_angular_radius) = planet_patch_angular_bound(volume, id);
    let camera_direction = radial / camera_distance;
    let center_angle = camera_direction
        .dot(patch_direction)
        .clamp(-1.0, 1.0)
        .acos();
    let camera_horizon = (inner_radius / camera_distance).clamp(0.0, 1.0).acos();
    let outer_radius = volume.planet_radius + f64::from(volume.height_scale);
    let elevation_margin = (inner_radius / outer_radius.max(inner_radius))
        .clamp(0.0, 1.0)
        .acos();

    center_angle
        <= camera_horizon + patch_angular_radius + elevation_margin + f64::from(f32::EPSILON)
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
                    TerrainChunkId::for_volume(id.volume_id, x, z, child_lod),
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
        if candidate.volume_id != node.volume_id {
            return false;
        }
        if candidate.lod >= node.lod {
            return false;
        }
        if candidate.face != node.face {
            return false;
        }
        let level_delta = u32::from(node.lod - candidate.lod);
        let Some(scale) = 1i64.checked_shl(level_delta) else {
            return false;
        };
        candidate.x.div_euclid(scale) == node.x && candidate.z.div_euclid(scale) == node.z
    })
}

fn distance_to_planet_chunk(volume: &TerrainVolume, id: TerrainChunkId, focus: DVec3) -> f64 {
    let (center, bound_radius) = planet_patch_bound(volume, id);
    (focus.distance(center) - bound_radius).max(0.0)
}

fn planet_patch_bound(volume: &TerrainVolume, id: TerrainChunkId) -> (DVec3, f64) {
    let (center_direction, _) = planet_patch_angular_bound(volume, id);
    let divisions = 1_i64 << u32::from(volume.planet_max_lod - id.lod);
    let step = 2.0 / divisions as f64;
    let u_min = -1.0 + id.x as f64 * step;
    let v_min = -1.0 + id.z as f64 * step;
    let center = DVec3::from_array(volume.planet_center) + center_direction * volume.planet_radius;
    let outer_radius = volume.planet_radius + f64::from(volume.height_scale);
    let corner_chord = [
        (u_min, v_min),
        (u_min + step, v_min),
        (u_min, v_min + step),
        (u_min + step, v_min + step),
    ]
    .into_iter()
    .map(|(u, v)| {
        let direction = crate::planet::face_cube_point(id.face, u, v).normalize();
        (direction * outer_radius - center_direction * volume.planet_radius).length()
    })
    .fold(0.0, f64::max);
    (
        center,
        corner_chord + f64::from(volume.height_scale + volume.skirt_depth),
    )
}

fn planet_patch_angular_bound(volume: &TerrainVolume, id: TerrainChunkId) -> (DVec3, f64) {
    let divisions = 1_i64 << u32::from(volume.planet_max_lod - id.lod);
    let step = 2.0 / divisions as f64;
    let u_min = -1.0 + id.x as f64 * step;
    let v_min = -1.0 + id.z as f64 * step;
    let center_direction =
        crate::planet::face_cube_point(id.face, u_min + step * 0.5, v_min + step * 0.5).normalize();
    let angular_radius = [
        (u_min, v_min),
        (u_min + step, v_min),
        (u_min, v_min + step),
        (u_min + step, v_min + step),
    ]
    .into_iter()
    .map(|(u, v)| {
        let direction = crate::planet::face_cube_point(id.face, u, v).normalize();
        direction.dot(center_direction).clamp(-1.0, 1.0).acos()
    })
    .fold(0.0, f64::max);
    (center_direction, angular_radius)
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

    #[test]
    fn planet_selection_culls_the_back_side_and_refines_near_the_surface() {
        let volume = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1_000.0,
            planet_max_lod: 2,
            lod_distances: vec![100.0, 400.0, 2_000.0],
            lod_hysteresis: 10.0,
            ..TerrainVolume::default()
        };
        let requests = desired_terrain_chunks(&volume, [0.0, 1_010.0, 0.0]);
        assert!(!requests
            .iter()
            .any(|request| request.id.face == TerrainFace::NegativeY));
        assert!(requests
            .iter()
            .any(|request| { request.id.face == TerrainFace::PositiveY && request.id.lod == 0 }));
        assert!(requests.len() < 6 * 16);
    }

    #[test]
    fn identical_planet_coordinates_are_namespaced_by_volume() {
        let volume = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1_000.0,
            planet_max_lod: 1,
            lod_distances: vec![100.0, 2_000.0],
            ..TerrainVolume::default()
        };
        let first_id = TerrainVolumeId::from_persistent_id("planet:first");
        let second_id = TerrainVolumeId::from_persistent_id("planet:second");
        let first = desired_terrain_chunks_for_volume(first_id, &volume, [0.0, 1_010.0, 0.0]);
        let second = desired_terrain_chunks_for_volume(second_id, &volume, [0.0, 1_010.0, 0.0]);

        assert_eq!(first.len(), second.len());
        assert!(first.iter().all(|request| request.id.volume_id == first_id));
        assert!(second
            .iter()
            .all(|request| request.id.volume_id == second_id));
        assert!(first.iter().zip(&second).all(|(left, right)| {
            (left.id.face, left.id.x, left.id.z, left.id.lod)
                == (right.id.face, right.id.x, right.id.z, right.id.lod)
                && left.id != right.id
        }));
    }

    #[test]
    fn each_planet_uses_its_own_center_for_focus_lod_and_horizon() {
        let first = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_center: [-10_000.0, 250.0, 500.0],
            planet_radius: 1_000.0,
            planet_max_lod: 2,
            lod_distances: vec![100.0, 400.0, 2_000.0],
            ..TerrainVolume::default()
        };
        let mut second = first.clone();
        second.planet_center = [20_000.0, -750.0, -250.0];
        second.seed = 99;
        let first_id = TerrainVolumeId::from_persistent_id("planet:first");
        let second_id = TerrainVolumeId::from_persistent_id("planet:second");
        let focus_above = |volume: &TerrainVolume| {
            [
                volume.planet_center[0],
                volume.planet_center[1] + volume.planet_radius + 10.0,
                volume.planet_center[2],
            ]
        };
        let first_requests =
            desired_terrain_chunks_for_volume(first_id, &first, focus_above(&first));
        let second_requests =
            desired_terrain_chunks_for_volume(second_id, &second, focus_above(&second));

        for (id, requests) in [(first_id, &first_requests), (second_id, &second_requests)] {
            assert!(requests.iter().any(|request| {
                request.id.volume_id == id
                    && request.id.face == TerrainFace::PositiveY
                    && request.id.lod == 0
            }));
            assert!(!requests
                .iter()
                .any(|request| request.id.face == TerrainFace::NegativeY));
        }
    }

    #[test]
    fn horizon_culling_can_be_disabled_for_diagnostics() {
        let volume = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1_000.0,
            planet_max_lod: 1,
            lod_distances: vec![100.0, 2_000.0],
            horizon_culling: false,
            ..TerrainVolume::default()
        };
        let requests = desired_terrain_chunks(&volume, [0.0, 1_010.0, 0.0]);
        for face in TerrainFace::CUBE_FACES {
            assert!(requests.iter().any(|request| request.id.face == face));
        }
    }

    #[test]
    fn elevated_limb_patch_is_kept_conservatively() {
        let volume = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1_000.0,
            height_scale: 100.0,
            planet_max_lod: 1,
            lod_distances: vec![100.0, 2_000.0],
            ..TerrainVolume::default()
        };
        let limb = TerrainChunkId::on_face(TerrainFace::PositiveX, 0, 0, 1);
        assert!(planet_chunk_visible_from(
            &volume,
            limb,
            [0.0, 1_010.0, 0.0]
        ));
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
