//! Optional world partition manifest (`world.partition.json`).
//!
//! World partition foundations (ENG-02, Phase 2): a declarative cell → scene
//! mapping with Cartesian or f64 planetary bounds, validated by
//! `sandbox project check`. Runtime additive streaming is active;
//! this module is the shared parse/validate layer consumed by the cell driver.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::project::{validate_scene_id, GameProject, ProjectManifest};

/// Partition manifest contract understood by this engine version.
pub const WORLD_PARTITION_SCHEMA: &str = "WorldPartition-v0";
/// Conventional partition manifest file name, at the project root next to
/// `game.project.json`.
pub const WORLD_PARTITION_FILE_NAME: &str = "world.partition.json";

/// Axis-aligned world-space bounds of one partition cell.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellBounds {
    /// World-space centre of the cell.
    pub center: [f32; 3],
    /// World-space half extents of the cell. Every component must be finite
    /// and non-negative.
    pub half_extents: [f32; 3],
}

/// Precision-preserving region on or above a spherical planet. This avoids
/// representing planet centres and interplanetary camera positions as f32.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanetaryCellBounds {
    pub planet_center: [f64; 3],
    /// Unit direction from the planet centre to the region centre.
    pub direction: [f64; 3],
    /// Half-angle of the region's surface cap, in radians.
    pub angular_radius: f64,
    /// Inclusive altitude interval relative to the base planet radius.
    pub min_altitude: f64,
    pub max_altitude: f64,
    pub planet_radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StreamingCellBounds {
    Cartesian(CellBounds),
    Planetary(PlanetaryCellBounds),
}

impl StreamingCellBounds {
    pub fn contains(self, factor: f64, point: [f64; 3]) -> bool {
        let point = glam::DVec3::from_array(point);
        match self {
            Self::Cartesian(bounds) => (0..3).all(|axis| {
                let half_extent = f64::from(bounds.half_extents[axis]) * factor;
                (point[axis] - f64::from(bounds.center[axis])).abs() <= half_extent
            }),
            Self::Planetary(bounds) => {
                let radial = point - glam::DVec3::from_array(bounds.planet_center);
                let distance = radial.length();
                if distance <= f64::EPSILON {
                    return false;
                }
                let altitude = distance - bounds.planet_radius;
                let altitude_middle = (bounds.min_altitude + bounds.max_altitude) * 0.5;
                let altitude_half = (bounds.max_altitude - bounds.min_altitude) * 0.5 * factor;
                if (altitude - altitude_middle).abs() > altitude_half {
                    return false;
                }
                let target = glam::DVec3::from_array(bounds.direction).normalize_or_zero();
                radial.normalize().dot(target).clamp(-1.0, 1.0).acos()
                    <= bounds.angular_radius * factor
            }
        }
    }
}

/// One world partition cell: a cataloged scene plus its world-space bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartitionCell {
    /// Scene catalog ID this cell's content is loaded from. Must reference an
    /// entry of the project's scene catalog.
    pub scene: String,
    pub bounds: CellBounds,
    /// When present, runtime streaming uses this f64 spherical region instead
    /// of the legacy Cartesian AABB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planetary_bounds: Option<PlanetaryCellBounds>,
}

impl PartitionCell {
    pub fn streaming_bounds(&self) -> StreamingCellBounds {
        self.planetary_bounds.map_or(
            StreamingCellBounds::Cartesian(self.bounds),
            StreamingCellBounds::Planetary,
        )
    }
}

/// Versioned world partition manifest stored at the project root.
///
/// Cells are keyed by stable cell ID (same identifier rules as scene IDs) in
/// deterministic order. Cell bounds **may overlap**: overlapping cells are a
/// legitimate way to layer content (for example a dense gameplay cell over a
/// large background cell), so overlap is allowed and not reported.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldPartition {
    pub schema: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cells: BTreeMap<String, PartitionCell>,
}

