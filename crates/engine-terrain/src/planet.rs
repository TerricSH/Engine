use engine_procgen::{Fbm3D, FbmParams, Seed, WarpParams, WarpedFbm3D};
use glam::DVec3;

use crate::{
    TerrainChunkData, TerrainChunkRequest, TerrainFace, TerrainGeomorphData, TerrainMeshData,
    TerrainTopology, TerrainTriangleCollisionData, TerrainVolume,
};

/// Immutable, allocation-free query object backed by the exact same seeded
/// noise recipe as cube-sphere mesh generation.
///
/// Gameplay, navigation, physics helpers, and terrain generation should all
/// use this type instead of reimplementing planetary height mathematics.
#[derive(Clone, Copy, Debug)]
pub struct PlanetTerrainQuery {
    center: DVec3,
    radius: f64,
    height_scale: f64,
    fbm: Fbm3D,
    warped: Option<WarpedFbm3D>,
}

/// Local orthonormal basis and exact sampled surface point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetTangentFrame {
    pub surface_point: [f64; 3],
    pub normal: [f64; 3],
    pub east: [f64; 3],
    pub north: [f64; 3],
}

/// Latitude/longitude coordinates in radians plus signed terrain altitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetCoordinates {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
}

impl PlanetTerrainQuery {
    pub fn new(volume: &TerrainVolume) -> Result<Self, String> {
        volume.validate().map_err(|error| error.to_string())?;
        if volume.topology != TerrainTopology::CubeSphere {
            return Err("planet terrain queries require CubeSphere topology".into());
        }
        let fbm = Fbm3D::new(
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
        .map_err(|error| error.to_string())?;
        let warped = (volume.domain_warp_amplitude > 0.0)
            .then(|| {
                WarpedFbm3D::new(
                    fbm,
                    WarpParams {
                        amplitude: volume.domain_warp_amplitude,
                        frequency: volume.domain_warp_frequency,
                    },
                )
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            center: DVec3::from_array(volume.planet_center),
            radius: volume.planet_radius,
            height_scale: f64::from(volume.height_scale),
            fbm,
            warped,
        })
    }

    pub fn center(&self) -> [f64; 3] {
        self.center.to_array()
    }

    pub fn radius(&self) -> f64 {
        self.radius
    }

    pub fn height_along_direction(&self, direction: [f64; 3]) -> f64 {
        let direction = DVec3::from_array(direction).normalize_or_zero();
        if direction == DVec3::ZERO {
            return 0.0;
        }
        let sample_position = direction * self.radius;
        let [x, y, z] = sample_position.to_array();
        f64::from(self.warped.as_ref().map_or_else(
            || self.fbm.sample_wide(x, y, z),
            |sampler| sampler.sample_wide(x, y, z),
        )) * self.height_scale
    }

    pub fn surface_point_from_direction(&self, direction: [f64; 3]) -> [f64; 3] {
        let direction = DVec3::from_array(direction).normalize_or_zero();
        if direction == DVec3::ZERO {
            return self.center.to_array();
        }
        (self.center
            + direction * (self.radius + self.height_along_direction(direction.to_array())))
        .to_array()
    }

    pub fn project_world_to_surface(&self, world: [f64; 3]) -> [f64; 3] {
        let radial = DVec3::from_array(world) - self.center;
        self.surface_point_from_direction(radial.to_array())
    }

    pub fn altitude(&self, world: [f64; 3]) -> f64 {
        let radial = DVec3::from_array(world) - self.center;
        let distance = radial.length();
        if distance <= f64::EPSILON {
            return -self.radius;
        }
        distance - self.radius - self.height_along_direction((radial / distance).to_array())
    }

    pub fn coordinates(&self, world: [f64; 3]) -> PlanetCoordinates {
        let radial = DVec3::from_array(world) - self.center;
        let direction = radial.normalize_or_zero();
        PlanetCoordinates {
            latitude: direction.y.clamp(-1.0, 1.0).asin(),
            longitude: direction.z.atan2(direction.x),
            altitude: self.altitude(world),
        }
    }

    pub fn world_from_coordinates(&self, coordinates: PlanetCoordinates) -> [f64; 3] {
        let latitude_cos = coordinates.latitude.cos();
        let direction = DVec3::new(
            latitude_cos * coordinates.longitude.cos(),
            coordinates.latitude.sin(),
            latitude_cos * coordinates.longitude.sin(),
        );
        let surface = DVec3::from_array(self.surface_point_from_direction(direction.to_array()));
        (surface + direction * coordinates.altitude).to_array()
    }

    pub fn tangent_frame(&self, direction: [f64; 3]) -> PlanetTangentFrame {
        let radial = DVec3::from_array(direction).normalize_or_zero();
        let radial = if radial == DVec3::ZERO {
            DVec3::Y
        } else {
            radial
        };
        let reference = if radial.y.abs() > 0.92 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let east = reference.cross(radial).normalize();
        let angular_step = 1.0e-5;
        let north_hint = radial.cross(east).normalize();
        let left = DVec3::from_array(
            self.surface_point_from_direction((radial - east * angular_step).to_array()),
        );
        let right = DVec3::from_array(
            self.surface_point_from_direction((radial + east * angular_step).to_array()),
        );
        let down = DVec3::from_array(
            self.surface_point_from_direction((radial - north_hint * angular_step).to_array()),
        );
        let up = DVec3::from_array(
            self.surface_point_from_direction((radial + north_hint * angular_step).to_array()),
        );
        let tangent_east = (right - left).normalize_or_zero();
        let tangent_north = (up - down).normalize_or_zero();
        let mut normal = tangent_east.cross(tangent_north).normalize_or_zero();
        if normal.dot(radial) < 0.0 {
            normal = -normal;
        }
        let north = normal.cross(tangent_east).normalize_or_zero();
        PlanetTangentFrame {
            surface_point: self.surface_point_from_direction(radial.to_array()),
            normal: normal.to_array(),
            east: tangent_east.to_array(),
            north: north.to_array(),
        }
    }

    pub fn great_circle_distance(&self, from: [f64; 3], to: [f64; 3]) -> f64 {
        let from = (DVec3::from_array(from) - self.center).normalize_or_zero();
        let to = (DVec3::from_array(to) - self.center).normalize_or_zero();
        if from == DVec3::ZERO || to == DVec3::ZERO {
            return 0.0;
        }
        from.dot(to).clamp(-1.0, 1.0).acos() * self.radius
    }
}

pub(crate) fn generate_planet_chunk(
    request: &TerrainChunkRequest,
) -> Result<TerrainChunkData, String> {
    let volume = &request.volume;
    volume.validate().map_err(|error| error.to_string())?;
    if request.id.face == TerrainFace::Planar {
        return Err("cube-sphere chunk requires a cube face identity".into());
    }
    if request.id.lod > volume.planet_max_lod {
        return Err(format!(
            "planet chunk LOD {} exceeds planet_max_lod {}",
            request.id.lod, volume.planet_max_lod
        ));
    }

    let divisions = 1_i64
        .checked_shl(u32::from(volume.planet_max_lod - request.id.lod))
        .ok_or_else(|| "planet quadtree depth is not representable".to_string())?;
    if request.id.x < 0
        || request.id.z < 0
        || request.id.x >= divisions
        || request.id.z >= divisions
    {
        return Err(format!(
            "planet chunk coordinates ({}, {}) are outside face level 0..{}",
            request.id.x,
            request.id.z,
            divisions - 1
        ));
    }

    let query = PlanetTerrainQuery::new(volume)?;
    let world_at = |u: f64, v: f64| -> (DVec3, DVec3) {
        let direction = face_cube_point(request.id.face, u, v).normalize();
        let world = DVec3::from_array(query.surface_point_from_direction(direction.to_array()));
        (world, direction)
    };

    let resolution = volume.base_resolution;
    let cells = resolution - 1;
    let base_vertex_count = u64::from(resolution) * u64::from(resolution);
    if base_vertex_count > u64::from(u32::MAX) {
        return Err("planet vertex count exceeds the u32 mesh limit".into());
    }

    let face_step = 2.0 / divisions as f64;
    let u_min = -1.0 + request.id.x as f64 * face_step;
    let v_min = -1.0 + request.id.z as f64 * face_step;
    let grid_step = face_step / f64::from(cells);
    let (origin, _) = world_at(u_min + face_step * 0.5, v_min + face_step * 0.5);

    let mut positions = Vec::with_capacity(base_vertex_count as usize + 4 * resolution as usize);
    let mut directions = Vec::with_capacity(positions.capacity());
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for z in 0..resolution {
        for x in 0..resolution {
            let u = u_min + f64::from(x) * grid_step;
            let v = v_min + f64::from(z) * grid_step;
            let (world, direction) = world_at(u, v);
            let local = world - origin;

            let (left, _) = world_at(u - grid_step, v);
            let (right, _) = world_at(u + grid_step, v);
            let (down, _) = world_at(u, v - grid_step);
            let (up, _) = world_at(u, v + grid_step);
            let tangent_u = right - left;
            let tangent_v = up - down;
            let mut normal = tangent_v.cross(tangent_u).normalize_or_zero();
            if normal.dot(direction) < 0.0 {
                normal = -normal;
            }

            positions.push(local.to_array().map(|value| value as f32));
            directions.push(direction);
            normals.push(normal.to_array().map(|value| value as f32));
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
    let geomorph = if volume.geomorph_enabled && request.id.lod < volume.planet_max_lod {
        let mut deltas = vec![0.0f32; base_vertex_count as usize];
        let mut max_delta = 0.0f32;
        for z in 0..resolution as usize {
            for x in 0..resolution as usize {
                let index = z * resolution as usize + x;
                let fine = DVec3::from_array(positions[index].map(f64::from));
                let coarse = coarse_parent_position(&positions, resolution as usize, x, z);
                let delta = (coarse - fine).dot(directions[index]) as f32;
                deltas[index] = delta;
                max_delta = max_delta.max(delta.abs());
            }
        }
        let delta_scale = (max_delta * 2.0).max(1.0e-4);
        for (normal, delta) in normals.iter_mut().zip(deltas) {
            let encoded_length = 1.0 + delta / delta_scale;
            for component in normal {
                *component *= encoded_length;
            }
        }
        let local_origin = DVec3::from_array(volume.planet_center) - origin;
        Some(TerrainGeomorphData {
            delta_scale,
            local_origin: local_origin.to_array().map(|value| value as f32),
        })
    } else {
        None
    };
    let collision = volume
        .collision_enabled
        .then(|| TerrainTriangleCollisionData {
            positions: positions.clone(),
            triangles: indices
                .chunks_exact(3)
                .map(|triangle| [triangle[0], triangle[1], triangle[2]])
                .collect(),
        });

    if volume.skirt_depth > 0.0 {
        let north = (0..resolution).collect::<Vec<_>>();
        let south = (0..resolution)
            .map(|x| (resolution - 1) * resolution + x)
            .collect::<Vec<_>>();
        let west = (0..resolution).map(|z| z * resolution).collect::<Vec<_>>();
        let east = (0..resolution)
            .map(|z| z * resolution + resolution - 1)
            .collect::<Vec<_>>();
        let mut mesh = SphericalSkirtMesh {
            positions: &mut positions,
            normals: &mut normals,
            uvs: &mut uvs,
            indices: &mut indices,
        };
        append_spherical_skirt(&north, &directions, &mut mesh, volume.skirt_depth, true)?;
        append_spherical_skirt(&south, &directions, &mut mesh, volume.skirt_depth, false)?;
        append_spherical_skirt(&west, &directions, &mut mesh, volume.skirt_depth, false)?;
        append_spherical_skirt(&east, &directions, &mut mesh, volume.skirt_depth, true)?;
    }

    let (bounds_min, bounds_max) = mesh_bounds(&positions);
    Ok(TerrainChunkData {
        id: request.id,
        revision: request.revision,
        origin: origin.to_array(),
        local_center: [0.0; 3],
        mesh: TerrainMeshData {
            positions,
            normals,
            uvs,
            indices,
            bounds_min,
            bounds_max,
        },
        geomorph,
        collision: None,
        triangle_collision: collision,
    })
}

fn coarse_parent_position(positions: &[[f32; 3]], resolution: usize, x: usize, z: usize) -> DVec3 {
    let last = resolution - 1;
    let x0 = (x / 2) * 2;
    let z0 = (z / 2) * 2;
    let x1 = (x0 + 2).min(last);
    let z1 = (z0 + 2).min(last);
    let tx = if x1 == x0 {
        0.0
    } else {
        (x - x0) as f64 / (x1 - x0) as f64
    };
    let tz = if z1 == z0 {
        0.0
    } else {
        (z - z0) as f64 / (z1 - z0) as f64
    };
    let at = |sample_x: usize, sample_z: usize| {
        DVec3::from_array(positions[sample_z * resolution + sample_x].map(f64::from))
    };
    let a = at(x0, z0);
    let b = at(x1, z0);
    let c = at(x0, z1);
    let d = at(x1, z1);
    if tx + tz <= 1.0 {
        a + (b - a) * tx + (c - a) * tz
    } else {
        d + (c - d) * (1.0 - tx) + (b - d) * (1.0 - tz)
    }
}

pub(crate) fn face_cube_point(face: TerrainFace, u: f64, v: f64) -> DVec3 {
    match face {
        TerrainFace::PositiveX => DVec3::new(1.0, v, u),
        TerrainFace::NegativeX => DVec3::new(-1.0, v, -u),
        TerrainFace::PositiveY => DVec3::new(u, 1.0, v),
        TerrainFace::NegativeY => DVec3::new(u, -1.0, -v),
        TerrainFace::PositiveZ => DVec3::new(v, u, 1.0),
        TerrainFace::NegativeZ => DVec3::new(-v, u, -1.0),
        TerrainFace::Planar => DVec3::new(u, 1.0, v),
    }
}

struct SphericalSkirtMesh<'a> {
    positions: &'a mut Vec<[f32; 3]>,
    normals: &'a mut Vec<[f32; 3]>,
    uvs: &'a mut Vec<[f32; 2]>,
    indices: &'a mut Vec<u32>,
}

fn append_spherical_skirt(
    edge: &[u32],
    directions: &[DVec3],
    mesh: &mut SphericalSkirtMesh<'_>,
    depth: f32,
    flip: bool,
) -> Result<(), String> {
    let skirt_start =
        u32::try_from(mesh.positions.len()).map_err(|_| "planet mesh exceeds u32 indices")?;
    for &base in edge {
        let direction = directions[base as usize]
            .to_array()
            .map(|value| value as f32);
        let mut position = mesh.positions[base as usize];
        let normal = mesh.normals[base as usize];
        let uv = mesh.uvs[base as usize];
        for axis in 0..3 {
            position[axis] -= direction[axis] * depth;
        }
        mesh.positions.push(position);
        mesh.normals.push(normal);
        mesh.uvs.push(uv);
    }
    for segment in 0..edge.len().saturating_sub(1) {
        let a = edge[segment];
        let b = edge[segment + 1];
        let c = skirt_start + segment as u32;
        let d = c + 1;
        if flip {
            mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
        } else {
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Ok(())
}

fn mesh_bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(position[axis]);
            max[axis] = max[axis].max(position[axis]);
        }
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        HeightfieldGenerator, TerrainChunkGenerator, TerrainChunkId, TerrainTopology, TerrainVolume,
    };

    fn volume() -> TerrainVolume {
        TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1_000.0,
            planet_max_lod: 1,
            lod_distances: vec![200.0, 2_000.0],
            height_scale: 20.0,
            base_resolution: 9,
            ..TerrainVolume::default()
        }
    }

