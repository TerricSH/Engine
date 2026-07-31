//! Optional, game-agnostic terrain generation and streaming primitives.
//!
//! A [`TerrainVolume`] can describe a planar heightfield or a streamed
//! cube-sphere planet. [`HeightfieldGenerator`] converts a requested chunk
//! into CPU mesh/collision data, and
//! [`TerrainRuntime`] schedules that work independently from world cells.
//! Hosts decide how to upload meshes, create colliders, and choose parameters.

#![forbid(unsafe_code)]

mod chunk;
mod component;
mod generator;
mod lod;
mod placement;
mod planet;
mod runtime;
mod transition;
mod transition_component;

pub use chunk::*;
pub use component::{
    register_terrain_extensions, serialize_terrain_fields, TerrainMaterialProjection,
    TerrainTopology, TerrainVolume,
};
pub use generator::HeightfieldGenerator;
pub use lod::{
    chunk_span, desired_chunks, desired_chunks_for_volume, desired_chunks_for_volume_hysteretic,
    desired_chunks_hysteretic, desired_terrain_chunks, desired_terrain_chunks_for_volume,
    desired_terrain_chunks_for_volume_hysteretic, desired_terrain_chunks_hysteretic,
    planet_chunk_visible_from, terrain_chunk_bounds, terrain_chunk_distance, TerrainLodConfig,
};
pub use placement::{
    serialize_planet_surface_anchor_fields, PlanetConstructionFootprint, PlanetPlacementError,
    PlanetSurfaceAnchor, PlanetSurfaceOccupancy, PlanetSurfaceOwnerKey, PlanetSurfacePlacement,
    PlanetSurfaceReservation, PlanetSurfaceVolumeKey,
};
pub use planet::{PlanetCoordinates, PlanetTangentFrame, PlanetTerrainQuery};
pub use runtime::{
    TerrainChunkGenerator, TerrainDebugSnapshot, TerrainRuntime, TerrainRuntimeConfig,
    TerrainRuntimeEvent, TerrainRuntimeStats,
};
pub use transition::{
    PlanetSceneBand, PlanetSceneTransitionConfig, PlanetSceneTransitionController,
    PlanetSceneTransitionError, PlanetSceneTransitionRequest,
};
pub use transition_component::serialize_planet_scene_transition_fields;
