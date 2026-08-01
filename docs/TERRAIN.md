# Optional terrain component

`engine-terrain` is an optional, game-agnostic engine component. It provides
deterministic planar and cube-sphere generation, chunk scheduling,
hierarchical CDLOD, crack repair, collision payloads, surface attachment, and
construction-footprint validation. It does not contain faction, economy,
resource, weather, atmosphere, or project-specific build rules.

## Enabling it

Engine hosts enable the `engine-core/terrain` feature. The sandbox exposes the
same switch as `sandbox/terrain`; editor builds enable it so Terrain Volume can
be authored and debugged. Builds without the feature do not register the
component or start terrain worker threads.

The registered component ID is `engine.terrain_volume` (`TerrainVolume` in
Rust). Its registry metadata is `ReadWrite`, so the generic component Script
API can inspect and update the same schema used by scene serialization.

## Runtime boundary

Terrain chunks use
`TerrainChunkId { volume_id, face, x, z, lod }`; planar chunks use the `Planar`
face while a planet uses six independent cube faces. Persistent and runtime
terrain owners occupy separate identity domains, so two planets can stream the
same face/quadtree coordinates without sharing cache, material, collision, or
retirement state. They are not world partition cells: changing cell size does
not change terrain density or LOD, and unloading a world cell does not
implicitly cancel a terrain chunk.

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
material, increasing LOD distance cutoffs, LOD hysteresis, horizon culling,
and continuous geomorph controls. Values are data, not engine policy; games
own the authored parameter sets and seed allocation rules.

## Cube-sphere planet contract

Set `topology` to `CubeSphere` and author `planet_center`, `planet_radius`, and
`planet_max_lod`. `lod_distances` must then contain exactly
`planet_max_lod + 1` entries. Each of the six faces starts as one coarse root
patch and independently refines around the full f64 logical-space focus. A
conservative inner-sphere horizon test rejects back-side nodes before
generation. The test expands the visible cap by the patch angular radius and
maximum authored displacement, so elevated limb silhouettes remain resident.
`horizon_culling = false` is available for diagnostics.

Vertices are projected from a cube face onto the sphere and displaced by one
shared 3D noise field, so adjacent faces sample the same position at their
seam. Render skirts point toward the planet center and are excluded from
collision. Curved chunks use exact static triangle-mesh colliders; planar
chunks retain the cheaper heightfield collider. Floating-origin conversion
happens only when a generated logical-space patch is committed to ECS, so
orbit-to-surface travel does not lose chunk identity.

Fine patches encode the signed radial displacement to the exact parent
triangle surface. `geomorph_start_ratio` starts a smoothstep transition inside
the same distance cutoff used by CDLOD selection. The render extractor carries
that state as a generic `VertexGeomorph`; Vulkan and DX12 apply the identical
radial displacement in both forward and shadow vertex shaders. Vulkan uses two
draw push-constant vectors, while DX12 uses the matching first 32 bytes of its
64-byte `VertexDraw` header (the remaining 32 bytes carry material projection)
before its bone palette. DX12 packs one 256-byte-aligned arena record per
drawable and reuses that offset across shadow and all forward views instead of
allocating a committed constant-buffer resource per patch/pass. Parents still
remain active until every overlapping child commits, while the per-vertex morph
removes the visible geometric pop inside an active level.

## Continuous projected PBR materials

Terrain material coordinates are an engine rendering policy, independent of
which textures a game assigns. `material_projection` supports:

- `Automatic` (the default): world-relative triplanar for planar chunks and
  `planet_center`-relative triplanar for cube-sphere chunks;
- `MeshUv`: the legacy/generated UV path for explicitly patch-aware assets;
- `WorldTriplanar` and `PlanetTriplanar`: explicit coordinate policies.

`material_tile_size` is the world-space length of one texture repeat and
`triplanar_blend_sharpness` controls the transition between projection axes.
The engine derives a small mesh-local projection origin for every streamed
chunk. World-relative mapping retains only the repeat phase, while
planet-relative mapping first subtracts the f64 planet center and then retains
only the repeat phase in f64 before conversion. This keeps adjacent patches
continuous even for 10^12-scale centers and large radii, without sending large
logical coordinates through f32 vertex data.

