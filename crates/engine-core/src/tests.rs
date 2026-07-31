//! Tests for the engine-core composition root.
//!
//! This module remains a child of the crate root, so private implementation
//! details stay testable without burying the public facade.

use super::*;

include!("tests/support.rs");
include!("tests/runtime_registration.rs");
include!("tests/frame_rendering.rs");
include!("tests/planetary_lens.rs");
include!("tests/runtime_ui_assets.rs");
include!("tests/world_registry.rs");
include!("tests/scripting_lifecycle.rs");
include!("tests/scripting_prefabs.rs");
include!("tests/scripting_ui.rs");
include!("tests/scripting_scene.rs");
