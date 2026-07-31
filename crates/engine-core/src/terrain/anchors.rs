use super::*;
use engine_terrain::{PlanetSurfaceOwnerKey, PlanetTerrainQuery};

pub(super) fn update_surface_anchors(
    system: &mut TerrainSystem,
    engine: &mut EngineRuntime,
    volumes: &BTreeMap<TerrainVolumeId, ActiveTerrainVolume>,
    world_origin: [f64; 3],
) -> bool {
    system.surface_occupancy = PlanetSurfaceOccupancy::default();
    let queries = volumes
        .values()
        .filter(|volume| volume.volume.topology == TerrainTopology::CubeSphere)
        .filter_map(|volume| {
            PlanetTerrainQuery::new(&volume.volume).ok().map(|query| {
                (
                    volume.persistent_id.as_deref(),
                    &volume.occupancy_scope,
                    query,
                )
            })
        })
        .collect::<Vec<_>>();
    if system
        .stats
        .last_error
        .as_deref()
        .is_some_and(|message| message.starts_with("planet surface anchor '"))
    {
        system.stats.last_error = None;
    }
    let anchors = engine.with_world(|world| {
        let mut anchors = world
            .query::<PlanetSurfaceAnchor>()
            .filter(|(_, anchor)| anchor.enabled)
            .map(|(entity, anchor)| {
                let (identity_label, owner) =
                    if let Some(persistent_id) = world.persistent_id(entity) {
                        (
                            persistent_id.to_string(),
                            PlanetSurfaceOwnerKey::Persistent(persistent_id.to_string()),
                        )
                    } else {
                        (
                            runtime_entity_identity_label(entity),
                            PlanetSurfaceOwnerKey::from_runtime_entity(
                                entity.index(),
                                entity.generation(),
                            ),
                        )
                    };
                (
                    identity_label,
                    owner,
                    entity,
                    anchor.clone(),
                    world.get::<Transform>(entity).cloned(),
                )
            })
            .collect::<Vec<_>>();
        anchors.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        anchors
    });
    let Some(anchors) = anchors else {
        return false;
    };

    let mut updates = Vec::new();
    for (identity_label, occupancy_owner, entity, anchor, current) in anchors {
        let selected = if anchor.terrain_volume_id.is_empty() {
            if queries.len() == 1 {
                queries
                    .first()
                    .map(|(_, occupancy_scope, query)| (*occupancy_scope, query))
            } else {
                system.stats.last_error = Some(if queries.is_empty() {
                    format!(
                        "planet surface anchor '{identity_label}' has no enabled cube-sphere terrain volume"
                    )
                } else {
                    format!(
                        "planet surface anchor '{identity_label}' must set terrain_volume_id when multiple cube-sphere terrain volumes are enabled"
                    )
                });
                None
            }
        } else {
            let selected = queries
                .iter()
                .find(|(persistent_id, _, _)| {
                    *persistent_id == Some(anchor.terrain_volume_id.as_str())
                })
                .map(|(_, occupancy_scope, query)| (*occupancy_scope, query));
            if selected.is_none() {
                system.stats.last_error = Some(format!(
                    "planet surface anchor '{identity_label}' references unavailable terrain volume '{}'",
                    anchor.terrain_volume_id
                ));
            }
            selected
        };
        let Some((occupancy_scope, query)) = selected else {
            continue;
        };
        let Some(current) = current else {
            system.stats.last_error = Some(format!(
                "planet surface anchor '{identity_label}' requires a Transform"
            ));
            continue;
        };
        if current.parent.is_some() {
            system.stats.last_error = Some(format!(
                "planet surface anchor '{identity_label}' must be a root transform"
            ));
            continue;
        }
        let placement = match anchor.resolve(query) {
            Ok(placement) => placement,
            Err(error) => {
                system.stats.last_error = Some(format!(
                    "planet surface anchor '{identity_label}' is invalid: {error}"
                ));
                continue;
            }
        };
        let mut next = match placement.to_transform(world_origin) {
            Ok(transform) => transform,
            Err(error) => {
                system.stats.last_error = Some(format!(
                    "planet surface anchor '{identity_label}' cannot resolve locally: {error}"
                ));
                continue;
            }
        };
        if let Err(error) = system.surface_occupancy.reserve_scoped(
            occupancy_scope.clone(),
            occupancy_owner,
            placement,
            anchor.blocks_navigation,
            0.0,
        ) {
            system.stats.last_error = Some(format!(
                "planet surface anchor '{identity_label}' cannot reserve its footprint: {error}"
            ));
            continue;
        }
        next.scale = current.scale;
        let translation_changed = current.translation.distance_squared(next.translation) > 1.0e-10;
        let rotation_changed = current.rotation.dot(next.rotation).abs() < 1.0 - 1.0e-7;
        if translation_changed || rotation_changed {
            updates.push((entity, next));
        }
    }
    if updates.is_empty() {
        return false;
    }
    engine
        .with_world_mut(|world| {
            for (entity, transform) in updates {
                if let Some(current) = world.get_mut::<Transform>(entity) {
                    *current = transform;
                }
            }
        })
        .is_some()
}

