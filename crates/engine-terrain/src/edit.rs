//! Sparse editable density terrain.
//!
//! Heightfield terrain remains the cheapest representation for untouched
//! planetary surfaces. This module supplies the volumetric layer required by
//! caves, tunnels and runtime excavation: edits are stored in sparse 3-D
//! chunks, meshed independently and persisted as append-only chunk revisions.

mod density;
mod mesh;
mod rebuild;
mod store;

pub use density::*;
pub use mesh::*;
pub use rebuild::*;
pub use store::*;
