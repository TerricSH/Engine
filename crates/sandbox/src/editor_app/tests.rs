use super::*;
use engine_asset::cook::{MaterialSource, MATERIAL_SOURCE_SCHEMA};
use engine_asset::project::ProjectManifest;
use engine_renderer::{BackendRenderer, MaterialUpload, MeshUpload};

include!("tests/frame_and_jobs.rs");
include!("tests/assets_and_scripts.rs");
include!("tests/documents.rs");
include!("tests/prefabs_and_preview.rs");
include!("tests/materials.rs");
include!("tests/gizmos.rs");
