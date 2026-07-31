use engine_scene::{
    validate_scene_for_authoring, ComponentRecord, ComponentRegistry, EntityRecord, Scene,
    SceneSettings,
};
use engine_serialize::{ComponentTypeId, PersistentId, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::EditorError;

mod components;
mod core;
mod entity_clipboard;
mod hierarchy;

pub use components::*;
pub use core::*;
pub use entity_clipboard::*;
pub use hierarchy::*;

#[cfg(test)]
use hierarchy::sibling_ids;

#[cfg(test)]
#[path = "commands/tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("commands.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in ["components", "core", "entity_clipboard", "hierarchy"] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
