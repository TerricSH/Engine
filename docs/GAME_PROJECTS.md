# Game projects

Every authoring and runtime command is rooted by `game.project.json`. Its
containing directory is the game-project workspace. Paths in the manifest are
relative to that directory and may not be absolute or contain `..` traversal.

The project workspace is independent of the engine installation. An installed
editor may open a workspace on another drive or in another repository; game
source, imported assets, generated project outputs, and exported releases are
not written into the engine installation.

## Layout

```text
MyGame/
  game.project.json
  world.partition.json       # optional world partition cells
  assets/
    scenes/main.scene.ron
    scenes/level-two.scene.ron
    source/game.manifest
  config/input.actions.json
  scripts/GameScripts/       # optional C# authoring source
  build/                     # generated, project-local, ignored by Git
    script-sdk/              # optional copied/built EngineGameplay.dll
    script-sdk-source/       # source-development mode only
    cooked/
    scripts/                 # optional game assembly plus managed dependencies
    script-host/             # optional copied/built .NET protocol host
  Dist/                      # installed-editor Windows package output
```

`project new` creates the core layout and `main.scene.ron` as an immediately
usable basic scene with a positioned Main Camera, a visible Cube, and a
Directional Light; additional scene files appear after `project scene new`. It
also creates an empty source manifest. `build/` and `Dist/` are project-local
outputs rather than engine-installation directories. `build/` is generated and
ignored by Git. `Dist/` is created when a project is packaged and is also
ignored by the generated project `.gitignore`.

## Installed editor and workspace deployment

An installed Windows editor discovers `engine.installation.json` by walking up
from its executable (or from the explicit `ENGINE_INSTALL_ROOT` override).
Before installed tools are used, the installation manifest, required paths,
and recorded SHA-256 hashes are validated.

When a valid scripted project is opened, the editor copies the installation's
precompiled managed tools into the external project workspace:

```text
<engine-installation>/sdk/EngineGameplay.dll
  -> <project>/build/script-sdk/EngineGameplay.dll

<engine-installation>/sdk/script-host/*
  -> <project>/build/script-host/*
```

The host directory is replaced transactionally when its contents differ and is
reused when already identical, which also avoids replacing a running Windows
host unnecessarily. During project opening, this managed-runtime deployment
does not execute MSBuild or compile game-authored C# (the editor may still cook
project assets as part of opening). Rebuild, Play, Build,
`project build-scripts`, and `project build` are the explicit script-build
paths.

Installed script builds validate the project's generated API contract, copy
the matching SDK and host from the installation, then invoke .NET only for the
game project. They do not compile the engine SDK/host from source and do not
search for an engine repository. A `.NET 8` SDK is therefore required for
authoring a C# project, but Cargo, Rust, and Git are not. Non-scripted projects
do not require .NET.

`build/script-sdk-source/` is an engine-owned source-development cache. An
installed engine does not create or consume it; a cache left by an earlier
source-development run may remain under `build/`, but it is ignored by the
installed build path. It is not game-authored source.

## Manifest

```json
{
  "schema": "GameProject-v0",
  "name": "My Game",
  "startup_scene": "main",
  "scenes": {
    "main": "assets/scenes/main.scene.ron",
    "level_two": "assets/scenes/level-two.scene.ron"
  },
  "asset_source": "assets/source",
  "cooked_assets": "build/cooked",
  "backend": "vulkan",
  "window": {
    "title": "My Game",
    "width": 1280,
    "height": 720
  },
  "world_streaming": {
    "enabled": true,
    "enter_percent": 100,
    "exit_percent": 115,
    "max_merges_per_frame": 1,
    "max_unloads_per_frame": 4,
    "seamless_planetary": true
  }
}
```

`scenes` is the project scene catalog: each key is the stable ID used by the
CLI and scripts, and each value is a project-relative `.scene.ron` path.
`startup_scene` may name a catalog ID as shown above. For compatibility, it may
also contain the path of one cataloged scene. A legacy manifest without
`scenes` is treated as a single-scene project with the synthesized ID `main`;
the first `project scene new` or `project scene set-startup` command writes an
explicit catalog without changing that ID.

Scene IDs contain 1 to 128 ASCII letters, digits, hyphens, underscores, or
dots, except `.` and `..`. IDs are case-insensitively unique. Scene paths must
remain inside the project, end in `.scene.ron`, and be unique under portable,
case-insensitive comparison.

Authoring commands require `asset_source` and an optional `script_project` to exist. A scripted project also declares `script_assembly`; authoring builds produce that DLL while runtime packages omit the `.csproj` and keep only the DLL. `input_actions`, when present, is required in both authoring and packaged projects.

## World partition

A project may add an optional `world.partition.json` at its root (next to
`game.project.json`) to describe world partition cells — the foundation for
scene-based world streaming:

```json
{
  "schema": "WorldPartition-v0",
  "cells": {
    "cell_forest": {
      "scene": "forest",
      "bounds": { "center": [0.0, 0.0, 0.0], "half_extents": [64.0, 16.0, 64.0] }
    },
    "cell_town": {
      "scene": "town",
      "bounds": { "center": [128.0, 0.0, 0.0], "half_extents": [32.0, 8.0, 32.0] }
    },
    "planet_surface_north": {
      "scene": "planet_surface_north",
      "bounds": { "center": [0.0, 0.0, 0.0], "half_extents": [0.0, 0.0, 0.0] },
      "planetary_bounds": {
        "planet_center": [1000000000000.0, 0.0, 0.0],
        "direction": [0.0, 1.0, 0.0],
        "angular_radius": 0.35,
        "min_altitude": -1000.0,
        "max_altitude": 50000.0,
        "planet_radius": 6000000.0
      }
    }
  }
}
```

Each key of `cells` is a stable cell ID following the same identifier rules as
scene IDs (1–128 ASCII letters, digits, hyphens, underscores, or dots, except
`.` and `..`, case-insensitively unique). `scene` must reference an ID from the
project scene catalog. `bounds` is an axis-aligned box: `center` and
`half_extents` must be finite, and half extents must be non-negative. Cells
may also define `planetary_bounds`; when present, runtime selection uses its
f64 planet centre, unit surface direction, angular cap and altitude band in
place of the AABB. This keeps cell identity stable at interplanetary coordinate
scales.
Cells **may overlap** — overlapping bounds are a legitimate way to layer content,
such as a dense gameplay cell over a large background cell.

`project check` validates the partition file when it is present (schema, cell
IDs, scene references, and bounds) and reports the cell count as
`partition_cells` in its JSON report. It also enforces the streaming
compatibility rules the runtime driver relies on: persistent entity IDs must
be unique across cells, and a cell that does not reference the startup scene
must not share entity IDs with it (a cell that intentionally reuses the
startup scene's content must reference the startup scene itself — the driver
then adopts the already-live entities instead of merging duplicates).
`engine.script` metadata is supported in cells: managed instances attach after
the ECS merge and receive `OnDestroy` before non-resident cell entities unload.
Scripts on entities promoted to the resident set remain attached.

### Runtime cell streaming

