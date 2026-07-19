//! Optional, game-agnostic terrain generation and streaming primitives.
//!
//! `engine-terrain` deliberately knows nothing about planets, biomes, or
//! gameplay. A [`TerrainVolume`] is a parameter block, [`HeightfieldGenerator`]
//! converts a requested chunk into CPU mesh/collision data, and
//! [`TerrainRuntime`] schedules that work independently from world cells.
//! Hosts decide how to upload meshes, create colliders, and choose parameters.

#![forbid(unsafe_code)]

mod chunk;
mod component;
mod generator;
mod lod;
mod runtime;

pub use chunk::*;
pub use component::{register_terrain_extensions, TerrainVolume};
pub use generator::HeightfieldGenerator;
pub use lod::{chunk_span, desired_chunks, desired_chunks_hysteretic, TerrainLodConfig};
pub use runtime::{
    TerrainChunkGenerator, TerrainDebugSnapshot, TerrainRuntime, TerrainRuntimeConfig,
    TerrainRuntimeEvent, TerrainRuntimeStats,
};
