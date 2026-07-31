use std::path::Path;

use engine_asset::partition::WorldPartition;
use engine_asset::project::GameProject;
use engine_core::cell_stream::{CellStreamingConfig, CellStreamingDriver};
use engine_core::game_loop::GameLoop;
use engine_core::{CookedAssetLoadReport, EngineConfig, EngineRuntime, SceneLoadRequest};
use engine_scene::Scene;

use crate::project_cli::ProjectRunRequest;

mod assets;
mod headless;
mod run;
mod transitions;
mod windowed;

#[cfg(any(test, feature = "tooling-editor"))]
pub(crate) use assets::load_project_assets;
#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
pub(crate) use assets::missing_render_asset_dependencies;
use assets::missing_runtime_asset_dependencies;
pub(crate) use assets::prepare_project_runtime;
use headless::{format_diagnostics, run_headless};
#[cfg(all(feature = "runtime-subsystems", feature = "backend-vulkan"))]
use run::route_project_player_ui_event;
pub use run::run_project;
use run::MAX_CHAINED_SCENE_TRANSITIONS;
pub(crate) use transitions::process_pending_scene_transitions;
#[cfg(all(test, feature = "subsystem-terrain"))]
use transitions::settle_planet_scene_transition;
#[cfg(test)]
use transitions::{
    capture_scene_transition_rollback, rollback_failed_scene_transition,
    rollback_failed_scene_transition_classified, transition_to_project_scene,
    SceneTransitionFailure,
};
use transitions::{
    create_cell_streaming_driver, create_game_loop, load_startup_scene, tick_cell_streaming,
};
use windowed::run_windowed;

#[cfg(test)]
mod tests {
    include!("project_app/tests/common.rs");
    include!("project_app/tests/streaming.rs");
    include!("project_app/tests/transitions.rs");
}