Set `world_streaming.enabled` in `game.project.json` to activate additive cell
streaming for ordinary headless and windowed players. A
`world.partition.json` is then required. `--stream-cells` remains an explicit
development override for legacy projects. `enter_percent`, `exit_percent`,
`max_merges_per_frame`, and `max_unloads_per_frame` configure hysteresis and
frame-boundary work budgets. With `seamless_planetary = true`, the host keeps
orbit, atmosphere, and surface content in one additive world and disables the
legacy altitude-triggered scene-replacement policy.

The `project check` report includes a structured `world_streaming` object with
the project switch, seamless mode, partition-manifest presence, cell count,
hysteresis percentages, and frame budgets. Enabling streaming without a
partition manifest is a validation error instead of being reported as the
ambiguous legacy value `partition_cells: 0`.

Each frame, at the existing frame boundary (after `update()` and scene
transition processing, before `render()`), the streaming driver resolves the
active camera's world position and computes the desired cell set with
hysteresis: an unloaded cell becomes desired at `enter_percent`, while an
already loading or loaded cell stays desired until `exit_percent`. The exit
margin prevents boundary ping-ponging. Selection consumes the camera's f64
logical position; planetary cells test altitude and angular distance without
round-tripping the planet centre through f32.

Every cell moves through a small state machine: `Unloaded → LoadingAssets →
Merging → Loaded`, and `Loaded → Unloading → Unloaded`. A cell's cooked asset
dependencies are decoded on the Phase-3 background asset stream
(`enqueue_cooked_asset_stream` / `drain_cooked_asset_stream`); the scene merge
is committed only after every dependency is installed. Merges and unloads
commit under per-frame budgets (1 merge, 4 unloads per frame boundary), and
unloads commit before merges so departing content frees IDs first. Queued
commits are cancelled when the camera changes the desired set in time — a
pending unload is dropped if its cell becomes desired again, and a pending
merge is dropped if its cell falls out of range. A cell whose cooked
artifacts are missing (run `sandbox project cook`) or whose merge fails
enters a terminal `Failed` state, surfaces a `CELL_STREAM` diagnostic, and
never retries until the next scene load.

Residency rules protect content that no longer belongs to its authoring cell:

- **Runtime-created entities never unload.** A persistent entity that appears
  in the world without being merged by the driver (for example
  `Scene.CreateEntity` / `Scene.Spawn` output) becomes resident automatically.
- **Entities that leave their authoring cell never unload.** A merged cell
  entity whose world position moves outside its cell bounds (exit factor
  applied) becomes resident; on unload it is detached from its cell hierarchy
  instead of destroyed, and a later re-merge of the cell skips its old record
  instead of duplicating it.

Physics follows the world incrementally: after any frame boundary that
committed merges or unloads, the player calls
`GameLoop::resync_physics_from_world()`, which runs
`PhysicsWorld::sync_from_ecs` — newly merged entities gain bodies, unloaded
entities lose theirs, and every untouched body keeps its exact simulation
state (no physics world rebuild).

A scene transition (`Scene.Load`) replaces the whole world; the driver then
rebaselines — the resident set clears, failed cells reset, and cells stream
from scratch around the new scene. A cell whose entire entity set is already
live after a load (typically one referencing the startup scene) is adopted as
`Loaded` without merging. Headless run reports expose the final streaming
state under `cell_streaming` (loaded cells, total merges/unloads, resident
count, per-cell states).

Authoring notes and current limitations:

- Keep the active camera in the startup scene. Camera entities in cell scenes
  still merge and become ordinary additional cameras (priority ordering
  picks the active one), so cells should not author cameras.
- Shared cooked assets are never unloaded; there is no per-cell asset
  reference counting yet (follow-up work).
- Once a cell's asset stream is enqueued it is never cancelled; leaving the
  cell's bounds before the merge only skips the merge, and the committed
  assets stay installed.
- Checkpoints capture the whole currently live ECS world. Streaming-driver
  residency, desired-cell, and in-flight asset-request state is not serialized;
  after restore the driver rebaselines from the restored world and focus.
- The editor is unaffected: streaming is a player-runtime behaviour and the
  editor continues to open whole scenes.

## Commands

```powershell
sandbox project new <directory> [--name NAME] [--with-csharp]
sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|audio|animation|skeleton|navmesh|prefab] [--separate-primitives] [--no-bake-node-transforms]
sandbox project check <project> [--report PATH]
sandbox project scene list <project>
sandbox project scene new <project> <scene-id> [--name NAME]
sandbox project scene set-startup <project> <scene-id>
sandbox project cook <project>
sandbox project sync-script-api <project>
sandbox project build-scripts <project>
sandbox project build <project>
sandbox project run <project> [--headless] [--frames N] [--report PATH] [--stream-cells]
sandbox project editor <project>
```

`sandbox game` and `sandbox editor` are short aliases for the last two
commands. A project path may be either the project directory or its manifest.
In an installation, substitute the absolute
`<engine-installation>\bin\EngineEditor.exe` path for `sandbox`; source-tree
developers commonly use `cargo run -p sandbox --`.

`project import` copies a supported source into the configured `asset_source`, adds it to `game.manifest`, cooks the project in an isolated staging directory, and installs validated cooked artifacts. A glTF/GLB import merges all primitives into `<asset-id>` and bakes selected-scene node world transforms into static vertices by default, preserving complete model geometry instead of silently selecting one primitive. Skinned sources automatically keep node transforms external; use `--separate-primitives` for the former `<asset-id>`, `<asset-id>.mesh.1`, ... layout, `--no-bake-node-transforms` to opt a static model out, or `--bake-node-transforms` to require baking explicitly. Mirrored transforms reverse triangle winding; normals use inverse-transpose transformation. The same import generates `<asset-id>.material.<index>.material.json` and normalized `<asset-id>.texture.<index>.png` sources, registers them as Material and Texture manifest entries, and cooks them through the normal project pipeline. Material conversion preserves metallic-roughness/emissive factors, base-color/normal/metallic-roughness/occlusion/emissive texture slots, alpha mode/cutoff, occlusion strength, and double-sided state. A manifest-authored Mesh entry can request the same behavior with `gltf_merge_primitives: true` and `gltf_bake_node_transforms: true`; `gltf_primitive_index` remains available and is mutually exclusive with merging.

glTF import also preserves normalized `JOINTS_0`/`WEIGHTS_0`, remaps joint indices into parent-before-child order, imports inverse-bind matrices and LINEAR/STEP/CUBICSPLINE translation/rotation/scale channels, and selects the skinned GPU vertex layout automatically. STEP tracks are converted to held linear keys; CUBICSPLINE tracks are deterministically baked at 60 Hz (with a bounded per-segment sample count), including component-wise quaternion Hermite evaluation followed by normalization. Each skin produces `<asset-id>.skeleton.<skin-index>` and its matching clips produce `<asset-id>.animation.<skin-index>.<clip-index>`. Transform baking is rejected for skinned primitives because changing bind space would corrupt animation. Merged morph-target primitives are also rejected with an instruction to use separate primitives. Relative external buffer and image dependencies are copied without allowing paths to escape the source directory; decoded images, including GLB buffer-view/data-URI images, are additionally emitted as deterministic PNG texture sources. Generated `.skel`, `.anim`, material, and texture sources keep subsequent `project cook` deterministic. Animation compression, retargeting, and event tracks remain unsupported. Textures accept supported image formats such as PNG, JPEG, and PPM. `MaterialSource-v0` details are documented in [`MATERIAL_SURFACES.md`](MATERIAL_SURFACES.md). Runtime subsystem assets also accept standalone WAV/MP3/OGG/FLAC audio, bincode `.anim` animation clips, bincode `.skel` skeletons, and bincode `.navmesh`/`.nav` navigation meshes. Prefab imports accept the canonical `Prefab-v0.1.0` RON source (`*.prefab.ron`), which is also the extension inferred when `--type` is omitted. Type inference is used only for unambiguous extensions; `--type material` can identify a plain `.json` material. Asset IDs are portable and case-insensitively unique. Existing source or cooked files are never overwritten, and a failed cook restores the manifest and removes every copied/generated source and cooked artifact.