#[cfg(test)]
mod tests {
    use engine_scene::World;

    use super::*;
    use crate::EngineConfig;

    #[test]
    fn runtime_entities_get_distinct_occupancy_and_native_surface_transforms() {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut world = World::new();
        let volume = world.create_entity();
        world.add_component(
            volume,
            TerrainVolume {
                topology: TerrainTopology::CubeSphere,
                planet_radius: 100.0,
                planet_max_lod: 0,
                lod_distances: vec![500.0],
                height_scale: 0.0,
                ..TerrainVolume::default()
            },
        );
        let first = world.create_entity();
        world.add_component(first, Transform::default());
        world.add_component(
            first,
            PlanetSurfaceAnchor {
                direction: [1.0, 0.0, 0.0],
                footprint_radius: 1.0,
                max_slope_radians: 1.0e-4,
                max_height_delta: 1.0,
                ..PlanetSurfaceAnchor::default()
            },
        );
        let second = world.create_entity();
        world.add_component(second, Transform::default());
        world.add_component(
            second,
            PlanetSurfaceAnchor {
                direction: [0.0, 1.0, 0.0],
                footprint_radius: 1.0,
                max_slope_radians: 1.0e-4,
                max_height_delta: 1.0,
                ..PlanetSurfaceAnchor::default()
            },
        );
        let invalid = world.create_entity();
        world.add_component(
            invalid,
            PlanetSurfaceAnchor {
                direction: [0.0, 0.0, 1.0],
                ..PlanetSurfaceAnchor::default()
            },
        );
        runtime.set_world(world);

        let mut terrain = TerrainSystem::default();
        terrain.tick(&mut runtime, Some([0.0, 0.0, 150.0]));
        assert_eq!(terrain.surface_occupancy().reservations().len(), 2);
        let positions = runtime
            .with_world(|world| {
                [
                    world.get::<Transform>(first).unwrap().translation,
                    world.get::<Transform>(second).unwrap().translation,
                ]
            })
            .unwrap();
        assert!((positions[0].length() - 100.0).abs() < 1.0e-4);
        assert!((positions[1].length() - 100.0).abs() < 1.0e-4);
    }

