use std::collections::BTreeMap;

use engine_scene::{active_camera_world_position, World};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use engine_terrain::{
    PlanetSceneBand, PlanetSceneTransitionConfig, TerrainTopology, TerrainVolume,
};

pub(super) struct PlanetTransitionFrame {
    pub scene_id: String,
    pub camera_logical: Option<[f64; 3]>,
    pub resolved: Vec<ResolvedTransition>,
    pub valid_volumes: BTreeMap<String, TerrainVolume>,
    pub issues: Vec<RuntimeIssue>,
}

pub(super) struct ResolvedTransition {
    pub controller_id: String,
    pub config: PlanetSceneTransitionConfig,
    pub terrain_volume_id: String,
    pub terrain_volume: TerrainVolume,
    pub band: PlanetSceneBand,
}

pub(super) struct RuntimeIssue {
    pub key: String,
    pub diagnostic: Diagnostic,
}

impl PlanetTransitionFrame {
    pub fn capture(world: &World, scene_id: String) -> Self {
        let mut issues = Vec::new();
        let configurations = world
            .query::<PlanetSceneTransitionConfig>()
            .filter(|(_, config)| config.enabled)
            .filter_map(|(entity, config)| {
                let band = band_for_scene(config, &scene_id)?;
                let Some(controller_id) = world.persistent_id(entity).map(str::to_owned) else {
                    issues.push(RuntimeIssue::new(
                        format!("missing-controller-id:{entity:?}"),
                        None,
                        "enabled planet scene transition has no persistent entity ID",
                    ));
                    return None;
                };
                if let Err(error) = config.validate() {
                    issues.push(RuntimeIssue::new(
                        format!("invalid-config:{controller_id}"),
                        Some(&controller_id),
                        format!("planet scene transition configuration is invalid: {error}"),
                    ));
                    return None;
                }
                Some((controller_id, config.clone(), band))
            })
            .collect::<Vec<_>>();

        let valid_volumes = world
            .query::<TerrainVolume>()
            .filter(|(_, volume)| {
                volume.enabled
                    && volume.topology == TerrainTopology::CubeSphere
                    && volume.validate().is_ok()
            })
            .filter_map(|(entity, volume)| {
                world
                    .persistent_id(entity)
                    .map(|id| (id.to_string(), volume.clone()))
            })
            .collect::<BTreeMap<_, _>>();

        if configurations.is_empty() {
            return Self {
                scene_id,
                camera_logical: logical_camera_position(world),
                resolved: Vec::new(),
                valid_volumes,
                issues,
            };
        }

        let camera_logical = logical_camera_position(world);
        if camera_logical.is_none() {
            issues.push(RuntimeIssue::new(
                "missing-camera".into(),
                None,
                "planet scene transitions require an enabled active camera with a valid transform",
            ));
        }
        let mut resolved = Vec::new();
        for (controller_id, config, band) in configurations {
            let requested_id = config.terrain_volume_id.trim();
            let selected = if requested_id.is_empty() {
                match valid_volumes.len() {
                    1 => valid_volumes
                        .first_key_value()
                        .map(|(id, volume)| (id.clone(), volume.clone())),
                    count => {
                        issues.push(RuntimeIssue::new(
                            format!("ambiguous-terrain:{controller_id}:{count}"),
                            Some(&controller_id),
                            format!(
                                "planet scene transition omitted terrain_volume_id, but the scene has {count} enabled valid CubeSphere terrain volumes; set an explicit persistent ID"
                            ),
                        ));
                        None
                    }
                }
            } else {
                match valid_volumes.get(requested_id) {
                    Some(volume) => Some((requested_id.to_string(), volume.clone())),
                    None => {
                        issues.push(RuntimeIssue::new(
                            format!("invalid-terrain:{controller_id}:{requested_id}"),
                            Some(&controller_id),
                            format!(
                                "planet scene transition target '{requested_id}' is missing or is not an enabled valid CubeSphere TerrainVolume"
                            ),
                        ));
                        None
                    }
                }
            };
            let Some((terrain_volume_id, terrain_volume)) = selected else {
                continue;
            };
            resolved.push(ResolvedTransition {
                controller_id,
                config,
                terrain_volume_id,
                terrain_volume,
                band,
            });
        }
        Self {
            scene_id,
            camera_logical,
            resolved,
            valid_volumes,
            issues,
        }
    }
}

impl RuntimeIssue {
    pub fn terrain_query(controller_id: &str, terrain_id: &str, error: String) -> Self {
        Self::new(
            format!("terrain-query:{controller_id}:{terrain_id}:{error}"),
            Some(controller_id),
            format!("could not query target planet '{terrain_id}': {error}"),
        )
    }

    pub fn sample(controller_id: &str, error: engine_terrain::PlanetSceneTransitionError) -> Self {
        Self::new(
            format!("invalid-sample:{controller_id}:{error}"),
            Some(controller_id),
            format!("planet scene transition sample was rejected: {error}"),
        )
    }

    pub fn missing_camera(controller_id: &str) -> Self {
        Self::new(
            format!("missing-camera:{controller_id}"),
            Some(controller_id),
            "planet scene transition cannot evaluate the retained surface policy without an active camera",
        )
    }

    pub fn transaction_ids_exhausted() -> Self {
        Self::new(
            "transaction-ids-exhausted".into(),
            None,
            "planet scene transition transaction ID space is exhausted; refusing to reuse an acknowledgement identity",
        )
    }

    fn new(key: String, controller_id: Option<&str>, message: impl Into<String>) -> Self {
        let mut diagnostic = Diagnostic::new(
            "PLANET_SCENE_TRANSITION",
            DiagnosticSeverity::Warning,
            "engine-core.planet-scene-transition",
            message,
        );
        diagnostic.recoverable = true;
        if let Some(controller_id) = controller_id {
            diagnostic
                .fields
                .insert("controller_id".into(), controller_id.into());
        }
        Self { key, diagnostic }
    }
}

fn band_for_scene(config: &PlanetSceneTransitionConfig, scene_id: &str) -> Option<PlanetSceneBand> {
    if scene_id == config.orbit_scene_id {
        Some(PlanetSceneBand::Orbit)
    } else if scene_id == config.surface_scene_id {
        Some(PlanetSceneBand::Surface)
    } else {
        None
    }
}

fn logical_camera_position(world: &World) -> Option<[f64; 3]> {
    let camera = active_camera_world_position(world)?;
    let origin = world.world_origin();
    Some([
        origin[0] + f64::from(camera.x),
        origin[1] + f64::from(camera.y),
        origin[2] + f64::from(camera.z),
    ])
}
