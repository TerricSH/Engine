#[cfg(feature = "subsystem-scripting-csharp")]
mod commands;
#[cfg(feature = "subsystem-scripting-csharp")]
mod context;
#[cfg(feature = "subsystem-scripting-csharp")]
mod extended_commands;
#[cfg(feature = "subsystem-scripting-csharp")]
mod lifecycle;
#[cfg(feature = "subsystem-scripting-csharp")]
mod queries;
#[cfg(feature = "subsystem-scripting-csharp")]
mod state;
#[cfg(feature = "subsystem-scripting-csharp")]
mod world;

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) use state::ScriptRuntimeState;
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-physics"))]
pub(crate) use world::destroy_script_entity;
#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) use world::script_component_diagnostic;
#[cfg(all(feature = "subsystem-scripting-csharp", test))]
pub(crate) use world::validate_script_transform;

use crate::{EngineRuntime, SceneLoadRequest};

impl EngineRuntime {
    /// Take the next deferred script scene-load request, if scripting is
    /// enabled and a script emitted one during OnCreate/OnUpdate.
    pub fn take_pending_scene_request(&mut self) -> Option<SceneLoadRequest> {
        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.scripting.pending_scene_request.take()
        }
        #[cfg(not(feature = "subsystem-scripting-csharp"))]
        {
            None
        }
    }
}
