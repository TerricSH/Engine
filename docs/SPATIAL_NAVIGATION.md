# Spatial and spherical navigation

`engine-nav` contains three independent, game-agnostic navigation models:

- `NavMesh` and `Pathfinder` for conventional locally planar walkable areas;
- `SpaceNavGrid` for unrestricted three-axis movement through sparse voxel
  occupancy;
- `SphericalNavGraph` for seam-free routes over a complete planetary surface.

No model contains faction, mission, vehicle, landing, or construction rules.
Those systems select endpoints and consume the returned waypoints.

## Three-dimensional space grid

`SpaceNavGrid` uses bounded integer `SpaceCell` addresses and stores only
blocked cells. Its A* search supports six axial or all 26 neighbors, prevents a
diagonal step from cutting through a blocked axial neighbor, enforces a
configurable expansion budget, and greedily removes waypoints with clear voxel
line of sight. Start and goal points remain exact world positions.

Use this model for spacecraft, drones, zero-gravity agents, underwater motion,
or any volume where a planar X/Z heuristic is incorrect.

## Spherical surface graph

`SphericalNavGraph::fibonacci_sampled` distributes candidates without cube-face
seams. The caller-provided sampler can:

- project a direction through `PlanetTerrainQuery` onto the exact generated
  surface;
- return a positive traversal cost;
- return `None` for oceans, cliffs, holes, construction footprints, or other
  non-traversable samples.

Edges are density-bounded angular neighbors and are made symmetric. Positions,
planet centres, arc lengths, and costs remain `f64`. A* uses great-circle arc
length as its heuristic, so routes remain valid across poles, former cube-face
boundaries, and large floating-world coordinates.

Dynamic geodesic caps can be inserted, replaced, or removed without rebuilding
the sampled graph. An obstacle blocks covered nodes/endpoints; a traversal area
multiplies cost while keeping the great-circle heuristic admissible. Producer
layers are replaced atomically, so construction footprints can be refreshed
without deleting weather, hazard, or project-owned obstacles.

`SphericalNavigationRuntime` adds deterministic runtime agents. It retains
paths between frames, invalidates them when the graph's dynamic revision
changes, caps A* replans per tick, and follows each segment by spherical
interpolation instead of cutting a chord through the planet. Agent positions
remain `f64`.

## Native and managed access

`engine-ffi` exposes opaque handles for a mutable `SpaceNavGrid`, uniform or
terrain-projected `SphericalNavGraph`, and their immutable path results. The in-process
`Engine.API` wrappers are `SpaceNavigationGrid` and
`SphericalSurfaceNavigation`; they copy native waypoints into managed
`NavigationPath` values and release native path allocations immediately.

`PlanetTerrainQuery.CreateSurfaceNavigation` constructs the spherical graph
through the native query handle. Its nodes therefore use the same seeded
surface function as the terrain mesh; managed code does not duplicate height,
projection, or tangent mathematics.

Out-of-process ProcessHost scripts must use the engine IPC protocol rather than
direct P/Invoke. Native Rust systems can use the richer sampled spherical
builder and runtime directly, including the `PlanetTerrainQuery` projection
closure.

With both terrain and navigation enabled,
`GameLoop::sync_planet_construction_navigation` atomically publishes all
navigation-blocking `PlanetSurfaceAnchor` footprints into a dedicated graph
layer. A `SphericalNavigationRuntime` then observes the revision and spreads
affected replans across its configured frame budget.

The default bridge binds a graph to the cube-sphere whose centre is nearest to
the graph centre. An exact or near-equal match is rejected as
`AmbiguousSurfaceBinding`; callers then use the explicit per-volume API. Typed
planet and owner scopes keep anonymous runtime entities isolated from authored
persistent IDs on every planet.

## Performance contract

Grid searches are bounded by `SpaceNavConfig::max_expansions`. Spherical graph
construction is intentionally an offline or infrequent operation; path queries
reuse its adjacency. Dynamic updates rebuild only the graph's lightweight
blocked-node overlay. Agent replanning is explicitly budgeted, so a large
construction batch cannot force every path query into one frame.
