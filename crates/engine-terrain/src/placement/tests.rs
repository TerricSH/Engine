use std::collections::BTreeMap;

use engine_scene::{Component, ComponentRegistry};
use engine_serialize::Value;
use glam::{DQuat, DVec3, Vec3};

use super::*;
use crate::{TerrainTopology, TerrainVolume};

fn query() -> PlanetTerrainQuery {
    PlanetTerrainQuery::new(&TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_radius: 1_000.0,
        height_scale: 8.0,
        planet_max_lod: 1,
        lod_distances: vec![100.0, 300.0],
        ..TerrainVolume::default()
    })
    .unwrap()
}

#[test]
fn placement_builds_an_orthonormal_surface_transform_without_script_math() {
    let query = query();
    let anchor = PlanetSurfaceAnchor {
        direction: [1.0, 0.2, -0.3],
        heading_radians: 0.75,
        footprint_radius: 0.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        ..Default::default()
    };
    let placement = anchor.resolve(&query).unwrap();
    let right = DVec3::from_array(placement.right);
    let up = DVec3::from_array(placement.normal);
    let forward = DVec3::from_array(placement.forward);
    assert!(right.dot(up).abs() < 1.0e-10);
    assert!(right.cross(up).dot(forward) > 0.999_999);
    let rotation = DQuat::from_array(placement.rotation);
    assert!(rotation.is_normalized());

    let transform = placement.to_transform(query.center()).unwrap();
    assert!(transform.translation.length() > 975.0);
    assert!(transform.rotation.is_normalized());
}

#[test]
fn footprint_validation_rejects_slope_or_support_span_deterministically() {
    let query = query();
    let direction = [0.72, 0.41, -0.56];
    let permissive = PlanetSurfaceAnchor {
        direction,
        footprint_radius: 18.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        support_samples: 24,
        ..Default::default()
    };
    let placement = permissive.resolve(&query).unwrap();
    let slope_rejected = PlanetSurfaceAnchor {
        max_slope_radians: (placement.maximum_slope_radians - 1.0e-7).max(0.0),
        ..permissive.clone()
    };
    assert!(matches!(
        slope_rejected.resolve(&query),
        Err(PlanetPlacementError::SlopeExceeded { .. })
    ));
    let span_rejected = PlanetSurfaceAnchor {
        max_height_delta: (placement.support_height_span - 1.0e-7).max(0.0),
        ..permissive
    };
    assert!(matches!(
        span_rejected.resolve(&query),
        Err(PlanetPlacementError::HeightSpanExceeded { .. })
    ));
}

#[test]
fn occupancy_uses_geodesic_caps_across_coordinate_seams_and_persists() {
    let query = query();
    let anchor = |direction| PlanetSurfaceAnchor {
        direction,
        footprint_radius: 30.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        ..Default::default()
    };
    let left = anchor([-1.0, 0.0, 0.001]).resolve(&query).unwrap();
    let right = anchor([-1.0, 0.0, -0.001]).resolve(&query).unwrap();
    let far = anchor([1.0, 0.0, 0.0]).resolve(&query).unwrap();

    let mut occupancy = PlanetSurfaceOccupancy::default();
    occupancy.reserve("left", left, true, 0.0).unwrap();
    assert!(matches!(
        occupancy.reserve("right", right, true, 0.0),
        Err(PlanetPlacementError::Occupied { .. })
    ));
    occupancy.reserve("far", far, false, 0.0).unwrap();
    assert!(occupancy.contains_direction([-1.0, 0.0, 0.0]));
    assert_eq!(occupancy.navigation_blockers().count(), 1);

    let encoded = occupancy.to_bincode().unwrap();
    let restored = PlanetSurfaceOccupancy::from_bincode_compatible(&encoded).unwrap();
    assert_eq!(restored, occupancy);
}

#[test]
fn occupancy_uses_unambiguous_planet_keys_and_reports_entity_ids() {
    let placement = PlanetSurfaceAnchor {
        direction: [1.0, 0.0, 0.0],
        footprint_radius: 10.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        ..Default::default()
    }
    .resolve(&query())
    .unwrap();
    let mut occupancy = PlanetSurfaceOccupancy::default();
    occupancy
        .reserve_for_volume("planet:a", "building", placement, true, 0.0)
        .unwrap();
    occupancy
        .reserve_for_volume("planet", "a:building", placement, true, 0.0)
        .unwrap();

    assert_eq!(occupancy.reservations().len(), 2);
    assert_eq!(
        occupancy
            .reservations_for_volume("planet:a")
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["building"]
    );
    assert_eq!(
        occupancy
            .reservations_for_volume("planet")
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["a:building"]
    );
    assert!(matches!(
        occupancy.reserve_for_volume("planet:a", "other", placement, true, 0.0),
        Err(PlanetPlacementError::Occupied { existing_id }) if existing_id == "building"
    ));
}

