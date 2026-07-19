use engine_procgen::{Fbm2D, FbmParams, Seed, WarpParams, WarpedFbm2D};
use thiserror::Error;

use crate::{
    TerrainChunkData, TerrainChunkGenerator, TerrainChunkRequest, TerrainCollisionData,
    TerrainMeshData,
};

#[derive(Clone, Debug, Default)]
pub struct HeightfieldGenerator;

#[derive(Debug, Error)]
enum HeightfieldError {
    #[error("invalid terrain parameters: {0}")]
    Config(String),
    #[error("heightfield vertex/index count exceeds the u32 mesh limit")]
    MeshTooLarge,
}

impl TerrainChunkGenerator for HeightfieldGenerator {
    fn generate(&self, request: &TerrainChunkRequest) -> Result<TerrainChunkData, String> {
        generate_heightfield(request).map_err(|error| error.to_string())
    }
}

fn generate_heightfield(
    request: &TerrainChunkRequest,
) -> Result<TerrainChunkData, HeightfieldError> {
    let volume = &request.volume;
    volume
        .validate()
        .map_err(|error| HeightfieldError::Config(error.to_string()))?;

    // Every CDLOD node keeps the same topology. Coarser nodes cover a larger
    // physical footprint, so their sample spacing grows without introducing
    // a second, same-footprint mesh hierarchy.
    let cells = volume.base_resolution - 1;
    let resolution = cells + 1;
    let vertex_count = u64::from(resolution) * u64::from(resolution);
    if vertex_count > u64::from(u32::MAX) {
        return Err(HeightfieldError::MeshTooLarge);
    }

    let fbm = Fbm2D::new(
        Seed(volume.seed),
        FbmParams {
            octaves: volume.octaves,
            frequency: volume.frequency,
            amplitude: 1.0,
            lacunarity: volume.lacunarity,
            gain: volume.gain,
            offset: [0.0; 3],
            normalize: true,
        },
    )
    .map_err(|error| HeightfieldError::Config(error.to_string()))?;
    let warped = (volume.domain_warp_amplitude > 0.0)
        .then(|| {
            WarpedFbm2D::new(
                fbm,
                WarpParams {
                    amplitude: volume.domain_warp_amplitude,
                    frequency: volume.domain_warp_frequency,
                },
            )
        })
        .transpose()
        .map_err(|error| HeightfieldError::Config(error.to_string()))?;
    let sample = |x: f64, z: f64| -> f32 {
        warped.as_ref().map_or_else(
            || fbm.sample_wide(x, z),
            |sampler| sampler.sample_wide(x, z),
        ) * volume.height_scale
    };

    let span = crate::chunk_span(volume, request.id.lod);
    let origin_x = request.id.x as f64 * span;
    let origin_z = request.id.z as f64 * span;
    let spacing = (span / f64::from(cells)) as f32;
    let mut heights = Vec::with_capacity(vertex_count as usize);
    for z in 0..resolution {
        for x in 0..resolution {
            heights.push(sample(
                origin_x + f64::from(x) * f64::from(spacing),
                origin_z + f64::from(z) * f64::from(spacing),
            ));
        }
    }

    let mut positions = Vec::with_capacity(vertex_count as usize + 4 * resolution as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut min_height = f32::INFINITY;
    let mut max_height = f32::NEG_INFINITY;
    for z in 0..resolution {
        for x in 0..resolution {
            let local_x = x as f32 * spacing;
            let local_z = z as f32 * spacing;
            let world_x = origin_x + f64::from(local_x);
            let world_z = origin_z + f64::from(local_z);
            let height = heights[(z * resolution + x) as usize];
            min_height = min_height.min(height);
            max_height = max_height.max(height);
            let spacing_wide = f64::from(spacing);
            let left = sample(world_x - spacing_wide, world_z);
            let right = sample(world_x + spacing_wide, world_z);
            let down = sample(world_x, world_z - spacing_wide);
            let up = sample(world_x, world_z + spacing_wide);
            let normal =
                glam::Vec3::new(left - right, 2.0 * spacing, down - up).normalize_or_zero();
            positions.push([local_x, height, local_z]);
            normals.push(normal.to_array());
            uvs.push([x as f32 / cells as f32, z as f32 / cells as f32]);
        }
    }

    let mut indices = Vec::with_capacity((cells * cells * 6 + resolution * 24) as usize);
    for z in 0..cells {
        for x in 0..cells {
            let a = z * resolution + x;
            let b = a + 1;
            let c = a + resolution;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    // A vertical skirt is a topology-local crack repair mechanism: adjacent
    // chunks may independently choose an LOD and never need to share a world
    // cell or synchronise edge topology.
    if volume.skirt_depth > 0.0 {
        let north = (0..resolution).collect::<Vec<_>>();
        let south = (0..resolution)
            .map(|x| (resolution - 1) * resolution + x)
            .collect::<Vec<_>>();
        let west = (0..resolution).map(|z| z * resolution).collect::<Vec<_>>();
        let east = (0..resolution)
            .map(|z| z * resolution + resolution - 1)
            .collect::<Vec<_>>();
        append_skirt(
            &north,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            volume.skirt_depth,
            true,
        )?;
        append_skirt(
            &south,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            volume.skirt_depth,
            false,
        )?;
        append_skirt(
            &west,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            volume.skirt_depth,
            false,
        )?;
        append_skirt(
            &east,
            &mut positions,
            &mut normals,
            &mut uvs,
            &mut indices,
            volume.skirt_depth,
            true,
        )?;
        min_height -= volume.skirt_depth;
    }

    let collision = volume.collision_enabled.then_some(TerrainCollisionData {
        rows: resolution,
        columns: resolution,
        heights,
        sample_spacing: spacing,
    });
    Ok(TerrainChunkData {
        id: request.id,
        revision: request.revision,
        origin: [origin_x, 0.0, origin_z],
        mesh: TerrainMeshData {
            positions,
            normals,
            uvs,
            indices,
            bounds_min: [0.0, min_height, 0.0],
            bounds_max: [span as f32, max_height, span as f32],
        },
        collision,
    })
}

fn append_skirt(
    edge: &[u32],
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    depth: f32,
    flip: bool,
) -> Result<(), HeightfieldError> {
    let skirt_start = u32::try_from(positions.len()).map_err(|_| HeightfieldError::MeshTooLarge)?;
    for &base in edge {
        let mut position = positions[base as usize];
        position[1] -= depth;
        positions.push(position);
        normals.push(normals[base as usize]);
        uvs.push(uvs[base as usize]);
    }
    for segment in 0..edge.len().saturating_sub(1) {
        let a = edge[segment];
        let b = edge[segment + 1];
        let c = skirt_start + segment as u32;
        let d = c + 1;
        if flip {
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        } else {
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TerrainChunkId;

    fn request(id: TerrainChunkId) -> TerrainChunkRequest {
        TerrainChunkRequest {
            id,
            revision: 1,
            priority: 0,
            volume: TerrainVolume::default(),
        }
    }

    use crate::TerrainVolume;

    #[test]
    fn adjacent_chunks_share_exact_border_heights() {
        let generator = HeightfieldGenerator;
        let a = generator
            .generate(&request(TerrainChunkId::new(0, 0, 0)))
            .unwrap();
        let b = generator
            .generate(&request(TerrainChunkId::new(1, 0, 0)))
            .unwrap();
        let resolution = a.collision.as_ref().unwrap().columns as usize;
        for z in 0..resolution {
            let a_height = a.collision.as_ref().unwrap().heights[z * resolution + resolution - 1];
            let b_height = b.collision.as_ref().unwrap().heights[z * resolution];
            assert_eq!(a_height.to_bits(), b_height.to_bits());
        }
    }

    #[test]
    fn far_adjacent_chunks_keep_detail_and_share_exact_border_heights() {
        let generator = HeightfieldGenerator;
        let far = 20_000_000i64;
        let a = generator
            .generate(&request(TerrainChunkId::new(far, far, 0)))
            .unwrap();
        let b = generator
            .generate(&request(TerrainChunkId::new(far + 1, far, 0)))
            .unwrap();
        let resolution = a.collision.as_ref().unwrap().columns as usize;
        for z in 0..resolution {
            let a_height = a.collision.as_ref().unwrap().heights[z * resolution + resolution - 1];
            let b_height = b.collision.as_ref().unwrap().heights[z * resolution];
            assert_eq!(a_height.to_bits(), b_height.to_bits());
        }
        let distinct = a
            .collision
            .as_ref()
            .unwrap()
            .heights
            .windows(2)
            .any(|pair| pair[0].to_bits() != pair[1].to_bits());
        assert!(distinct, "far terrain must retain local height variation");
    }

    #[test]
    fn lod_expands_footprint_at_fixed_topology_and_skirts_extend_bounds() {
        let data = HeightfieldGenerator
            .generate(&request(TerrainChunkId::new(0, 0, 2)))
            .unwrap();
        let collision = data.collision.unwrap();
        assert_eq!(collision.columns, 65);
        assert_eq!(collision.sample_spacing, 4.0);
        assert_eq!(data.mesh.bounds_max[0], 256.0);
        assert!(data.mesh.positions.len() > 65 * 65);
        assert!(data.mesh.bounds_min[1] <= -TerrainVolume::default().skirt_depth);
    }
}
