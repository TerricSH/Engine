mod assets;
mod builder;
mod rendering;
mod scripting;
mod state;

pub use builder::{EngineConfig, EngineRuntimeBuilder, SceneLoadRequest};
pub use state::EngineRuntime;

pub(crate) use assets::{
    install_builtin_render_assets, missing_registered_render_asset, scene_load_diagnostic,
    validate_registered_asset_id,
};
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
pub(crate) use scripting::destroy_script_entity;
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) use scripting::script_component_diagnostic;
#[cfg(all(feature = "subsystem-scripting-csharp", test))]
pub(crate) use scripting::validate_script_transform;
pub(crate) use state::SyncedRenderResources;