    #[test]
    fn anchors_resolve_against_explicit_planets_and_occupancy_is_isolated() {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut world = World::new();
        for (id, center, radius) in [
            ("planet-a", [0.0, 0.0, 0.0], 100.0),
            ("planet-b", [1_000.0, 0.0, 0.0], 200.0),
        ] {
            let planet = world.create_persistent_entity(id).unwrap();
            world.add_component(
                planet,
                TerrainVolume {
                    topology: TerrainTopology::CubeSphere,
                    planet_center: center,
                    planet_radius: radius,
                    planet_max_lod: 0,
                    base_resolution: 3,
                    lod_distances: vec![500.0],
                    height_scale: 0.0,
                    ..TerrainVolume::default()
                },
            );
        }
        let anchor = |terrain_volume_id: &str| PlanetSurfaceAnchor {
            terrain_volume_id: terrain_volume_id.to_string(),
            direction: [1.0, 0.0, 0.0],
            footprint_radius: 1.0,
            max_slope_radians: 1.0e-4,
            max_height_delta: 1.0,
            ..PlanetSurfaceAnchor::default()
        };
        let first = world.create_persistent_entity("anchor-a").unwrap();
        world.add_component(first, Transform::default());
        world.add_component(first, anchor("planet-a"));
        let second = world.create_persistent_entity("anchor-b").unwrap();
        world.add_component(second, Transform::default());
        world.add_component(second, anchor("planet-b"));
        let ambiguous = world
            .create_persistent_entity("anchor-legacy-ambiguous")
            .unwrap();
        world.add_component(ambiguous, Transform::default());
        world.add_component(ambiguous, anchor(""));
        runtime.set_world(world);

        let mut terrain = TerrainSystem::default();
        terrain.tick(&mut runtime, Some([0.0, 0.0, 150.0]));

        assert_eq!(
            terrain
                .surface_occupancy()
                .reservations_for_volume("planet-a")
                .count(),
            1
        );
        assert_eq!(
            terrain
                .surface_occupancy()
                .reservations_for_volume("planet-b")
                .count(),
            1
        );
        let positions = runtime
            .with_world(|world| {
                [
                    world.get::<Transform>(first).unwrap().translation,
                    world.get::<Transform>(second).unwrap().translation,
                    world.get::<Transform>(ambiguous).unwrap().translation,
                ]
            })
            .unwrap();
        assert!((positions[0].x - 100.0).abs() < 1.0e-4);
        assert!((positions[1].x - 1_200.0).abs() < 1.0e-4);
        assert_eq!(positions[2], Vec3::ZERO);
        assert!(terrain
            .binding_stats()
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("must set terrain_volume_id")));
    }

    #[test]
    fn authored_runtime_shaped_anchor_id_cannot_alias_anonymous_owner() {
        let mut runtime = EngineRuntime::new(EngineConfig::default());
        let mut world = World::new();
        let planet = world.create_persistent_entity("planet").unwrap();
        world.add_component(
            planet,
            TerrainVolume {
                topology: TerrainTopology::CubeSphere,
                planet_radius: 100.0,
                planet_max_lod: 0,
                base_resolution: 3,
                lod_distances: vec![500.0],
                height_scale: 0.0,
                ..TerrainVolume::default()
            },
        );
        let anchor = |direction| PlanetSurfaceAnchor {
            terrain_volume_id: "planet".into(),
            direction,
            footprint_radius: 1.0,
            max_slope_radians: 1.0e-4,
            max_height_delta: 1.0,
            ..PlanetSurfaceAnchor::default()
        };
        let anonymous = world.create_entity();
        world.add_component(anonymous, Transform::default());
        world.add_component(anonymous, anchor([1.0, 0.0, 0.0]));
        let authored_label = runtime_entity_identity_label(anonymous);
        let authored = world.create_persistent_entity(&authored_label).unwrap();
        world.add_component(authored, Transform::default());
        world.add_component(authored, anchor([0.0, 1.0, 0.0]));
        runtime.set_world(world);

        let mut terrain = TerrainSystem::default();
        terrain.tick(&mut runtime, Some([0.0, 0.0, 150.0]));
        let scope = PlanetSurfaceVolumeKey::Persistent("planet".into());
        let owners = terrain
            .surface_occupancy()
            .reservations_for_scope(&scope)
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        assert_eq!(owners.len(), 2);
        assert!(owners.contains(&PlanetSurfaceOwnerKey::Persistent(authored_label)));
        assert!(owners.contains(&PlanetSurfaceOwnerKey::from_runtime_entity(
            anonymous.index(),
            anonymous.generation()
        )));
        assert_ne!(owners[0].stable_key(), owners[1].stable_key());
    }
}
