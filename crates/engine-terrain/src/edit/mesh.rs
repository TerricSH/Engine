use std::collections::HashMap;

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
                    let mut lattice_points = [[0_i64; 3]; 8];
                    let mut density = [0.0_f32; 8];
                    for (index, offset) in CUBE_CORNERS.iter().enumerate() {
                        let lattice = [
                            cell[0] + offset[0],
                            cell[1] + offset[1],
                            cell[2] + offset[2],
                        ];
                        lattice_points[index] = lattice;
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
                            tetrahedron.map(|index| lattice_points[index]),
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
    vertices: HashMap<SurfaceVertexKey, u32>,
    positions: Vec<[f32; 3]>,
    normal_sums: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn triangle(&mut self, a: SurfaceVertex, b: SurfaceVertex, c: SurfaceVertex) {
        let a_vec = Vec3::from_array(a.position);
        let b_vec = Vec3::from_array(b.position);
        let c_vec = Vec3::from_array(c.position);
        let face_normal = (b_vec - a_vec).cross(c_vec - a_vec);
        if face_normal.length_squared() <= f32::EPSILON {
            return;
        }
        let Some(a_index) = self.vertex(a) else {
            return;
        };
        let Some(b_index) = self.vertex(b) else {
            return;
        };
        let Some(c_index) = self.vertex(c) else {
            return;
        };
        for index in [a_index, b_index, c_index] {
            self.normal_sums[index as usize] += face_normal;
        }
        self.indices.extend_from_slice(&[a_index, b_index, c_index]);
    }

    fn vertex(&mut self, vertex: SurfaceVertex) -> Option<u32> {
        if let Some(index) = self.vertices.get(&vertex.key) {
            return Some(*index);
        }
        let index = u32::try_from(self.positions.len()).ok()?;
        self.positions.push(vertex.position);
        self.normal_sums.push(Vec3::ZERO);
        self.uvs.push([vertex.position[0], vertex.position[2]]);
        self.vertices.insert(vertex.key, index);
        Some(index)
    }

    fn finish(self) -> TerrainMeshData {
        let (bounds_min, bounds_max) = bounds(&self.positions);
        TerrainMeshData {
            positions: self.positions,
            normals: self
                .normal_sums
                .into_iter()
                .map(|normal| normal.normalize_or_zero().to_array())
                .collect(),
            uvs: self.uvs,
            indices: self.indices,
            bounds_min,
            bounds_max,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SurfaceVertex {
    key: SurfaceVertexKey,
    position: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SurfaceVertexKey {
    Lattice([i64; 3]),
    Edge([i64; 3], [i64; 3]),
}

impl SurfaceVertexKey {
    fn on_edge(left: [i64; 3], right: [i64; 3], amount: f32) -> Self {
        if amount <= f32::EPSILON {
            return Self::Lattice(left);
        }
        if amount >= 1.0 - f32::EPSILON {
            return Self::Lattice(right);
        }
        if left <= right {
            Self::Edge(left, right)
        } else {
            Self::Edge(right, left)
        }
    }
}

fn polygonise_tetrahedron(
    builder: &mut MeshBuilder,
    points: [[f32; 3]; 4],
    density: [f32; 4],
    lattice: [[i64; 3]; 4],
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
        crossings.push(SurfaceVertex {
            key: SurfaceVertexKey::on_edge(lattice[left], lattice[right], amount),
            position: std::array::from_fn(|axis| {
                points[left][axis] + (points[right][axis] - points[left][axis]) * amount
            }),
        });
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
        let referenced = mesh
            .mesh
            .indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(referenced.len(), mesh.mesh.positions.len());
        assert!(mesh.mesh.positions.len() < mesh.mesh.indices.len());
    }
}
