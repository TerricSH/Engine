use engine_scene::components::TriplanarMaterialMapping;
use engine_terrain::{TerrainChunkData, TerrainMaterialProjection, TerrainTopology, TerrainVolume};

pub(super) fn projected_material_mapping(
    data: &TerrainChunkData,
    volume: Option<&TerrainVolume>,
) -> Option<TriplanarMaterialMapping> {
    let volume = volume?;
    let projection = match volume.material_projection {
        TerrainMaterialProjection::Automatic => match volume.topology {
            TerrainTopology::Planar => TerrainMaterialProjection::WorldTriplanar,
            TerrainTopology::CubeSphere => TerrainMaterialProjection::PlanetTriplanar,
        },
        projection => projection,
    };
    if projection == TerrainMaterialProjection::MeshUv {
        return None;
    }
    let logical_entity_origin =
        std::array::from_fn(|axis| data.origin[axis] + f64::from(data.local_center[axis]));
    let tile_size = f64::from(volume.material_tile_size);
    let local_origin = match projection {
        TerrainMaterialProjection::WorldTriplanar => {
            logical_entity_origin.map(|coordinate| -(coordinate.rem_euclid(tile_size) as f32))
        }
        TerrainMaterialProjection::PlanetTriplanar => std::array::from_fn(|axis| {
            -((logical_entity_origin[axis] - volume.planet_center[axis]).rem_euclid(tile_size)
                as f32)
        }),
        TerrainMaterialProjection::Automatic | TerrainMaterialProjection::MeshUv => {
            unreachable!("automatic and mesh UV projections were resolved above")
        }
    };
    local_origin
        .into_iter()
        .all(f32::is_finite)
        .then_some(TriplanarMaterialMapping {
            local_origin,
            meters_per_tile: volume.material_tile_size,
            blend_sharpness: volume.triplanar_blend_sharpness,
        })
}

#[cfg(test)]
mod tests {
    use engine_terrain::{TerrainChunkId, TerrainMeshData};

    use super::*;

    fn chunk(origin: [f64; 3]) -> TerrainChunkData {
        TerrainChunkData {
            id: TerrainChunkId::new(0, 0, 0),
            revision: 1,
            origin,
            local_center: [0.0; 3],
            mesh: TerrainMeshData::default(),
            geomorph: None,
            collision: None,
            triangle_collision: None,
        }
    }

    #[test]
    fn large_planet_projection_keeps_only_f64_repeat_phase_and_matches_across_chunks() {
        let center = [1.0e12, -2.0e12, 3.0e12];
        let radius = 60_000_000.25;
        let volume = TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            material_projection: TerrainMaterialProjection::PlanetTriplanar,
            material_tile_size: 128.0,
            planet_center: center,
            planet_radius: radius,
            ..TerrainVolume::default()
        };
        let first_origin = [center[0] + radius, center[1] + 17.5, center[2] - 9.25];
        let second_origin = [first_origin[0] + 1_000.0, first_origin[1], first_origin[2]];
        let first = projected_material_mapping(&chunk(first_origin), Some(&volume))
            .expect("first projection");
        let second = projected_material_mapping(&chunk(second_origin), Some(&volume))
            .expect("second projection");

        for axis in 0..3 {
            let expected = (first_origin[axis] - center[axis]).rem_euclid(128.0);
            assert!((f64::from(-first.local_origin[axis]) - expected).abs() < 1.0e-5);
            assert!(first.local_origin[axis].abs() < 128.0);
            assert!(second.local_origin[axis].abs() < 128.0);
        }

        let first_shared_u = (1_000.0 - first.local_origin[0]) / first.meters_per_tile;
        let second_shared_u = -second.local_origin[0] / second.meters_per_tile;
        assert!(
            (first_shared_u.rem_euclid(1.0) - second_shared_u.rem_euclid(1.0)).abs() < 1.0e-6,
            "adjacent chunks must sample the same repeated texture coordinate"
        );
    }
}