    fn generate(face: TerrainFace) -> TerrainChunkData {
        HeightfieldGenerator
            .generate(&TerrainChunkRequest {
                id: TerrainChunkId::on_face(face, 0, 0, 1),
                revision: 7,
                priority: 0,
                volume: volume(),
            })
            .unwrap()
    }

    fn world_position(chunk: &TerrainChunkData, index: usize) -> DVec3 {
        DVec3::from_array(chunk.origin)
            + DVec3::from_array(chunk.mesh.positions[index].map(f64::from))
    }

    #[test]
    fn adjacent_cube_faces_share_the_exact_geometric_seam() {
        let top = generate(TerrainFace::PositiveY);
        let right = generate(TerrainFace::PositiveX);
        let resolution = volume().base_resolution as usize;
        for coordinate in 0..resolution {
            let top_index = coordinate * resolution + resolution - 1;
            let right_index = (resolution - 1) * resolution + coordinate;
            let delta = world_position(&top, top_index) - world_position(&right, right_index);
            assert!(delta.length() < 1.0e-4, "seam delta was {delta:?}");
        }
    }

    #[test]
    fn generated_planet_patch_has_outward_normals_and_exact_trimesh_collision() {
        let chunk = generate(TerrainFace::PositiveY);
        let collision = chunk.triangle_collision.as_ref().expect("planet collision");
        assert_eq!(collision.positions.len(), 9 * 9);
        assert_eq!(collision.triangles.len(), 8 * 8 * 2);
        assert!(chunk.mesh.positions.len() > collision.positions.len());
        for (position, normal) in collision.positions.iter().zip(&chunk.mesh.normals) {
            let radial =
                DVec3::from_array(chunk.origin) + DVec3::from_array(position.map(f64::from));
            let normal = DVec3::from_array(normal.map(f64::from));
            assert!(radial.dot(normal) > 0.0);
        }
    }

