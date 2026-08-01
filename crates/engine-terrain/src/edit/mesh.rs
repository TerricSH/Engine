use glam::Vec3;

use crate::{TerrainMeshData, TerrainTriangleCollisionData};

use super::{DensityChunkKey, EditableTerrain};

/// One independently replaceable volumetric terrain render/collision chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct EditableTerrainMesh {
    pub key: DensityChunkKey,
    pub revision: u64,
    pub origin: [f64; 3],
    pub mesh: TerrainMeshData,
    pub collision: TerrainTriangleCollisionData,
}

impl EditableTerrain {
    /// Polygonise a density chunk with marching tetrahedra. The caller supplies
    /// the same immutable base field used by editing so untouched boundary
    /// samples remain continuous without materialising neighbouring chunks.
    pub fn build_chunk_mesh(
        &self,
        key: DensityChunkKey,
        base_density: impl Fn([f64; 3]) -> f32,
    ) -> EditableTerrainMesh {
        let cells = i64::from(self.config().chunk_cells);
        let start = [key.x * cells, key.y * cells, key.z * cells];
        let origin = key.origin(self.config());
        let mut builder = MeshBuilder::default();

        for z in 0..cells {
            for y in 0..cells {
                for x in 0..cells {
                    let cell = [start[0] + x, start[1] + y, start[2] + z];
                    let mut points = [[0.0_f32; 3]; 8];
                    let mut density = [0.0_f32; 8];
                    for (index, offset) in CUBE_CORNERS.iter().enumerate() {
                        let lattice = [
                            cell[0] + offset[0],
                            cell[1] + offset[1],
                            cell[2] + offset[2],
                        ];
                        let world = self.lattice_to_world(lattice);
                        points[index] =
                            std::array::from_fn(|axis| (world[axis] - origin[axis]) as f32);
                        density[index] = self.sample_density_with(lattice, &base_density);
                    }
                    for tetrahedron in TETRAHEDRA {
                        polygonise_tetrahedron(
                            &mut builder,
                            tetrahedron.map(|index| points[index]),
                            tetrahedron.map(|index| density[index]),
                            self.config().iso_level,
                        );
                    }
                }
            }
        }

        let mesh = builder.finish();
        let triangles = mesh
            .indices
            .chunks_exact(3)
            .map(|indices| [indices[0], indices[1], indices[2]])
            .collect();
        let collision = TerrainTriangleCollisionData {
            positions: mesh.positions.clone(),
            triangles,
        };
        EditableTerrainMesh {
            key,
            revision: self.chunk_revision(key),
            origin,
            mesh,
            collision,
        }
    }
}

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let Some(base) = u32::try_from(self.positions.len()).ok() else {
            return;
        };
        let a_vec = Vec3::from_array(a);
        let b_vec = Vec3::from_array(b);
        let c_vec = Vec3::from_array(c);
        let normal = (b_vec - a_vec).cross(c_vec - a_vec).normalize_or_zero();
        if normal == Vec3::ZERO {
            return;
        }
        self.positions.extend_from_slice(&[a, b, c]);
        self.normals.extend_from_slice(&[normal.to_array(); 3]);
        self.uvs.extend([a, b, c].map(|point| [point[0], point[2]]));
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn finish(self) -> TerrainMeshData {
        let (bounds_min, bounds_max) = bounds(&self.positions);
        TerrainMeshData {
            positions: self.positions,
            normals: self.normals,
            uvs: self.uvs,
            indices: self.indices,
            bounds_min,
            bounds_max,
        }
    }
}

fn polygonise_tetrahedron(
    builder: &mut MeshBuilder,
    points: [[f32; 3]; 4],
    density: [f32; 4],
    iso_level: f32,
) {
    let mut crossings = Vec::with_capacity(4);
    for [left, right] in TETRAHEDRON_EDGES {
        let left_inside = density[left] >= iso_level;
        let right_inside = density[right] >= iso_level;
        if left_inside == right_inside {
            continue;
        }
        let denominator = density[right] - density[left];
        let amount = if denominator.abs() <= f32::EPSILON {
            0.5
        } else {
            ((iso_level - density[left]) / denominator).clamp(0.0, 1.0)
        };
        crossings.push(std::array::from_fn(|axis| {
            points[left][axis] + (points[right][axis] - points[left][axis]) * amount
        }));
    }
    match crossings.as_slice() {
        [a, b, c] => builder.triangle(*a, *b, *c),
        [a, b, c, d] => {
            builder.triangle(*a, *b, *c);
            builder.triangle(*a, *c, *d);
        }
        _ => {}
    }
}

fn bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(position[axis]);
            maximum[axis] = maximum[axis].max(position[axis]);
        }
    }
    (minimum, maximum)
}

const CUBE_CORNERS: [[i64; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [1, 1, 0],
    [0, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [1, 1, 1],
    [0, 1, 1],
];
const TETRAHEDRA: [[usize; 4]; 6] = [
    [0, 5, 1, 6],
    [0, 1, 2, 6],
    [0, 2, 3, 6],
    [0, 3, 7, 6],
    [0, 7, 4, 6],
    [0, 4, 5, 6],
];
const TETRAHEDRON_EDGES: [[usize; 2]; 6] = [[0, 1], [1, 2], [2, 0], [0, 3], [1, 3], [2, 3]];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DensityTerrainConfig, TerrainBrush, TerrainBrushFalloff, TerrainBrushMode};

    #[test]
    fn polygonises_a_closed_excavation_for_rendering_and_collision() {
        let mut terrain = EditableTerrain::new(DensityTerrainConfig {
            chunk_cells: 8,
            ..DensityTerrainConfig::default()
        })
        .unwrap();
        terrain
            .apply_brush(
                &TerrainBrush {
                    center: [4.0; 3],
                    radius: 2.5,
                    strength: 4.0,
                    falloff: TerrainBrushFalloff::Smooth,
                    mode: TerrainBrushMode::Subtract,
                    material: None,
                },
                |_| 1.0,
            )
            .unwrap();
        let mesh = terrain.build_chunk_mesh(DensityChunkKey::new(0, 0, 0), |_| 1.0);
        assert!(!mesh.mesh.positions.is_empty());
        assert_eq!(mesh.mesh.positions.len(), mesh.mesh.normals.len());
        assert_eq!(mesh.mesh.indices.len() / 3, mesh.collision.triangles.len());
    }
}
