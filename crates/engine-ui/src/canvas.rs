use std::collections::{BTreeMap, BTreeSet};

use engine_renderer::{Rect, UiBatch};
use engine_serialize::{AssetId, Value};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::batch;
use crate::color::Color;
use crate::input::UiInputState;
use crate::layout::{Layout, ScaleMode};
use crate::types::{ElementId, UiElement, UiElementKind, UiRect};
use crate::DEFAULT_UI_MATERIAL;

mod batching;
mod component;
mod serialization;
mod state;

pub use component::register_ui_extensions;
pub(crate) use serialization::serialize_canvas;
pub use state::Canvas;

#[cfg(test)]
use serialization::{deserialize_canvas, encode_element};

#[cfg(test)]
#[path = "canvas/tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("canvas.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in ["batching", "component", "serialization", "state"] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