    #[test]
    fn query_round_trips_coordinates_and_matches_generated_vertices() {
        let volume = volume();
        let query = PlanetTerrainQuery::new(&volume).unwrap();
        let coordinates = PlanetCoordinates {
            latitude: 0.47,
            longitude: -1.13,
            altitude: 37.5,
        };
        let world = query.world_from_coordinates(coordinates);
        let round_trip = query.coordinates(world);
        assert!((round_trip.latitude - coordinates.latitude).abs() < 1.0e-9);
        assert!((round_trip.longitude - coordinates.longitude).abs() < 1.0e-9);
        assert!((round_trip.altitude - coordinates.altitude).abs() < 1.0e-8);

        let chunk = generate(TerrainFace::PositiveY);
        let resolution = volume.base_resolution as usize;
        let vertex = world_position(&chunk, resolution * resolution / 2);
        let projected = DVec3::from_array(query.project_world_to_surface(vertex.to_array()));
        assert!(projected.distance(vertex) < 1.0e-4);
    }

    #[test]
    fn tangent_frame_is_orthonormal_and_outward() {
        let query = PlanetTerrainQuery::new(&volume()).unwrap();
        let direction = DVec3::new(0.3, 0.8, -0.2).normalize();
        let frame = query.tangent_frame(direction.to_array());
        let normal = DVec3::from_array(frame.normal);
        let east = DVec3::from_array(frame.east);
        let north = DVec3::from_array(frame.north);
        assert!(normal.dot(direction) > 0.99);
        assert!(normal.dot(east).abs() < 1.0e-8);
        assert!(normal.dot(north).abs() < 1.0e-8);
        assert!(east.dot(north).abs() < 1.0e-8);
    }

