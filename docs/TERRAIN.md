# Optional terrain component

`engine-terrain` is an optional, game-agnostic engine component. It provides
deterministic heightfield generation, chunk scheduling, hierarchical CDLOD, crack
repair, and collision payloads. It does not contain biome, planet, resource,
weather, placement, or gameplay rules.

## Enabling it

Engine hosts enable the `engine-core/terrain` feature. The sandbox exposes the
same switch as `sandbox/terrain`; editor builds enable it so Terrain Volume can
be authored and debugged. Builds without the feature do not register the
component or start terrain worker threads.

The registered component ID is `engine.terrain_volume` (`TerrainVolume` in
Rust). Its registry metadata is `ReadWrite`, so the generic component Script
API can inspect and update the same schema used by scene serialization.

## Runtime boundary

Terrain chunks use `TerrainChunkId { x, z, lod }`. They are not world
partition cells: changing cell size does not change terrain density or LOD,
and unloading a world cell does not implicitly cancel a terrain chunk.

`TerrainRuntime` owns the reusable scheduling state machine:

- a priority queue and bounded in-flight count;
- background worker generation;
- revision checks that discard stale hot-reload output;
- explicit cancellation/unload events;
- a byte budget for main-thread commits and a resident CPU-payload cache budget;
- queue, timing, memory, cancellation, stale-result, and eviction counters.

The `engine-core` adapter consumes ready events at the frame boundary using a
two-phase commit: generated CPU data becomes resident only after both its
runtime mesh and ECS/physics binding exist. A failed host commit remains
explicitly failed until retry, and an older binding stays live until every
overlapping replacement is committed. This prevents holes during asynchronous
parent/child and revision transitions. Meshes use the same ENG-20 path as
cooked meshes. Evicting retained CPU payload never unloads the live render or
physics resource and never starts a regeneration loop.

Selection uses the active camera plus the f64 floating world origin. Chunk
coordinates are signed 64-bit values and procedural sampling keeps f64 logical
coordinates until deterministic fixed-point conversion, avoiding far-world
f32 identity collapse. Existing
terrain entities participate in normal origin shifting, while newly committed
chunks are converted from logical coordinates back into current origin-relative
coordinates.

## Heightfield and LOD contract

`base_resolution` must be `2^n + 1` in `3..=513`. Every chunk samples noise at
logical world coordinates, so equal-level neighbour borders have bit-identical
heights. LOD 0 covers `chunk_size`; each coarser quadtree level doubles the
physical span on both axes while retaining fixed patch topology. The selected
leaves never overlap. Authored `lod_hysteresis` supplies a world-space dead band
for split/merge stability, and a generated vertical skirt on all four edges
hides T-junctions between adjacent levels.

The height recipe exposes scale, height amplitude, octave count, frequency,
lacunarity, gain, domain-warp amplitude/frequency, skirt depth, collision,
material, increasing LOD distance cutoffs, and LOD hysteresis. Values are data, not engine
policy; games own the authored parameter sets and seed allocation rules.

## Debugging

Open **Analysis / Terrain** in the editor. The panel shows resident/queued/
generating/failed chunks, cache and commit bytes, latest generation time,
stale-result drops, and evictions. It supports exact decimal-u64 seed replay,
forced regeneration, failed-work retry, and hot editing of the primary terrain
parameters, including LOD hysteresis. LOD distance arrays remain editable
through the generic Inspector.

The same data is available headlessly from
`GameLoop::terrain_debug_snapshot()`. `terrain_stream` also appears as a CPU
stage in ENG-04 frame timing.

## Scope

This implementation covers ENG-T0, the planar heightfield part of ENG-11,
ENG-14, ENG-15, and ENG-70. The spherical mapping variant (ENG-12) and editable
voxel/marching-cubes layer (ENG-13) remain deliberately separate optional
work; no planetary shape or digging rules are hidden in this component.