Extraction transports the transient mapping through `RenderableItem`, not the
shared material uniform block, because neighbouring chunks can use the same
material with different local origins. Vulkan uses the final 32 bytes of its
128-byte static draw push block; DX12 uses the matching two `float4` values in
`VertexDraw`. Base color, tangent-space normal, metallic-roughness, occlusion,
and emissive textures all use the same three-axis weights. A flat normal map
preserves the curved geometric normal. Ordinary meshes, particles, skinned
meshes, and terrain authored as `MeshUv` retain their existing UV sampling.
Projected terrain is intentionally excluded from static instance batching
because its origin is per draw.

## Camera-altitude planetary lens

`PlanetaryLensSettings::mode` keeps authored post processing separate from
terrain policy:

- `Manual` preserves all authored distortion, curvature, atmosphere, and
  chromatic-aberration values exactly.
- `CameraAltitude` asks `engine-core` render extraction for the active camera's
  logical f64 position (origin-relative transform plus floating world origin),
  then obtains signed surface altitude from `PlanetTerrainQuery`.

Automatic mode applies a smoothstep weight between `altitude_fade_start` and
`altitude_fade_end`; it is zero at/below the start and full strength at/above
the end. The query therefore follows the same displaced surface used by terrain
generation instead of assuming sea-level radius. With multiple enabled
cube-sphere volumes, the engine selects the surface with the smallest absolute
signed distance from the camera and uses a stable typed identity to break an
exact tie deterministically. If there is no active camera or no valid
cube-sphere volume, the automatic effect fails closed for that frame. This is
render extraction behavior, not a gameplay state machine or a scene-transition
trigger.

## Native planetary query library

`PlanetTerrainQuery` owns the exact seeded fBm/domain-warp recipe used by mesh
generation. It provides height sampling, world-to-surface projection, signed
altitude, latitude/longitude conversion, terrain-aware tangent frames, and
great-circle distance in f64. Physics, navigation, and hosts should call this
library rather than porting planetary math into gameplay scripts.

The same implementation is available through `engine-ffi` and the in-process
managed `Engine.API` `PlanetTerrainQuery` owner. The API uses opaque native
handles; managed code does not duplicate the noise or spherical math.

`PlanetTerrainQuery.ResolveSurfacePlacement` also resolves a complete
terrain-aware construction basis in native code. It samples the authored
footprint, rejects excessive slope or support-height variation, and returns
position, rotation, tangent axes, angular footprint, and diagnostics in `f64`.

## Editable density terrain, caves, and incremental rebuilds

`EditableTerrain` layers a sparse signed-density field over the exact planar
or cube-sphere base sampler. `TerrainBrush` supports add, subtract, smooth and
set-density modes with bounded radius, strength, falloff and optional material
samples. Because the field is volumetric rather than a second height map,
subtractive edits can create tunnels, caves and overhangs.

Edits are partitioned by signed 64-bit density-chunk coordinates. Dirty work
is coalesced and polygonised with marching tetrahedra and shared boundary
samples, at most four 16-cubed chunks per frame. The engine expands the dirty
set to cover the overlapping heightfield/CDLOD patch. It keeps that original
patch visible and collidable until every replacement density chunk has
committed, then atomically hides the old renderable and removes its collider;
failed or oversized replacement work therefore retains valid terrain instead
of opening a transient hole. New LOD patches repeat the same coverage proof.

Density meshes use the normal runtime mesh registry. Static triangle collider
changes are detected by `PhysicsWorld::sync_from_ecs` and only the changed
Rapier collider is rebuilt; unrelated bodies retain pose, velocity, sleep and
joint state. When navigation is enabled, the
affected `DynamicNavTile` is cooked and atomically replaced; a failed bake
retains the previous tile and never invalidates unrelated chunks. Hosts that
own a different physics or navigation implementation can instead implement
`EditableTerrainRebuildSink` and consume the same bounded rebuild stream.