    #[test]
    fn planetary_noise_retains_surface_detail_at_large_radius() {
        let query = PlanetTerrainQuery::new(&TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_radius: 1.0e12,
            height_scale: 100.0,
            ..TerrainVolume::default()
        })
        .unwrap();
        let heights = (0..16)
            .map(|index| {
                query
                    .height_along_direction(
                        DVec3::new(1.0, f64::from(index) * 1.0e-10, 0.0)
                            .normalize()
                            .to_array(),
                    )
                    .to_bits()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            heights.len() > 1,
            "large-radius neighboring directions must not collapse to one f32 sample"
        );
    }

    #[test]
    fn detailed_chunk_encodes_a_continuous_parent_surface_delta() {
        let chunk = HeightfieldGenerator
            .generate(&TerrainChunkRequest {
                id: TerrainChunkId::on_face(TerrainFace::PositiveY, 0, 0, 0),
                revision: 1,
                priority: 0,
                volume: volume(),
            })
            .unwrap();
        let morph = chunk.geomorph.expect("fine chunk geomorph");
        assert!(morph.delta_scale.is_finite() && morph.delta_scale > 0.0);
        let lengths = chunk.mesh.normals
            [..volume().base_resolution as usize * volume().base_resolution as usize]
            .iter()
            .map(|normal| DVec3::from_array(normal.map(f64::from)).length())
            .collect::<Vec<_>>();
        assert!(lengths.iter().any(|length| (*length - 1.0).abs() > 1.0e-5));
        assert!((lengths[0] - 1.0).abs() < 1.0e-5);
    }
}