#[test]
fn legacy_and_explicit_planet_keys_cannot_overwrite_each_other() {
    let placement = PlanetSurfaceAnchor {
        direction: [1.0, 0.0, 0.0],
        footprint_radius: 0.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        ..Default::default()
    }
    .resolve(&query())
    .unwrap();
    let mut occupancy = PlanetSurfaceOccupancy::default();
    occupancy
        .reserve_for_volume("a", "b", placement, true, 0.0)
        .unwrap();
    occupancy.reserve("b", placement, true, 0.0).unwrap();

    assert_eq!(occupancy.reservations().len(), 2);
    assert_eq!(
        occupancy
            .reservations_for_volume("")
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["b"]
    );
    assert_eq!(
        occupancy
            .reservations_for_volume("a")
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["b"]
    );
}

#[test]
fn persistent_and_runtime_owner_domains_cannot_overwrite_each_other() {
    let placement = PlanetSurfaceAnchor {
        direction: [1.0, 0.0, 0.0],
        footprint_radius: 0.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        ..Default::default()
    }
    .resolve(&query())
    .unwrap();
    let volume = PlanetSurfaceVolumeKey::Persistent("planet".into());
    let mut occupancy = PlanetSurfaceOccupancy::default();
    occupancy
        .reserve_scoped(
            volume.clone(),
            PlanetSurfaceOwnerKey::Persistent("runtime:7:3".into()),
            placement,
            true,
            0.0,
        )
        .unwrap();
    occupancy
        .reserve_scoped(
            volume.clone(),
            PlanetSurfaceOwnerKey::Runtime {
                index: 7,
                generation: 3,
            },
            placement,
            true,
            0.0,
        )
        .unwrap();

    let owners = occupancy
        .reservations_for_scope(&volume)
        .map(|(owner, _)| owner.clone())
        .collect::<Vec<_>>();
    assert_eq!(owners.len(), 2);
    assert_ne!(owners[0], owners[1]);
    assert_eq!(owners[0].display_id(), owners[1].display_id());
}

#[test]
fn compatible_decoder_reads_real_legacy_bincode_bytes_and_normalizes_keys() {
    #[derive(Serialize)]
    struct LegacyOccupancy {
        reservations: BTreeMap<String, PlanetConstructionFootprint>,
    }

    let legacy_entity_id = "a:b".to_string();
    let legacy = LegacyOccupancy {
        reservations: BTreeMap::from([(
            legacy_entity_id.clone(),
            PlanetConstructionFootprint {
                direction: [1.0, 0.0, 0.0],
                angular_radius: 0.05,
                blocks_navigation: true,
            },
        )]),
    };
    let legacy_bytes = bincode::serialize(&legacy).unwrap();
    assert!(bincode::deserialize::<PlanetSurfaceOccupancy>(&legacy_bytes).is_err());

    let restored = PlanetSurfaceOccupancy::from_bincode_compatible(&legacy_bytes).unwrap();
    let reservations = restored.reservations().collect::<Vec<_>>();
    assert_eq!(reservations.len(), 1);
    assert_eq!(reservations[0].volume, &PlanetSurfaceVolumeKey::Legacy);
    assert_eq!(
        reservations[0].owner,
        &PlanetSurfaceOwnerKey::Persistent(legacy_entity_id)
    );

    let current_bytes = restored.to_bincode().unwrap();
    let current: PlanetSurfaceOccupancy = bincode::deserialize(&current_bytes).unwrap();
    assert_eq!(current, restored);
    for length in 0..current_bytes.len() {
        assert!(
            PlanetSurfaceOccupancy::from_bincode_compatible(&current_bytes[..length]).is_err(),
            "accepted truncated current occupancy at {length}/{} bytes",
            current_bytes.len()
        );
    }
    let mut with_trailing = current_bytes;
    with_trailing.push(0);
    assert!(PlanetSurfaceOccupancy::from_bincode_compatible(&with_trailing).is_err());
}

