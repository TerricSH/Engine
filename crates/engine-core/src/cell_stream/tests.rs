use std::collections::BTreeMap;
use std::path::PathBuf;

use engine_asset::partition::{CellBounds, WorldPartition};
use engine_asset::project::GameProject;
use engine_scene::Scene;
use engine_serialize::AssetId;
use glam::Vec3;

use crate::EngineRuntime;

use super::*;

include!("tests/common.rs");
include!("tests/hysteresis_and_assets.rs");
include!("tests/residency_and_rebaseline.rs");
include!("tests/validation.rs");
include!("tests/physics_and_origin.rs");