`TerrainEditStore` writes append-only, versioned and checksummed edit revisions
under the project save directory. A normal terrain tick automatically restores
the latest revisions for persistent volumes, invalidates all neighbouring mesh
boundaries, and drains the bounded rebuild queue across later frames. The
persisted voxel configuration must match the authored volume before edits are
applied. Editable mesh transforms are recomputed against every floating-origin
shift. Managed gameplay calls
`Terrain.ApplyBrush(terrainEntityId, WorldPosition64, ...)` and observes the
asynchronous result with `Terrain.TryGetResult`; density sampling, chunk math,
meshing, persistence, collision and navigation rebuild remain native engine
responsibilities.

## Surface attachment and construction occupancy

`engine.planet_surface_anchor` persists a planet-relative direction, heading,
altitude offset, footprint radius, support limits, and whether the footprint
blocks navigation. During the terrain tick the engine resolves root transforms
from the same query used by mesh generation. The logical position stays `f64`
until conversion against the current floating origin, so anchored buildings
survive origin shifts and terrain regeneration without script-side projection
math.

`PlanetSurfaceOccupancy` stores stable-ID geodesic caps. Reservation and
overlap checks work across cube-face and longitude seams, serialize
deterministically, and expose only navigation-blocking footprints to the
spherical-navigation bridge. Economy, ownership, build costs, and which model
to instantiate remain project data.

An anchor's `terrain_volume_id` binds it to a persistent planet. The legacy
empty value resolves only when exactly one valid cube-sphere volume exists;
multi-planet ambiguity fails closed. Occupancy keys combine typed planet and
owner identities, keeping persistent and runtime entities in separate domains.
The binary format uses a strict magic/version envelope with an explicit legacy
decoder, while human-readable scene data remains backward compatible.

## Optional altitude scene policy

Continuous cube-sphere streaming does not require a landing scene switch.
Projects that intentionally separate orbit and surface simulation can use
the authorable `engine.planet_scene_transition` component. Its orbit/surface
scene IDs, enter/exit altitudes, and minimum dwell time feed
`PlanetSceneTransitionController`; the gap between enter and exit is the
hysteresis band. An absent component, or one with `enabled = false`, never
switches scenes.

`terrain_volume_id` is the persistent entity ID of the target
`engine.terrain_volume`. Multi-planet scenes must set it explicitly. An empty
ID is retained only for compatibility and resolves when exactly one enabled,
valid `CubeSphere` volume exists; zero or multiple candidates fail closed with
a recoverable diagnostic.

`GameLoop` samples the active camera in logical f64 space and emits a
host-facing transition ticket with a runtime-global transaction identity.
Acknowledgements must present that complete ticket, so a delayed result cannot
commit a newer request after a controller rebuild. `ProjectApp` loads and
validates the requested catalog scene transactionally, then commits the ticket.
Unknown, rejected, or failed loads retain/restore the current scene, reject the
ticket, and can retry on a later frame. If restoring the previous scene also
fails, the host receives a fatal error instead of a false retained-scene
success. A committed surface policy remains in the engine across the scene
replacement so climbing above the exit altitude can load the orbit scene even
when the surface scene does not duplicate the component. This policy and its
acknowledgement state are engine-owned and are not exposed through the generic
gameplay-script component bridge.

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

This implementation covers ENG-T0 and ENG-13: planar and cube-sphere terrain,
f64 streaming, horizon rejection, continuous geomorph, editable volumetric
density chunks, cave/overhang meshing, persistent brush edits, collision and
navigation tile rebuilds, surface attachment/occupancy, and shared native
planetary queries alongside ENG-11, ENG-14, ENG-15, and ENG-70. The renderer
also exposes a generic altitude-driven planetary-lens post effect. Volumetric
atmosphere, ocean simulation, and project-specific biome/resource placement
remain separate optional systems.