/// Failure returned when parsing or validating a world partition manifest.
#[derive(Debug, Error)]
pub enum PartitionError {
    #[error("unsupported world partition schema: {0}")]
    UnsupportedSchema(String),
    #[error("invalid world partition cell ID: {0:?}")]
    InvalidCellId(String),
    #[error(
        "world partition cell IDs must be unique under portable case-insensitive comparison: {0:?}"
    )]
    DuplicateCellId(String),
    #[error("world partition cell {cell_id:?} references unknown project scene {scene_id:?}")]
    UnknownScene { cell_id: String, scene_id: String },
    #[error("world partition cell {cell_id:?} has invalid bounds: {reason}")]
    InvalidBounds { cell_id: String, reason: String },
    #[error("failed to access {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid world partition JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl WorldPartition {
    /// Validate the schema marker, cell IDs, and bounds of every cell.
    ///
    /// Scene references are checked separately by
    /// [`validate_for_project`](Self::validate_for_project).
    pub fn validate(&self) -> Result<(), PartitionError> {
        if self.schema != WORLD_PARTITION_SCHEMA {
            return Err(PartitionError::UnsupportedSchema(self.schema.clone()));
        }
        let mut portable_cell_ids = BTreeSet::new();
        for (cell_id, cell) in &self.cells {
            if validate_scene_id(cell_id).is_err() {
                return Err(PartitionError::InvalidCellId(cell_id.clone()));
            }
            if !portable_cell_ids.insert(cell_id.to_ascii_lowercase()) {
                return Err(PartitionError::DuplicateCellId(cell_id.clone()));
            }
            if !cell.bounds.center.iter().all(|value| value.is_finite()) {
                return Err(PartitionError::InvalidBounds {
                    cell_id: cell_id.clone(),
                    reason: "center components must be finite".into(),
                });
            }
            if !cell
                .bounds
                .half_extents
                .iter()
                .all(|value| value.is_finite())
            {
                return Err(PartitionError::InvalidBounds {
                    cell_id: cell_id.clone(),
                    reason: "half extents must be finite".into(),
                });
            }
            if cell.bounds.half_extents.iter().any(|value| *value < 0.0) {
                return Err(PartitionError::InvalidBounds {
                    cell_id: cell_id.clone(),
                    reason: "half extents must be non-negative".into(),
                });
            }
            if let Some(bounds) = cell.planetary_bounds {
                let direction_length_squared = bounds
                    .direction
                    .iter()
                    .map(|value| value * value)
                    .sum::<f64>();
                let valid = bounds
                    .planet_center
                    .iter()
                    .chain(bounds.direction.iter())
                    .all(|value| value.is_finite())
                    && direction_length_squared.is_finite()
                    && direction_length_squared > f64::EPSILON
                    && bounds.angular_radius.is_finite()
                    && (0.0..=std::f64::consts::PI).contains(&bounds.angular_radius)
                    && bounds.min_altitude.is_finite()
                    && bounds.max_altitude.is_finite()
                    && bounds.min_altitude <= bounds.max_altitude
                    && bounds.planet_radius.is_finite()
                    && bounds.planet_radius > 0.0;
                if !valid {
                    return Err(PartitionError::InvalidBounds {
                        cell_id: cell_id.clone(),
                        reason: "planetary bounds require finite centre/direction, positive radius, an ordered altitude interval, and angular_radius in [0, pi]".into(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate intrinsic rules plus every cell's scene reference against the
    /// project's scene catalog.
    pub fn validate_for_project(&self, manifest: &ProjectManifest) -> Result<(), PartitionError> {
        self.validate()?;
        let catalog = manifest.scene_catalog();
        for (cell_id, cell) in &self.cells {
            if !catalog.contains_key(&cell.scene) {
                return Err(PartitionError::UnknownScene {
                    cell_id: cell_id.clone(),
                    scene_id: cell.scene.clone(),
                });
            }
        }
        Ok(())
    }

    /// Load and validate the project's optional partition manifest.
    ///
    /// Returns `Ok(None)` when the project has no `world.partition.json`;
    /// when the file exists it must parse and validate against the project
    /// scene catalog.
    pub fn load_for_project(project: &GameProject) -> Result<Option<Self>, PartitionError> {
        let path = project.root.join(WORLD_PARTITION_FILE_NAME);
        if !path.is_file() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(&path).map_err(|source| PartitionError::Io {
            path: path.clone(),
            source,
        })?;
        let partition: WorldPartition =
            serde_json::from_str(&json).map_err(|source| PartitionError::Json {
                path: path.clone(),
                source,
            })?;
        partition.validate_for_project(&project.manifest)?;
        Ok(Some(partition))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{GameProject, ProjectManifest, GAME_PROJECT_FILE_NAME};

    fn cell(scene: &str, center: [f32; 3], half_extents: [f32; 3]) -> PartitionCell {
        PartitionCell {
            scene: scene.to_string(),
            bounds: CellBounds {
                center,
                half_extents,
            },
            planetary_bounds: None,
        }
    }

    fn partition_json(cells: &str) -> String {
        format!("{{ \"schema\": \"{WORLD_PARTITION_SCHEMA}\", \"cells\": {{ {cells} }} }}")
    }

    fn project_at(root: &std::path::Path) -> GameProject {
        let manifest = ProjectManifest::new("Partition Game");
        GameProject {
            manifest: manifest.clone(),
            manifest_path: root.join(GAME_PROJECT_FILE_NAME),
            root: root.to_path_buf(),
            startup_scene: root.join(&manifest.startup_scene),
            asset_source: root.join(&manifest.asset_source),
            cooked_assets: root.join(&manifest.cooked_assets),
            script_project: None,
            script_assembly: None,
            input_actions: None,
        }
    }

    fn unique_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("engine-partition-{name}-{unique}"))
    }

    #[test]
    fn partition_manifest_roundtrips_and_validates() {
        let json = partition_json(
            "\"cell_forest\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } },\n\
             \"cell_town\": { \"scene\": \"main\", \"bounds\": { \"center\": [128.0, 0.0, 0.0], \"half_extents\": [32.0, 8.0, 32.0] } }",
        );
        let partition: WorldPartition = serde_json::from_str(&json).expect("parse partition");
        assert_eq!(partition.cells.len(), 2);
        assert_eq!(partition.cells["cell_forest"].scene, "main");
        assert_eq!(
            partition.cells["cell_town"].bounds.half_extents,
            [32.0, 8.0, 32.0]
        );
        partition.validate().expect("valid partition");

        // Overlapping cells are allowed (layering); validation must accept.
        let overlapping = partition_json(
            "\"cell_base\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } },\n\
             \"cell_layer\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [64.0, 16.0, 64.0] } }",
        );
        let overlapping: WorldPartition =
            serde_json::from_str(&overlapping).expect("parse overlapping");
        overlapping.validate().expect("overlap is allowed");
    }

    #[test]
    fn partition_rejects_unknown_schema() {
        let partition = WorldPartition {
            schema: "WorldPartition-v9".to_string(),
            cells: BTreeMap::new(),
        };
        assert!(matches!(
            partition.validate(),
            Err(PartitionError::UnsupportedSchema(schema)) if schema == "WorldPartition-v9"
        ));
    }

    #[test]
    fn planetary_bounds_preserve_large_centres_and_altitude_bands() {
        let bounds = StreamingCellBounds::Planetary(PlanetaryCellBounds {
            planet_center: [9.0e12, -4.0e12, 2.0e12],
            direction: [1.0, 0.0, 0.0],
            angular_radius: 0.1,
            min_altitude: -100.0,
            max_altitude: 1_000.0,
            planet_radius: 6_000.0,
        });
        assert!(bounds.contains(1.0, [9.0e12 + 6_500.0, -4.0e12, 2.0e12]));
        assert!(!bounds.contains(1.0, [9.0e12, -4.0e12 + 6_500.0, 2.0e12]));
        assert!(!bounds.contains(1.0, [9.0e12 + 9_000.0, -4.0e12, 2.0e12]));
    }

    #[test]
    fn partition_rejects_invalid_cell_ids() {
        for bad_id in ["", ".", "..", "cell/forest", "cell forest", "cell\\x"] {
            let mut cells = BTreeMap::new();
            cells.insert(
                bad_id.to_string(),
                cell("main", [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            );
            let partition = WorldPartition {
                schema: WORLD_PARTITION_SCHEMA.to_string(),
                cells,
            };
            assert!(
                matches!(
                    partition.validate(),
                    Err(PartitionError::InvalidCellId(id)) if id == bad_id
                ),
                "invalid cell ID accepted: {bad_id:?}"
            );
        }
    }

    #[test]
    fn partition_rejects_case_conflicting_cell_ids() {
        let mut cells = BTreeMap::new();
        cells.insert("CellA".to_string(), cell("main", [0.0, 0.0, 0.0], [1.0; 3]));
        cells.insert("cella".to_string(), cell("main", [8.0, 0.0, 0.0], [1.0; 3]));
        let partition = WorldPartition {
            schema: WORLD_PARTITION_SCHEMA.to_string(),
            cells,
        };
        assert!(matches!(
            partition.validate(),
            Err(PartitionError::DuplicateCellId(id)) if id == "cella"
        ));
    }

    #[test]
    fn partition_rejects_non_finite_and_negative_bounds() {
        let cases: [([f32; 3], [f32; 3]); 3] = [
            ([f32::NAN, 0.0, 0.0], [1.0; 3]),
            ([0.0; 3], [f32::INFINITY, 1.0, 1.0]),
            ([0.0; 3], [-1.0, 1.0, 1.0]),
        ];
        for (center, half_extents) in cases {
            let mut cells = BTreeMap::new();
            cells.insert("cell".to_string(), cell("main", center, half_extents));
            let partition = WorldPartition {
                schema: WORLD_PARTITION_SCHEMA.to_string(),
                cells,
            };
            assert!(
                matches!(
                    partition.validate(),
                    Err(PartitionError::InvalidBounds { .. })
                ),
                "invalid bounds accepted: {center:?} / {half_extents:?}"
            );
        }
    }

    #[test]
    fn partition_rejects_unknown_scene_references() {
        let root = unique_dir("unknown-scene");
        std::fs::create_dir_all(&root).expect("root");
        let project = project_at(&root);
        let mut cells = BTreeMap::new();
        cells.insert(
            "cell_missing".to_string(),
            cell("missing_scene", [0.0, 0.0, 0.0], [1.0; 3]),
        );
        let partition = WorldPartition {
            schema: WORLD_PARTITION_SCHEMA.to_string(),
            cells,
        };
        assert!(matches!(
            partition.validate_for_project(&project.manifest),
            Err(PartitionError::UnknownScene { cell_id, scene_id })
                if cell_id == "cell_missing" && scene_id == "missing_scene"
        ));

        let mut cells = BTreeMap::new();
        cells.insert(
            "cell_main".to_string(),
            cell("main", [0.0, 0.0, 0.0], [1.0; 3]),
        );
        let partition = WorldPartition {
            schema: WORLD_PARTITION_SCHEMA.to_string(),
            cells,
        };
        partition
            .validate_for_project(&project.manifest)
            .expect("cataloged scene resolves");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn load_for_project_returns_none_without_manifest_and_validates_present_one() {
        let root = unique_dir("load");
        std::fs::create_dir_all(&root).expect("root");
        let project = project_at(&root);

        assert!(WorldPartition::load_for_project(&project)
            .expect("no partition file")
            .is_none());

        std::fs::write(
            root.join(WORLD_PARTITION_FILE_NAME),
            partition_json(
                "\"cell_main\": { \"scene\": \"main\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [16.0, 4.0, 16.0] } }",
            ),
        )
        .expect("write partition");
        let partition = WorldPartition::load_for_project(&project)
            .expect("load partition")
            .expect("partition present");
        assert_eq!(partition.cells.len(), 1);

        // A manifest that fails validation must surface an error, not silence.
        std::fs::write(
            root.join(WORLD_PARTITION_FILE_NAME),
            partition_json(
                "\"cell_bad\": { \"scene\": \"not-a-scene\", \"bounds\": { \"center\": [0.0, 0.0, 0.0], \"half_extents\": [16.0, 4.0, 16.0] } }",
            ),
        )
        .expect("write invalid partition");
        assert!(matches!(
            WorldPartition::load_for_project(&project),
            Err(PartitionError::UnknownScene { .. })
        ));

        std::fs::write(root.join(WORLD_PARTITION_FILE_NAME), "{ not json")
            .expect("write malformed partition");
        assert!(matches!(
            WorldPartition::load_for_project(&project),
            Err(PartitionError::Json { .. })
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
