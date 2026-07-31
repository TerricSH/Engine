//! DAG-based configurable render graph (Gate 4).
//!
//! Provides an extensible `PassKind`, rich `PassNode` with resource
//! declarations, and a `RenderGraph` builder that supports topological
//! sorting. This is the single render-graph contract used by the renderer and
//! every backend.
//!
//! The canonical 4-pass ordering is produced by `RenderGraph::build()`:
//!   directional_shadow_pass → opaque_pbr_forward_pass → tone_map_pass → present
//!
//! Custom orderings can be expressed via `build_with_config()` which
//! honours `PassGraphConfig` (loadable from scene settings).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::{RenderFrameInput, RenderView, ViewCompose};

mod graph;
mod passes;
mod transient;

pub use graph::*;
pub use passes::*;
pub use transient::*;

#[cfg(test)]
#[path = "render_graph2/tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("render_graph2.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in ["graph", "passes", "transient"] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
