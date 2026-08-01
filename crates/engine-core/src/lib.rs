#![forbid(unsafe_code)]

pub mod diagnostics;
pub use diagnostics::*;
pub mod component_audit;
pub mod cooked_assets;
pub use cooked_assets::*;
pub mod runtime_mesh;
pub use runtime_mesh::*;
pub mod asset_stream;
pub use asset_stream::*;
mod runtime;
pub(crate) use runtime::{
    install_builtin_render_assets, missing_registered_render_asset, scene_load_diagnostic,
    validate_registered_asset_id, SyncedRenderResources,
};
pub use runtime::{EngineConfig, EngineRuntime, EngineRuntimeBuilder, SceneLoadRequest};
pub mod cell_stream;
pub use cell_stream::{CellStreamingConfig, CellStreamingDriver};
#[cfg(feature = "subsystem-network")]
pub use engine_network;
#[cfg(feature = "subsystem-xr")]
pub use engine_xr;
pub mod savegame;
pub use savegame::*;
#[cfg(feature = "subsystem-terrain")]
pub mod terrain;
#[cfg(feature = "subsystem-terrain")]
pub use terrain::{TerrainBindingStats, TerrainSystem};

use engine_asset::{AssetHandle, AssetRegistry};
use engine_renderer::{
    AssetId, DebugDrawRegistry, HashDigest, MaterialUpload, MeshUpload, MeshVertexFormat,
    RenderExtensionRegistry, Renderer, TextureUpload,
};
use engine_scene::{
    validate_scene, AssetTypeRegistry, ComponentRegistry, Scene, SceneLoadDiagnostic, World,
    WorldSlot,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[cfg(test)]
use engine_renderer::FrameStats;

pub mod ffi_init;
pub mod game_loop;
#[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
mod ragdoll_runtime;
#[cfg(feature = "subsystem-ui")]
pub use game_loop::{RuntimeUiEvent, RuntimeUiValue};

// ── Optional script subsystem ─────────────────────────────────────────────

#[cfg(feature = "subsystem-scripting-csharp")]
pub mod script;
#[cfg(feature = "subsystem-scripting-csharp")]
mod script_commands;
#[cfg(feature = "subsystem-scripting-csharp")]
mod script_components;
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
use engine_script::GameplayDamageEvent;
#[cfg(all(
    feature = "subsystem-scripting-csharp",
    feature = "subsystem-physics",
    feature = "subsystem-animation"
))]
use engine_script::GameplayRagdollEvent;
#[cfg(feature = "subsystem-scripting-csharp")]
use engine_script::{
    GameplayCameraSnapshot, GameplayCommand, GameplayContext, GameplayEntitySnapshot,
    GameplayInputTransitions, GameplayInputValue, GameplayPhysicsEvent, GameplayPointerSnapshot,
    GameplaySaveEvent, GameplayUiEvent, ScriptEngine, ScriptError, ScriptHost, ScriptTransform,
};
#[cfg(feature = "subsystem-scripting-csharp")]
use script::{collect_scene_scripts, script_engine_state_summary};
#[cfg(feature = "subsystem-scripting-csharp")]
use script_commands::animation::{
    apply_script_animation_command, apply_script_morph_weights, ScriptAnimationCommand,
};
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
use script_commands::ui::apply_script_ui_command;

#[cfg(test)]
mod tests;