#[test]
fn human_readable_occupancy_round_trips_current_and_accepts_legacy_shape() {
    let placement = PlanetSurfaceAnchor {
        direction: [1.0, 0.0, 0.0],
        footprint_radius: 0.0,
        max_slope_radians: std::f64::consts::FRAC_PI_2,
        max_height_delta: f64::MAX,
        ..Default::default()
    }
    .resolve(&query())
    .unwrap();
    let mut occupancy = PlanetSurfaceOccupancy::default();
    occupancy
        .reserve_for_volume("planet", "building", placement, true, 0.0)
        .unwrap();

    let current_json = serde_json::to_string(&occupancy).unwrap();
    let current: PlanetSurfaceOccupancy = serde_json::from_str(&current_json).unwrap();
    assert_eq!(current, occupancy);

    let legacy_json = r#"{
        "reservations": {
            "legacy-building": {
                "direction": [1.0, 0.0, 0.0],
                "angular_radius": 0.05,
                "blocks_navigation": true
            }
        }
    }"#;
    let legacy: PlanetSurfaceOccupancy = serde_json::from_str(legacy_json).unwrap();
    let reservation = legacy.reservations().next().unwrap();
    assert_eq!(reservation.volume, &PlanetSurfaceVolumeKey::Legacy);
    assert_eq!(
        reservation.owner,
        &PlanetSurfaceOwnerKey::Persistent("legacy-building".into())
    );
}

#[test]
fn registered_anchor_round_trips_and_rejects_lossy_fields() {
    let mut registry = ComponentRegistry::new();
    crate::register_terrain_extensions(&mut registry);
    assert!(registry.get(PlanetSurfaceAnchor::TYPE_ID).is_some());
    let extension = registry.get(PlanetSurfaceAnchor::TYPE_ID).unwrap();
    let anchor = PlanetSurfaceAnchor {
        terrain_volume_id: "planet:primary".to_string(),
        direction: [0.2, 0.9, -0.3],
        footprint_radius: 4.5,
        ..Default::default()
    };
    let fields = (extension.serialize.unwrap())(&anchor);
    assert!(registry
        .validate_fields(PlanetSurfaceAnchor::TYPE_ID, &fields)
        .is_ok());
    let restored = (extension.deserialize.unwrap())(&fields)
        .downcast::<PlanetSurfaceAnchor>()
        .unwrap();
    assert_eq!(*restored, anchor);

    let mut legacy_fields = fields.clone();
    legacy_fields.remove("terrain_volume_id");
    let legacy = (extension.deserialize.unwrap())(&legacy_fields)
        .downcast::<PlanetSurfaceAnchor>()
        .unwrap();
    assert!(legacy.terrain_volume_id.is_empty());
    assert!(registry
        .validate_fields(PlanetSurfaceAnchor::TYPE_ID, &legacy_fields)
        .is_ok());

    let mut invalid = BTreeMap::from([("support_samples".into(), Value::UInt(u64::MAX))]);
    assert!(registry
        .validate_fields(PlanetSurfaceAnchor::TYPE_ID, &invalid)
        .is_err());
    invalid.insert("unknown".into(), Value::Bool(true));
    assert!(registry
        .validate_fields(PlanetSurfaceAnchor::TYPE_ID, &invalid)
        .is_err());
}

#[test]
fn local_transform_tracks_large_world_origins_without_position_collapse() {
    let query = PlanetTerrainQuery::new(&TerrainVolume {
        topology: TerrainTopology::CubeSphere,
        planet_center: [1.0e12, -2.0e12, 3.0e12],
        planet_radius: 6_000_000.0,
        height_scale: 0.0,
        planet_max_lod: 0,
        lod_distances: vec![10_000.0],
        ..TerrainVolume::default()
    })
    .unwrap();
    let placement = PlanetSurfaceAnchor {
        direction: Vec3::new(0.2, 0.9, -0.3).normalize().as_dvec3().to_array(),
        footprint_radius: 0.0,
        max_slope_radians: 1.0e-4,
        ..Default::default()
    }
    .resolve(&query)
    .unwrap();
    let transform = placement.to_transform(query.center()).unwrap();
    assert!((f64::from(transform.translation.length()) - query.radius()).abs() < 1.0);
}