The editor Asset Browser exposes **Reimport Project Assets**. It recooks the
configured source tree and refreshes the live typed asset registry without restarting
the editor. Importing a brand-new external file still uses `project import`,
which provides the explicit asset ID and transactional copy/manifest update.

`project scene list` prints the catalog and marks its startup entry.
`project scene new` creates the same basic starter scene at
`assets/scenes/<scene-id>.scene.ron` and adds it to the catalog without
overwriting an existing ID or file. `project scene set-startup` accepts only an
existing catalog ID and stores that ID in `startup_scene`.

`project check` validates the manifest and safe paths, then loads **every**
cataloged scene. For each scene it checks the scene schema, script references,
strict ECS restoration, and declared asset dependencies; it also validates the
project input map, source manifests, duplicate IDs, and source files, plus the
optional `world.partition.json` when present. Its JSON
report also records validated cooked render/extension counts. Its other fields
include `startup_scene_id`, the scene count, an entity count per scene, the
legacy top-level `partition_cells`, and a structured `world_streaming` status
that distinguishes a disabled feature from a missing or empty partition.
`game --headless` runs the normal GameLoop update/render path and fails
if the active scene produces no visible draw calls.

`--with-csharp` creates an empty .NET 8 class library and the engine SDK
integration. It does not invent a script class or attach one to the scene.
`sync-script-api` refreshes the engine-owned version/hash sidecar and MSBuild
integration after an engine upgrade; game rules belong in behaviour sources
created explicitly by the project author. In source-development mode, the
canonical `EngineGameplay.cs` is generated under `build/script-sdk-source`,
never in the game source directory. Installed mode deploys the SDK DLL instead.

In an installed engine, `build-scripts` validates the integration, deploys the
installation's matching `EngineGameplay.dll` and process host, compiles the
declared game DLL against that SDK, and runs the managed gameplay-bridge
self-test. In source-development mode it instead compiles the generated SDK
and publishes the host from engine sources before compiling the game assembly.
Both paths copy the SDK dependency beside `GameScripts.dll` for runtime
packaging. An authoring `game` launch rebuilds configured scripts; the editor
rebuilds on explicit Rebuild/Play/Build, not merely on project open. Runtime
reports expose loaded assemblies, attached/started instances, script-entity
translations, and script errors. See
[`GAME_ENGINE_BOUNDARY.md`](GAME_ENGINE_BOUNDARY.md) for the enforced ownership
and review rules.

The generated `Main` derives from `EngineBehaviour`. During `OnCreate`,
`OnStart`, and `OnUpdate`, it can read and write its owning entity's local
`Transform`, read the current resolved project input actions, queue bounded
movement intent through `Character.Move` / `Jump` / `Control`, query
persistent entities, edit another entity's Transform, destroy entities, and
request a cataloged scene by ID:

```csharp
public void OnStart()
{
    Scene.Load("level_two");
}

public void OnUpdate(float deltaTime)
{
    bool jumpHeld = Input.GetBool("jump");
    bool jumpStarted = Input.WasPressed("jump");
    bool jumpEnded = Input.WasReleased("jump");
    var position = Transform.Translation;
    Transform.Translation = new Vector3(position.X + Speed * deltaTime, position.Y, position.Z);

    // Prefer controller intent over direct Transform writes for playable or
    // AI characters that carry engine.character_controller.
    Character.Move(new Vector3(1.0f, 0.0f, 0.0f), 5.0f);
    if (Input.WasPressed("jump"))
        Character.Jump();

    var enemy = Scene.FindEntity("enemy-01");
    if (enemy?.HasTransform == true)
        enemy.Transform.Translation += new Vector3(0.0f, 0.0f, deltaTime);

    // Destruction is deferred until the frame boundary. OnDestroy runs first.
    if (Input.GetBool("remove_enemy") && enemy != null)
        enemy.Destroy();

    // Creation is committed after all callbacks. The new Transform-bearing
    // entity appears in Scene.Entities on the next frame.
    if (Input.WasPressed("fire"))
        Scene.CreateEntity($"projectile-{UpdateCount}", position);

    foreach (var physicsEvent in Physics.Events)
    {
        if (physicsEvent.Kind == "trigger_entered")
            Console.WriteLine($"trigger entered by {physicsEvent.OtherEntityId}");
    }

    // Physics queries are deferred: issue the query now and read its result
    // from the returned handle on the next frame.
    if (Input.WasPressed("fire"))
        _groundProbe = Physics.Raycast(position, new Vector3(0.0f, -1.0f, 0.0f), 10.0f);
    if (Physics.TryGetRaycastHit(_groundProbe, out var groundHit))
    {
        Console.WriteLine($"ray hit {groundHit.EntityId} at {groundHit.Point}");
        if (Input.WasPressed("use") && groundHit.Entity != null)
            Physics.ApplyImpulse(groundHit.Entity, new Vector3(0.0f, 2.0f, 12.0f));
    }

    // Sweeps and filters share the same handle model: a sphere cast for a
    // character controller, excluding the character's own colliders.
    _bodyProbe = Physics.SphereCast(
        position,
        0.4f,
        new Vector3(0.0f, -1.0f, 0.0f),
        2.0f,
        new PhysicsQueryFilter { ExcludeEntityId = EntityId });
    if (Physics.TryGetSphereCastHit(_bodyProbe, out var bodyHit))
        Console.WriteLine($"standing on {bodyHit.EntityId}, {bodyHit.Distance:F2}m below");

    if (UI.WasClicked("start-game"))
        Scene.Load("level_one");

    foreach (var uiEvent in UI.Events)
        Console.WriteLine($"UI click: {uiEvent.CanvasId}/{uiEvent.ElementId} ({uiEvent.CallbackId})");
}
```

Scene colliders can opt into the project-facing use convention with
`engine.interactable`. `Interaction.Probe` returns a deferred query and
`Interaction.TryGetTarget` exposes its prompt, action key, distance, and
grabbable flag on the next frame; see [`INTERACTION.md`](INTERACTION.md).

`Scene.Entities`, `Scene.Exists(id)`, `Scene.FindEntity(id)`, and `Scene.GetEntity(id)` operate on the current frame's persistent-entity snapshot. `Scene.CreateEntity(id)` and `Scene.CreateEntity(id, translation)` enqueue a new persistent entity with an identity Transform or the requested translation. Creation is validated and committed at the frame boundary, so the entity becomes queryable in the next frame; duplicate same-frame IDs use deterministic first-wins semantics. `Scene.DestroySelf()`, `Scene.Destroy(id)`, and `Entity.Destroy()` use the same deferred mutation boundary. Scripts never receive raw ECS handles.

