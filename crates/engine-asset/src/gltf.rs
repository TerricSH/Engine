//! Strict glTF 2.0 scene importer.
//!
//! The importer preserves document indices, expands every mesh primitive into
//! an explicit [`GltfPrimitive`], resolves the selected scene's complete world
//! transforms, and preserves `JOINTS_0` / `WEIGHTS_0` data for skinned meshes.
//! Triangle lists are the only supported topology. Position and normal morph
//! deltas are preserved per primitive for the renderer's bounded target-set
//! path.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use glam::{Mat4, Vec2, Vec3};
use thiserror::Error;

use crate::mesh::MeshData;

const MAX_GLTF_MORPH_TARGETS: usize = 8;

mod animation;
mod image;
mod load;
mod material;
mod mesh;
mod model;
mod node;

pub use load::load_gltf_scene;
pub use model::*;

#[cfg(test)]
use animation::{bake_quaternion_animation_track, bake_vec3_animation_track};
#[cfg(test)]
use node::generate_vertex_normals;

#[cfg(test)]
#[path = "gltf/tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("gltf.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in [
        "animation",
        "image",
        "load",
        "material",
        "mesh",
        "model",
        "node",
    ] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