`Scene.Spawn(prefabId)` and `Scene.Spawn(prefabId, translation)` instantiate a cooked prefab asset. `prefabId` is the asset id declared for a `Prefab` entry in the project's source manifest — not a file path — and it follows the same identifier rules as entity ids. Spawning is validated and committed at the frame boundary exactly like `Scene.CreateEntity`, so the new hierarchy becomes queryable through `Scene.FindEntity` on the next frame. The spawned root takes the first free persistent id from `prefabId`, `prefabId-2`, `prefabId-3`, …; every other spawned entity takes `<rootId>.<prefab-local id>` (with the same `-N` suffix on conflict), so two spawns of `prefab-enemy` produce `prefab-enemy` + `prefab-enemy.<child>` and `prefab-enemy-2` + `prefab-enemy-2.<child>`. The optional translation override replaces the prefab root's Transform translation while preserving the prefab's own rotation and scale. `engine.script` components inside the prefab attach to the spawned entities as part of the same boundary: their `OnCreate` runs immediately, and commands it enqueues (including further spawns, depth-bounded) are applied recursively before the next frame. Unknown or invalid prefab ids, non-finite translations, and unloadable prefab graphs surface as script errors; a failed spawn rolls back the whole instance.

To author a spawnable prefab, write a `Prefab-v0.1.0` RON document under the configured `asset_source` (for example `assets/source/Prefabs/enemy.prefab.ron`) or import an existing file with `project import --type prefab`, declare it in `game.manifest` with `asset_type: "Prefab"`, and cook the project. `project check` parses every declared prefab source, verifies that component fields reference only declared assets or engine builtins, requires nested `child_prefab_refs` to point at other declared prefab assets, and rejects missing children and cycles in the nested graph.

`Physics.Events` contains the owning entity's collision, trigger, and joint-break events for the current frame. Event kinds are `collision_entered`, `collision_stayed`, `collision_exited`, `trigger_entered`, `trigger_stayed`, `trigger_exited`, and `joint_broken`; `OtherEntityId` and `Other` identify the other persistent scene entity. A joint-break event also exposes `JointId`, `Force`, and `Torque`. The native physics queue is drained every frame even when no script consumes it, so events cannot accumulate indefinitely.

`Physics.Raycast(origin, direction, maxDistance)`, `Physics.SphereCast(origin, radius, direction, maxDistance)`, and `Physics.OverlapSphere(center, radius)` query the physics world. Queries are deferred: each call validates its arguments (non-finite values, a zero-length ray/sweep direction, or a non-positive distance/radius throw immediately and surface as script errors), returns a `PhysicsQuery` handle, and the engine executes the query against the physics world at the frame boundary. The result arrives with the next frame's context. `Physics.TryGetRaycastHit(query, out var hit)` and `Physics.TryGetSphereCastHit(query, out var hit)` report the closest hit's `EntityId`, `Entity`, world-space `Point` and `Normal`, and `Distance`, returning false on a miss; `Physics.TryGetOverlapResult(query, out var entityIds)` reports the overlapped persistent entity ids. Raycasts and sphere casts share the `RaycastHit` payload: for a sphere cast the point is the world-space contact on the hit collider, the normal is that collider's outward surface normal, and the distance is the sweep's travel distance. Results are frame-local — delivered in exactly one frame and expired afterwards — and a handle never resolves on the frame that issued it. Ray distance, sweep distance, and sphere radii are clamped to `ScriptPhysics.MaxQueryDistance` (10,000), and overlap results are sorted and bounded to `ScriptPhysics.MaxOverlapResults` (64). Queries report persistent entity ids, never raw ECS handles.

Every query kind accepts an optional trailing `PhysicsQueryFilter` with three independent knobs. `LayerMask` (uint) restricts candidates to colliders whose `collision_group` shares at least one bit with the mask — colliders already carry a collision group in the scene format, and the default group has every layer bit set, so a query without a mask hits everything and a mask of `1` selects the default layer exactly; a zero mask is rejected as a validation error. `IncludeSensors` (bool, default false) opts sensor (trigger) colliders into the query, preserving the original sensor-excluding behaviour unless requested. `ExcludeEntityId` (persistent id, optional) skips every collider owned by that entity — the standard self-exclusion for character casts; the id is validated, and an id that names no existing entity is rejected with a script error. A `null` filter (or all defaults) reproduces the pre-filter behaviour exactly.

`Physics.DrainAll()` is the batch counterpart to the per-handle lookups: one call returns every result delivered for the frame as `PhysicsQueryResult` entries (each exposing its `Query` handle, a `Kind` — `RaycastHit`, `RaycastMiss`, `SphereCastHit`, `SphereCastMiss`, or `OverlapSphere` — and the matching `Hit`/`EntityIds` payload), ordered by query id. Drained results are consumed, so the `TryGet*` lookups no longer resolve them afterwards; issuing several queries in one frame and draining them all next frame is the intended batch workflow. The per-frame query budget remains 256 queries per script command drain.

`Physics.ApplyForce(entity, force)` / `Physics.ApplyForce(entityId, force)` and the matching `ApplyImpulse`, `ApplyTorque`, and `ApplyTorqueImpulse` overloads enqueue validated rigid-body mutations. Targets use persistent IDs, vectors must be finite, every component is bounded by `ScriptPhysics.MaxMutationComponent` (1,000,000), and the native bridge accepts at most 256 mutations per command drain. Queries issued in the same callback observe the already-stepped current frame; mutations execute at the start of the next physics step. Unknown targets produce script diagnostics and targets without a live dynamic rigid body are safe no-ops in the backend.

`Physics.CreateJoint(jointId, bodyA, bodyB, settings)` creates or replaces a persistent fixed, revolute, prismatic, or spherical constraint. Reusing the same `jointId` is the update operation; `Physics.UpdateJoint` is its explicit alias, and `Physics.RemoveJoint` removes it. `Physics.Grab` / `Physics.ReleaseGrab` are fixed-joint conveniences intended for a kinematic hand or gravity-gun anchor. Anchors, axes, limits, motors, and break thresholds are validated on both the managed and native boundaries. Joints survive scene serialization and checkpoints, follow cell/body availability incrementally, and remove their component after a measured `break_force` or `break_torque` overload so they cannot be recreated on the next sync. See [`PHYSICS_JOINTS.md`](PHYSICS_JOINTS.md).

`Damage.Apply(entity, amount, kind, hitPosition, impulse)` and its persistent-ID overload enqueue bounded damage against `engine.physics.destructible`. Accepted hits apply the component's threshold and scale, persist current health, and emit next-frame `Damage.Events`. On the first break, an optional replacement prefab is spawned at the original position; rigid pieces can inherit the original linear/angular velocity and share the scaled hit impulse. The original entity is removed only after replacement succeeds. See [`PHYSICS_DESTRUCTION.md`](PHYSICS_DESTRUCTION.md).

`Ragdoll.Activate(entity, impulse)`, `Ragdoll.Recover(entity, duration)`, and `Ragdoll.SnapToAnimation(entity)` control an authored `engine.ragdoll` through persistent IDs. The native reconciler generates bounded bone rigid bodies and persistent constraints, switches animation/physics ownership without rebuilding the graph, writes simulated body transforms back into the final skinning pose, and blends back to animation over the requested duration. Confirmed changes arrive in next-frame `Ragdoll.Events`. Generated bodies, joints, active/recovery state, velocities, and recovery progress survive checkpoints. See [`RAGDOLLS.md`](RAGDOLLS.md).

`Components.Query(entityId, componentType)` and `Entity.QueryComponent(componentType)` read the engine's built-in components beyond Transform. Access is registry-driven, not a hardcoded list: a component type is queryable when its component-registry entry declares a script access level of `ReadOnly` or `ReadWrite` **and** carries the scene serde hooks (so scripts and scene files share one field layout). The queryable set is currently `engine.renderable`, `engine.camera`, `engine.light`, `engine.lod_group`, `engine.hlod_cluster`, `engine.interactable`, `engine.audio_source`, `engine.audio_listener`, `engine.physics.rigid_body`, `engine.physics.collider`, `engine.physics.physics_material`, `engine.gravity_source`, `engine.nav_agent`, `engine.terrain_volume`, `engine.planet_surface_anchor`, `engine.vfx.particle_emitter`, `engine.vfx.decal`, and — read-only — `engine.character_controller`; the full per-component matrix with reconciler status and caveats is kept in [COMPONENT_SCRIPT_ACCESS.md](COMPONENT_SCRIPT_ACCESS.md), guarded by a drift test. Any other key — unknown, opted out, hook-less, or routed through a dedicated API such as Transform commands or the retained `UICanvas` handles — is rejected with a `SCRIPT_COMPONENT_UNKNOWN` script error listing the supported set. Reads are deferred exactly like physics queries: `Query` returns a frame-local `ComponentQuery` handle, the engine snapshots the component's fields at the frame boundary (after that frame's commands apply, so same-frame writes are observed), and `Components.TryGet(query, out var snapshot)` delivers the `ComponentSnapshot` with the next frame's context. A handle never resolves on its issuing frame, results are frame-local, and `Components.IsMissing(query)` reports that the entity exists but does not have the component (querying an unknown entity reports missing as well rather than failing). Snapshots expose typed getters — `GetBool`, `GetInt`, `GetUInt`, `GetFloat`, `GetString`, `GetEnum`, `GetAsset`, `GetVector3`, `GetQuaternion`, `GetColor`, `GetList`, and `GetMap` — over the same field map the scene format uses; `HasField` checks presence, and reading an unknown field or with the wrong getter type throws.

`Components.Set(entityId, componentType, fields)`, `Components.SetField(entityId, componentType, field, value)`, and the matching `Entity.SetComponent`/`Entity.SetComponentField` helpers write component fields. Only `ReadWrite` components accept writes: writing a `ReadOnly` component such as `engine.character_controller` is rejected with a `SCRIPT_COMPONENT_READ_ONLY` script error, distinct from `SCRIPT_COMPONENT_UNKNOWN` (not script-accessible at all) and `SCRIPT_COMPONENT_PAYLOAD_INVALID` (malformed fields). Writes are deferred merge commands committed after all script callbacks finish: each provided field merges over the entity's current component (or over authored defaults when the entity lacks the component), so unmentioned fields keep their values. Field values are `ComponentValue` instances produced by the `ComponentValue.From*` factories (with implicit conversions from `bool`, `int`, `long`, `uint`, `ulong`, `float`, `string`, `Vector3`, and `Quaternion`). `FromUInt` serializes as the stable `"uint"` tag; Rust also accepts the legacy `"u_int"` spelling for already-recorded commands. The engine validates every write against the component's scene schema — unknown fields, wrong value types, and invalid enum cases are rejected with a `SCRIPT_COMPONENT_PAYLOAD_INVALID` script error listing the rejected and known fields — so a failed write never partially applies. `engine.renderable` exposes validated `mesh`, `material`, `visible`, `cast_shadows`, and `render_layer` fields; material/mesh changes are re-extracted on the next render frame, enabling live equipment and paint changes without replacing the entity. Backend caveats per component are tracked in [COMPONENT_SCRIPT_ACCESS.md](COMPONENT_SCRIPT_ACCESS.md): writes to `engine.physics.rigid_body`/`engine.physics.collider`/`engine.physics.physics_material` update ECS state (read-back and scene saves observe them) but do not re-sync bodies already created in the physics simulation, `engine.gravity_source` writes take effect on the next physics step because the step re-reads sources from the ECS world, `engine.nav_agent` writes take effect live and restart path following, `engine.terrain_volume` writes restream changed chunks on the next terrain tick, `engine.planet_surface_anchor` writes re-resolve its f64 placement/occupancy on that tick, `engine.vfx.particle_emitter`/`engine.vfx.decal` writes take effect live while intentionally restarting transient particles or lifetime clocks, and `engine.audio_source`/`engine.audio_listener` writes take effect through the audio output reconciler on targets that enable it.

`Network` is the managed multiplayer surface. Session host/connect/disconnect,
entity ownership, replicated-state writes, targeted RPCs, lobby operations, and
friend presence all return deferred `NetworkRequest` handles; results and
received snapshots arrive in the next frame through `Network.TryGetResult`,
`Network.RpcEvents`, and the other read-only collections. See
[NETWORKING.md](NETWORKING.md).

`XR` is the managed OpenXR snapshot. When an XR render host has installed an
active runtime/compositor pair, `XR.Frame` publishes two eye views and
predicted head/hand poses, and typed `TryGet*` methods expose action values.
Desktop builds without an active headset remain valid and report `XR.Active ==
false`; scripts do not need a different assembly. See [XR.md](XR.md).

The process host uses a frame snapshot/command model: entity Transforms and input values are sent before lifecycle execution, then validated commands are committed after all scripts finish. This avoids re-entering the single JSON pipe from inside `OnUpdate`. Consequently, scripts do not observe another script's same-frame writes.

`Scene.Load(sceneId)` therefore queues a request; it does not replace the World
inside the current lifecycle call. The project player resolves the ID against
`game.project.json`, loads it at the host frame boundary, and runs the new
scene's normal script attachment lifecycle before rendering it. An unknown ID,
an invalid scene, or conflicting same-frame requests produce an actionable
runtime error; when requests conflict, the first request wins.

Missing actions, action type mismatches, missing Transforms, invalid numeric
values, invalid or unknown scene IDs, and protocol failures are reported as
script errors instead of being silently ignored.

`Input.WasPressed(action)` and `Input.WasReleased(action)` expose one-update input edges; held values remain available through `GetBool`, `GetFloat`, and `GetVector2`. Digital, scalar, and vector actions all share deterministic native edge detection before the gameplay context is sent to C#.

`UI.Events` contains the runtime UI clicks routed during the current script update. Each `GameplayUiEvent` retains its `CanvasId`, numeric `ElementId`, and optional `CallbackId`; events without a callback ID remain visible in the list. `UI.WasClicked(callbackId)` is a convenience query using an exact, case-sensitive callback-ID match. This bridge currently emits click events only. It does not automatically change or expose Toggle, Checkbox, or Slider values; game code must not infer value changes from a click alone.

This bridge does not yet provide component access beyond the registry-driven `Components` set (Transform, joints, destructibles, ragdolls, and UI canvases keep dedicated paths), named callback methods such as `OnCollisionEnter`, character-controller movement commands, automatic ragdoll fitting, runtime geometric fracture, automatic collision-impulse damage, or in-process `Engine.API` calls. Runtime creation currently supplies a Transform; collision, trigger, and joint-break data is consumed through `Physics.Events`, damage results through `Damage.Events`, ragdoll ownership results through `Ragdoll.Events`, spatial queries through the deferred `Physics.Raycast`/`Physics.SphereCast`/`Physics.OverlapSphere` handles, and pushable/constraint-driven rigid bodies through the physics mutation commands. `ProcessHost` scripts must use this IPC gameplay API; direct `engine-ffi` P/Invoke remains intentionally rejected across processes.

The editor scene panel lists the catalog, creates and opens scenes, and can set
the startup scene. Switching away from a dirty scene requires an explicit
**Save & Switch**, **Discard & Switch**, or **Cancel** decision. Play mode also
honours `Scene.Load` at the same safe frame boundary as the standalone player;
Stop restores the open authoring document rather than saving runtime changes.

## Cooked assets

The cooker writes validated `.cooked` artifacts. A formal `project cook` builds a complete staging directory and swaps it into place only after every declared asset succeeds, so a failed rebuild preserves the previous playable batch. `project check` validates declared cooker/loader mappings and any existing cooked artifact against its manifest kind.

The player verifies each header, payload length, compression marker, and SHA-256 before committing a complete load batch. Mesh, RGBA8 texture, portable PBR materials (opaque, masked, blended, and double-sided), audio clips, animation clips, skeletons, navigation meshes, and prefabs are installed into the shared typed runtime asset registry. If any artifact or subsystem loader fails, the previous runtime batch remains active. Shader, scene, and logic artifacts remain available for their dedicated consumers and are reported as skipped.

Under the hood the whole-directory load is a three-stage pipeline — `decode_cooked_batch` → `validate_cooked_batch` → `commit_cooked_batch` — that game code can also drive directly. `install_cooked_assets_additive(paths)` commits an explicit set of artifacts without unloading anything: an asset ID that is already installed with an identical decoded payload is a no-op success (reported as `identical_assets`), while a differing payload under the same ID is an `AS0003` validation error and the batch installs nothing. For incremental streaming, `enqueue_cooked_asset_stream(paths)` decodes and structurally validates the batch on a background worker thread — every enqueued ID is observable as `AssetState::Loading` via `AssetRegistry::asset_state`/`pending_loads` — and `drain_cooked_asset_stream()` commits finished work additively at the frame boundary, at most `set_cooked_asset_stream_budget(n)` assets per call (default 8) so commit cost per frame is bounded. A batch that fails to decode installs nothing; a commit-time conflict discards the remainder of that batch while previously installed assets stay active. Textures commit before materials within a batch, so same-batch material → texture references resolve even when a budget splits the batch across frames. Additively installed assets join the tracked cooked set, so a later whole-directory replace load unloads them like any startup asset.

## Runtime meshes

Rust systems can create and mutate meshes at runtime — the native foundation for terrain chunks and game-side procedural geometry — through `EngineRuntime`'s runtime mesh API. A runtime mesh is a typed `MeshUpload` registered in the shared asset registry under a derived ID `runtime-mesh-{name}`, so it renders through the exact same per-frame sync and backend upload path as a cooked mesh: point a scene renderable's `mesh` field at `runtime_mesh_asset_id(handle)` and it draws.

- `create_runtime_mesh(name, RuntimeMeshDescriptor)` validates positions/normals/UVs/indices (non-empty finite positions, triangle-list indices in range), computes bounds when the descriptor omits them, and returns a slot+generation `RuntimeMeshHandle`. A duplicate live name is `RuntimeMeshError::DuplicateName`; a derived ID already owned by a foreign (e.g. cooked) asset is `AssetIdConflict`. All uploads use the portable 32-byte PBR vertex format; skinned runtime meshes are not supported yet.
- `update_runtime_mesh(handle, descriptor)` replaces the full vertex/index payload and recomputes bounds; the handle and asset ID stay stable.
- `update_runtime_mesh_vertices(handle, first_vertex, &[Pbr32Vertex])` rewrites a contiguous vertex range in place for LOD morphing or deformation. Index data is deliberately not partially updatable (topology changes require a full update), and partial edits do not recompute bounds.
- `destroy_runtime_mesh(handle)` removes the registry entry immediately, so later frames can no longer resolve the ID.

Handle lifecycle is total: unknown slots return `UnknownHandle`, and a handle used after its mesh was destroyed returns `StaleHandle` — never a panic. Re-creating a destroyed name reuses the slot with a fresh generation, so stale handles stay detectable.

Namespace safety is enforced in both directions: cooked-batch validation rejects any cooked asset ID naming a live runtime mesh (`AS0003`), so whole-directory replace swaps cannot overwrite and later unload runtime meshes, and additive/streamed installs already treat a differing payload under the same ID as a conflict. Conversely, creating a runtime mesh whose derived ID is already registered by a foreign asset fails instead of clobbering it.

GPU frame safety: destruction unregisters the typed mesh immediately. At the next rendered frame (a frames-in-flight boundary), the canonical render-asset synchronizer observes the missing registry entry and removes the backend resource; Vulkan's `remove_resource` performs its own `wait_idle` before freeing buffers, so a mesh referenced by an in-flight frame is never freed mid-frame. Re-creating a mesh under the same name before synchronization leaves one live registry entry, and the next upload replaces the old buffers without a conflicting removal.

Diagnostics: `runtime_mesh_memory()` (also surfaced as `RuntimeDiagnostics.runtime_meshes`) reports live mesh/vertex/index counts plus vertex and index payload bytes. Upload and removal both ride the standard per-frame registry sync (backends dedupe unchanged content by hash), and failed removals remain tracked for retry; there is no separate runtime-mesh GPU scheduler. Per-frame streaming cost stays governed by the cooked-asset stream budget above.

Managed gameplay code receives the same engine-owned registration path through
`RuntimeAssets.RegisterMesh`, `RegisterMaterial`, and `RegisterPrefab`. Each
call returns a `RuntimeAssetRequest`; `RuntimeAssets.TryGetResult` reports the
validated native registration result on a later script tick. Payload size,
finite-number, topology, index, identifier, and prefab-structure limits are
validated before the engine mutates the live registry. Generated assets use
stable runtime namespaces and can immediately be referenced by renderables or
spawned prefabs; scripts do not upload Vulkan/DX12 resources directly.

## Runtime UI, animation, and navigation

Scene `engine.canvas` components retain their complete element tree in the
scene file, including element IDs, kind data, layout, Z order, enabled state,
and child links. Image textures use typed asset references. The loader repairs
invalid IDs and unsafe child graphs deterministically, and computed rectangles
are recalculated after loading. Every render frame lays out all persistent
canvases in stable entity-ID order and submits their UI batches together with
the 3D scene. Text and control labels are rasterized into a shared font atlas,
registered as a renderer texture, and composited after the 3D pass. A project
font under `assets/fonts` is preferred; desktop development builds fall back
to a platform font. Button hover/pressed colors and stateful-control hover
feedback use the retained pointer state. Routed runtime interactions are
exposed to managed gameplay through `UI.Events` and
`UI.WasClicked(callbackId)`, with the originating canvas, element, and optional
Bool/Float value retained.

Managed scripts author runtime UI through retained class handles. Calls enqueue
validated gameplay commands; the Rust runtime applies them to `engine.canvas`
components at the frame boundary. Process-host scripts never receive native
pointers or raw ECS handles:

```csharp
private UICanvas? _hud;
private UIText? _score;
private UIToggle? _music;
private UISlider? _volume;

public void OnCreate()
{
    _hud = UI.CreateCanvas("hud", 1280.0f, 720.0f, UIScaleMode.FitWidth);
    _hud.AddPanel(
        UILayout.Absolute(24.0f, 24.0f, 320.0f, 32.0f),
        new UIColor(20, 20, 20, 210),
        zOrder: 10);
    _score = _hud.AddText(
        UILayout.Absolute(24.0f, 72.0f, 240.0f, 40.0f),
        "Score: 0",
        24.0f,
        UIColor.White,
        zOrder: 11);
    _hud.AddButton(
        UILayout.Absolute(24.0f, 128.0f, 180.0f, 48.0f),
        "Start",
        "start-game",
        new UIColor(70, 90, 180),
        new UIColor(90, 110, 210),
        new UIColor(50, 65, 140),
        zOrder: 12);
    _music = _hud.AddToggle(
        UILayout.Absolute(24.0f, 188.0f, 180.0f, 40.0f),
        "Music", true,
        new UIColor(30, 170, 90),
        new UIColor(80, 80, 80),
        "music-changed",
        zOrder: 12);
    _volume = _hud.AddSlider(
        UILayout.Absolute(24.0f, 240.0f, 240.0f, 40.0f),
        "Volume", 0.75f, 0.0f, 1.0f,
        "volume-changed",
        zOrder: 12);
}

public void OnUpdate(float deltaTime)
{
    _score!.Text = $"Score: {UpdateCount}";
    if (UI.WasClicked("start-game"))
        Scene.Load("level_one");
    // Native pointer input has already synchronized these retained values.
    bool musicEnabled = _music!.IsOn;
    float volume = _volume!.Value;
}
```

`UICanvas` exposes Panel, Image, Text, Button, Toggle, Checkbox, Slider, and
ScrollView creation plus resize/clear/remove operations. `UIElement` handles
can be enabled, disabled, or removed; `UIText.Text`, `UIToggle.IsOn`,
`UICheckbox.IsChecked`, and `UISlider.Value` queue typed replacements. Canvas
creation and script mutations are deferred, so a newly created canvas becomes
visible to the native world after the current script callback finishes. Native
pointer input updates Toggle/Checkbox values on click and Slider values while
dragging; the C# handle is synchronized before the next `OnUpdate`. FitWidth
and FitHeight canvases scale both rendering and hit-testing against the current
window viewport.

With `runtime-subsystems`, `engine.animation_player` and
`engine.skeleton_component` resolve their cooked typed assets on every update.
The latest evaluated skin pose replaces the matching static drawable before
rendering, including when multiple fixed updates occur before one rendered
frame.

An `engine.ai_agent` can drive a `CharacterController` on the same entity from
a cooked navigation mesh. Use `controller_entity_id = 0` for this portable
same-entity binding; serialized raw ECS entity handles are intentionally not
accepted. The loop advances every controller, not only the primary
player-bound one, and synchronizes each resulting position back to its
Transform. Cross-entity AI/controller binding still requires a future
persistent-ID field.

## Runtime audio

`target-desktop` enables `runtime-audio-output` and the cpal backend. An entity
with `engine.audio_source` references a cooked `audio_clip` by asset ID; setting
`playing = true` starts it once the typed asset is loaded. Volume, looping, 2D
or spatial mode, emitter position, maximum distance, and rolloff are reconciled
from the scene every frame. The first enabled `engine.audio_listener` in stable
persistent-ID order supplies the listener transform; without one, the origin
and the engine's -Z/+Y orientation are used.

The output device is opened lazily only when a playable source exists. A
missing or unavailable device emits a warning but does not stop scene loading
or the game loop. Scene switches stop all voices. A finished non-looping source
does not restart every frame while its `playing` request remains true; toggle it
false and then true (or change its clip) to play it again. Headless and tooling
builds can use `runtime-subsystems` without `runtime-audio-output`, so asset,
component, cooking, and validation tests do not require an audio device.

Project scenes remain portable RON files in this version. The Windows packager
rewrites `cooked_assets` to `assets/cooked`, copies every cataloged scene,
omits source assets, records each scene ID/path/hash in release metadata, and
verifies the resulting package by launching it from its staging directory. In
installed mode the package output root defaults to the project's `Dist`
directory and consumes only the verified prebuilt installation toolchain.

## Runtime physics gravity sources

An entity with `engine.gravity_source` shapes the gravity experienced by
dynamic rigid bodies, so games can build planet-style point gravity or
directional gravity fields instead of relying only on the scene's global
gravity vector (`scene_settings.gravity`, default `(0, -9.81, 0)`). Fields:

- `mode` — `Directional` for a constant field along `direction`, or `Point`
  for a planet-style field pulling towards the world-space `center`.
- `enabled` — a disabled source contributes nothing; sources on disabled
  entities are skipped as well.
- `strength` — acceleration in m/s²; negative values repel instead of
  attracting. For point sources with `InverseSquare` falloff this is the
  acceleration one metre from the centre.
- `direction` — pull direction for `Directional` mode. It is normalised at
  resolution time; a zero or non-finite vector contributes nothing.
- `center` — world-space centre for `Point` mode.
- `falloff` — `None` (full strength everywhere in range), `Linear` (full
  strength at the centre ramping to zero at `max_radius`; behaves like `None`
  when no radius is set), or `InverseSquare` (`strength / d²`).
- `max_radius` — optional range limit in metres for `Point` mode. Non-finite
  or non-positive values are treated as unlimited range.

Effective gravity is resolved per dynamic body per fixed physics step. Every
enabled source that reaches the body contributes an acceleration vector and
the contributions are **summed** (superposition); when at least one source
contributes, the sum replaces the global gravity for that body — even when it
cancels to zero — and the body's own `gravity_scale` still multiplies the
result. A body no source reaches (no sources exist, all are disabled, or the
body is outside every point source's `max_radius`) keeps the configured
global gravity exactly as before. Source-driven bodies receive their gravity
as per-step impulses so arbitrary pull directions work, and are kept awake
while a field drives them; bodies back on global gravity keep the usual sleep
behaviour. Static and kinematic bodies are never affected.

A minimal planet setup in a `.scene.ron` entity record:

```ron
(
    persistent_id: "planet-01",
    parent: None,
    name: Some("Planet"),
    enabled: true,
    components: {
        "engine.transform": (
            schema_version: (major: 0, minor: 1, patch: 0),
            enabled: true,
            fields: {
                "translation": Vec3((0.0, 0.0, 0.0)),
            },
        ),
        "engine.gravity_source": (
            schema_version: (major: 0, minor: 1, patch: 0),
            enabled: true,
            fields: {
                "mode": Enum("Point"),
                "strength": Float32(50.0),
                "center": Vec3((0.0, 0.0, 0.0)),
                "falloff": Enum("InverseSquare"),
                "max_radius": Float32(250.0),
            },
        ),
    },
)
```

A dynamic `engine.physics.rigid_body` spawned at `(10, 0, 0)` then
accelerates towards the planet at `50 / 10² = 0.5 m/s²`, while a second body
beyond 250 m keeps falling straight down under the scene's global gravity.

`engine.gravity_source` is part of the script component bridge, so C# reads
and writes it through `Components.Query`/`Components.Set` like the other
built-in components. Unlike `engine.physics.rigid_body` writes, gravity
source writes are re-read by the physics step and therefore take effect on
the next fixed step — a script can, for example, move `center` every frame to
track a flying planet or toggle `enabled` to switch the field off.

## Save games and checkpoints

Authoring scenes remain immutable starting points. Runtime checkpoints use
`GameLoop::capture_save_game(custom_state)` to capture the live ECS scene,
gameplay state, double-precision world origin and shift count, plus the pose,
linear/angular velocity, and sleeping state of every live rigid body keyed by
persistent entity ID. Project-owned inventory, objective, dialogue, and quest
state travels in the typed `BTreeMap<String, engine_serialize::Value>` supplied
by the caller; arbitrary managed C# object fields are intentionally not
reflected or serialized automatically.

```rust
use std::collections::BTreeMap;
use engine_core::{read_save_game, write_save_game};
use engine_serialize::Value;

let custom = BTreeMap::from([
    ("chapter".into(), Value::UInt(4)),
    ("has_suit".into(), Value::Bool(true)),
]);
let checkpoint = game_loop.capture_save_game(custom)?;
write_save_game("saves/quicksave.engsave", &checkpoint)?;

let checkpoint = read_save_game("saves/quicksave.engsave")?;
let restored = game_loop.restore_save_game(checkpoint)?;
let project_state = restored.custom_state;
```

The binary format has a fixed magic header, schema version, bounded payload
length, and SHA-256 payload checksum. Decoding and complete scene construction
finish before the live world is replaced. File replacement writes and flushes
a same-directory temporary file, retains the previous file as a temporary
backup, and rolls it back if the final rename fails. Missing rigid-body IDs are
reported in `SaveGameRestoreReport::skipped_physics_bodies`, allowing a newer
project version to remove a prop without corrupting the entire save.

Persistent joints, destructible health/break state, and ragdoll ownership,
generated graph, and recovery progress are rebuilt from the captured scene,
while rigid-body pose and velocity use the transient state section. Current
checkpoint scope does not include physics solver caches, streaming-driver
request state, automatic C# field reflection, or a user-facing save-slot/
autosave UI. Those belong above the native snapshot API.

## World origin shifting

Large worlds eventually exceed the range where f32 coordinates stay precise
(visible jitter starts a few kilometres from the origin). Periodic world-origin
shifting re-centres the world's f32 storage on the viewer at runtime while
keeping every **logical** position unchanged:

```text
logical_position(entity) = world_origin + Transform.translation
```

The world origin is a double-precision `[f64; 3]` value held by the runtime
`World` (`World::world_origin`); every `Transform.translation` — and every
other f32 world-space runtime value — is stored **relative** to it. A shift by
`delta` advances the origin by `delta` and translates all of that state by
`-delta`, so logical positions are bit-identical before and after.

### Configuration

Origin shifting is opt-in per scene via `scene_settings.origin_shift`
(disabled by default, so existing scenes simulate unchanged):

```ron
scene_settings: (
    // ... other settings ...
    origin_shift: (
        enabled: true,
        threshold: 8000.0,          // metres; default when omitted
        reference_entity: None,      // None = watch the active camera
    ),
)
```

- `enabled` (default `false`) — master switch.
- `threshold` (default `8000.0`) — distance in metres beyond which a shift
  triggers. Non-finite or non-positive values defensively disable the trigger.
- `reference_entity` (default `None`) — persistent ID of the entity watched
  for threshold crossing; when unset the active camera's world position is
  used. When the reference's distance from the origin exceeds `threshold`, the
  runtime shifts by the full reference position, landing the reference back on
  the relative origin.

### Frame-boundary execution

The trigger is evaluated at most once per frame, at the existing frame
boundary — after `update()` and scene-transition processing, in the same seam
as cell-streaming commits, before `render()`. Both sandbox players (headless
and windowed) call `GameLoop::tick_world_origin_shift()` there; `update()`
itself never shifts, so a shift is never observed mid-frame. At most one shift
runs per frame boundary, and because the reference lands on the relative
origin it cannot immediately retrigger.

### Atomic consistency sweep

One shift moves all of the following by `-delta` in the same boundary:

- every root `Transform` in the ECS (children follow through the hierarchy;
  disabled entities included),
- every physics body, teleported **in place** (`set_position` without waking)
  rather than rebuilt — velocities, forces, joints, and sleep state are
  preserved, collider positions are propagated, and the query pipeline is
  refreshed so raycasts and overlaps observe the shifted world immediately,
- every `CharacterController` position, including the primary mirror used by
  `update_character`,
- every navigation agent's target and in-progress path
  (`runtime-subsystems`),
- every point `engine.gravity_source` centre (`gameplay`).

Audio needs no sweep: emitter and listener snapshots are rebuilt from ECS
transforms every frame, and both move together, so relative audio geometry is
seamless. Camera-relative rendering composes unchanged — extraction already
subtracts the current camera translation, which is exactly what the shift
rebased. Cell streaming is origin-aware: bounds tests run in logical
(authored) space, and freshly merged cell hierarchy roots are rebased by
`-origin`.

### Observability

Headless run reports expose `world_origin` (final `[x, y, z]`) and
`world_origin_shifts` (cumulative shift count). `GameLoop` also surfaces
`world_origin()`, `world_origin_shift_count()`, and
`last_world_origin_shift()` (per-sweep counts of transforms, physics bodies,
character controllers, nav agents, and gravity sources moved). C# scripts see
a read-only `ScriptWorldOrigin WorldOrigin` on `EngineBehaviour`
(double precision); all script-visible positions stay origin-relative, so game
code never needs to add the origin itself.

### Current limitations

- **f32 storage still bounds local scale.** Shifting keeps the *viewer* near
  the origin; content more than ~threshold metres from the reference still
  loses f32 precision. Size cells and streaming radii accordingly.
- **Authoring scenes and checkpoints differ.** Plain scene files serialize
  origin-relative authored transforms and load with a zero runtime origin.
  `SaveGameSnapshot` stores that relative scene together with its exact
  double-precision origin and shift counter, restoring both without translating
  the already-relative component data a second time.
- **Cell scenes keep non-Transform world-space data un-rebased.** When a cell
  merges under a non-zero origin, only hierarchy-root `Transform`s are rebased
  by `-origin`; other world-space component data authored inside the cell
  (e.g. an `engine.nav_agent` target or an `engine.gravity_source` centre) is
  not rebased in this version. Author such components in the startup scene, or
  keep cells to plain geometry.
- The editor does not shift origins (it edits authored scenes directly); the
  shift is a player-runtime behaviour and tooling builds are unaffected.
