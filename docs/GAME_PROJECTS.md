# Game projects

Every authoring and runtime command is rooted by `game.project.json`. Paths in the manifest are relative to the manifest directory and may not be absolute or contain `..` traversal.

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
  build/script-sdk/          # optional engine-owned EngineGameplay.dll
  build/cooked/
  build/scripts/             # optional game assembly plus managed dependencies
  build/script-host/         # optional .NET protocol host
```

`project new` creates the core layout and `main.scene.ron` as an immediately usable basic scene with a positioned Main Camera, a visible Cube, and a Directional Light; additional scene files appear after `project scene new`. It also creates an empty source manifest. `build/` is generated and ignored by Git.

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
    }
  }
}
```

Each key of `cells` is a stable cell ID following the same identifier rules as
scene IDs (1–128 ASCII letters, digits, hyphens, underscores, or dots, except
`.` and `..`, case-insensitively unique). `scene` must reference an ID from the
project scene catalog. `bounds` is an axis-aligned box: `center` and
`half_extents` must be finite, and half extents must be non-negative. Cells
**may overlap** — overlapping bounds are a legitimate way to layer content,
such as a dense gameplay cell over a large background cell.

`project check` validates the partition file when it is present (schema, cell
IDs, scene references, and bounds) and reports the cell count as
`partition_cells` in its JSON report. **Cell streaming is not yet active**: the
runtime still loads only the startup scene, and the manifest is currently an
authoring/validation contract that future streaming phases will consume.

## Commands

```powershell
sandbox project new <directory> [--name NAME] [--with-csharp]
sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|audio|animation|skeleton|navmesh|prefab]
sandbox project check <project> [--report PATH]
sandbox project scene list <project>
sandbox project scene new <project> <scene-id> [--name NAME]
sandbox project scene set-startup <project> <scene-id>
sandbox project cook <project>
sandbox project sync-script-api <project>
sandbox project build-scripts <project>
sandbox project build <project>
sandbox project run <project> [--headless] [--frames N] [--report PATH]
sandbox project editor <project>
```

`sandbox game` and `sandbox editor` are short aliases for the last two commands. A project path may be either the project directory or its manifest.

`project import` copies a supported source into the configured `asset_source`, adds it to `game.manifest`, cooks the project in an isolated staging directory, and installs the validated `<asset-id>.cooked` artifact. Mesh imports accept glTF/GLB, textures accept supported image formats such as PNG, JPEG, and PPM, and materials accept `MaterialSource-v0` JSON. Runtime subsystem assets accept WAV/MP3/OGG/FLAC audio, bincode `.anim` animation clips, bincode `.skel` skeletons, and bincode `.navmesh`/`.nav` navigation meshes. Prefab imports accept the canonical `Prefab-v0.1.0` RON source (`*.prefab.ron`), which is also the extension inferred when `--type` is omitted. Type inference is used only for unambiguous extensions; `--type material` can identify a plain `.json` material. Asset IDs are portable and case-insensitively unique. Existing source or cooked files are never overwritten, and a failed cook restores the manifest and removes the copied source.

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
include `startup_scene_id`, the scene count, an entity count per scene, and
`partition_cells`.
`game --headless` runs the normal GameLoop update/render path and fails
if the active scene produces no visible draw calls.

`--with-csharp` creates an empty .NET 8 class library and the engine SDK
integration. It does not invent a script class or attach one to the scene.
`sync-script-api` refreshes the engine-owned version/hash sidecar and MSBuild
integration after an engine upgrade; game rules belong in behaviour sources
created explicitly by the project author. The canonical `EngineGameplay.cs` is generated under
`build/script-sdk-source`, not in the game source directory. `build-scripts`
rejects a missing or modified integration, compiles `EngineGameplay.dll`
independently, compiles the declared game DLL against that SDK, runs the managed
gameplay-bridge self-test, and publishes the engine-owned process host. The SDK
dependency is copied beside `GameScripts.dll` for runtime packaging. Authoring
`game` and `editor` launches rebuild configured scripts automatically. Runtime
reports expose loaded assemblies, attached/started instances, script-entity
translations, and script errors. See
[`GAME_ENGINE_BOUNDARY.md`](GAME_ENGINE_BOUNDARY.md) for the enforced ownership
and review rules.

The generated `Main` derives from `EngineBehaviour`. During `OnCreate`, `OnStart`, and `OnUpdate`, it can read and write its owning entity's local `Transform`, read the current resolved project input actions, query persistent entities, edit another entity's Transform, destroy entities, and request a cataloged scene by ID:

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
        Console.WriteLine($"ray hit {groundHit.EntityId} at {groundHit.Point}");

    if (UI.WasClicked("start-game"))
        Scene.Load("level_one");

    foreach (var uiEvent in UI.Events)
        Console.WriteLine($"UI click: {uiEvent.CanvasId}/{uiEvent.ElementId} ({uiEvent.CallbackId})");
}
```

`Scene.Entities`, `Scene.Exists(id)`, `Scene.FindEntity(id)`, and `Scene.GetEntity(id)` operate on the current frame's persistent-entity snapshot. `Scene.CreateEntity(id)` and `Scene.CreateEntity(id, translation)` enqueue a new persistent entity with an identity Transform or the requested translation. Creation is validated and committed at the frame boundary, so the entity becomes queryable in the next frame; duplicate same-frame IDs use deterministic first-wins semantics. `Scene.DestroySelf()`, `Scene.Destroy(id)`, and `Entity.Destroy()` use the same deferred mutation boundary. Scripts never receive raw ECS handles.

`Scene.Spawn(prefabId)` and `Scene.Spawn(prefabId, translation)` instantiate a cooked prefab asset. `prefabId` is the asset id declared for a `Prefab` entry in the project's source manifest — not a file path — and it follows the same identifier rules as entity ids. Spawning is validated and committed at the frame boundary exactly like `Scene.CreateEntity`, so the new hierarchy becomes queryable through `Scene.FindEntity` on the next frame. The spawned root takes the first free persistent id from `prefabId`, `prefabId-2`, `prefabId-3`, …; every other spawned entity takes `<rootId>.<prefab-local id>` (with the same `-N` suffix on conflict), so two spawns of `prefab-enemy` produce `prefab-enemy` + `prefab-enemy.<child>` and `prefab-enemy-2` + `prefab-enemy-2.<child>`. The optional translation override replaces the prefab root's Transform translation while preserving the prefab's own rotation and scale. `engine.script` components inside the prefab attach to the spawned entities as part of the same boundary: their `OnCreate` runs immediately, and commands it enqueues (including further spawns, depth-bounded) are applied recursively before the next frame. Unknown or invalid prefab ids, non-finite translations, and unloadable prefab graphs surface as script errors; a failed spawn rolls back the whole instance.

To author a spawnable prefab, write a `Prefab-v0.1.0` RON document under the configured `asset_source` (for example `assets/source/Prefabs/enemy.prefab.ron`) or import an existing file with `project import --type prefab`, declare it in `game.manifest` with `asset_type: "Prefab"`, and cook the project. `project check` parses every declared prefab source, verifies that component fields reference only declared assets or engine builtins, requires nested `child_prefab_refs` to point at other declared prefab assets, and rejects missing children and cycles in the nested graph.

`Physics.Events` contains the owning entity's collision and trigger events for the current frame. Event kinds are `collision_entered`, `collision_stayed`, `collision_exited`, `trigger_entered`, `trigger_stayed`, and `trigger_exited`; `OtherEntityId` and `Other` identify the other persistent scene entity. The native physics queue is drained every frame even when no script consumes it, so events cannot accumulate indefinitely.

`Physics.Raycast(origin, direction, maxDistance)` and `Physics.OverlapSphere(center, radius)` query the physics world. Queries are deferred: each call validates its arguments (non-finite values, a zero-length ray direction, or a non-positive distance/radius throw immediately and surface as script errors), returns a `PhysicsQuery` handle, and the engine executes the query against the physics world at the frame boundary. The result arrives with the next frame's context. `Physics.TryGetRaycastHit(query, out var hit)` reports the closest hit's `EntityId`, `Entity`, world-space `Point` and `Normal`, and `Distance`, returning false on a miss; `Physics.TryGetOverlapResult(query, out var entityIds)` reports the overlapped persistent entity ids. Results are frame-local — delivered in exactly one frame and expired afterwards — and a handle never resolves on the frame that issued it. Ray distance and sphere radius are clamped to `ScriptPhysics.MaxQueryDistance` (10,000), and overlap results are sorted and bounded to `ScriptPhysics.MaxOverlapResults` (64). Queries report persistent entity ids, never raw ECS handles; sensors such as trigger volumes are excluded from query results.

`Components.Query(entityId, componentType)` and `Entity.QueryComponent(componentType)` read the engine's built-in components beyond Transform. The supported component type keys are `engine.camera`, `engine.light`, `engine.audio_source`, `engine.physics.rigid_body`, `engine.physics.collider`, and `engine.gravity_source`; any other key is rejected with a `SCRIPT_COMPONENT_UNKNOWN` script error listing the supported set. Reads are deferred exactly like physics queries: `Query` returns a frame-local `ComponentQuery` handle, the engine snapshots the component's fields at the frame boundary (after that frame's commands apply, so same-frame writes are observed), and `Components.TryGet(query, out var snapshot)` delivers the `ComponentSnapshot` with the next frame's context. A handle never resolves on its issuing frame, results are frame-local, and `Components.IsMissing(query)` reports that the entity exists but does not have the component (querying an unknown entity reports missing as well rather than failing). Snapshots expose typed getters — `GetBool`, `GetInt`, `GetUInt`, `GetFloat`, `GetString`, `GetEnum`, `GetAsset`, `GetVector3`, `GetQuaternion`, `GetColor`, `GetList`, and `GetMap` — over the same field map the scene format uses; `HasField` checks presence, and reading an unknown field or with the wrong getter type throws.

`Components.Set(entityId, componentType, fields)`, `Components.SetField(entityId, componentType, field, value)`, and the matching `Entity.SetComponent`/`Entity.SetComponentField` helpers write component fields. Writes are deferred merge commands committed after all script callbacks finish: each provided field merges over the entity's current component (or over authored defaults when the entity lacks the component), so unmentioned fields keep their values. Field values are `ComponentValue` instances produced by the `ComponentValue.From*` factories (with implicit conversions from `bool`, `int`, `long`, `uint`, `ulong`, `float`, `string`, `Vector3`, and `Quaternion`). The engine validates every write against the component's scene schema — unknown fields, wrong value types, and invalid enum cases are rejected with a `SCRIPT_COMPONENT_PAYLOAD_INVALID` script error listing the rejected and known fields — so a failed write never partially applies. Three backend caveats: writes to `engine.physics.rigid_body`/`engine.physics.collider` update ECS state (read-back and scene saves observe them) but do not re-sync bodies already created in the physics simulation, `engine.gravity_source` writes take effect on the next physics step because the step re-reads sources from the ECS world, and `engine.audio_source` writes take effect through the audio output reconciler on targets that enable it.

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

This bridge does not yet provide component access beyond the curated `Components` set (Transform keeps its dedicated path and UI canvases stay on their retained handles), named callback methods such as `OnCollisionEnter`, physics-aware movement commands, or in-process `Engine.API` calls. Runtime creation currently supplies a Transform; collision and trigger data is consumed through `Physics.Events`, and spatial queries through the deferred `Physics.Raycast`/`Physics.OverlapSphere` handles. `ProcessHost` scripts must use this IPC gameplay API; direct `engine-ffi` P/Invoke remains intentionally rejected across processes.

The editor scene panel lists the catalog, creates and opens scenes, and can set
the startup scene. Switching away from a dirty scene requires an explicit
**Save & Switch**, **Discard & Switch**, or **Cancel** decision. Play mode also
honours `Scene.Load` at the same safe frame boundary as the standalone player;
Stop restores the open authoring document rather than saving runtime changes.

## Cooked assets

The cooker writes validated `.cooked` artifacts. A formal `project cook` builds a complete staging directory and swaps it into place only after every declared asset succeeds, so a failed rebuild preserves the previous playable batch. `project check` validates declared cooker/loader mappings and any existing cooked artifact against its manifest kind.

The player verifies each header, payload length, compression marker, and SHA-256 before committing a complete load batch. Mesh, RGBA8 texture, the current opaque PBR material subset, audio clips, animation clips, skeletons, navigation meshes, and prefabs are installed into the shared typed runtime asset registry. If any artifact or subsystem loader fails, the previous runtime batch remains active. Shader, scene, and logic artifacts remain available for their dedicated consumers and are reported as skipped.

Under the hood the whole-directory load is a three-stage pipeline — `decode_cooked_batch` → `validate_cooked_batch` → `commit_cooked_batch` — that game code can also drive directly. `install_cooked_assets_additive(paths)` commits an explicit set of artifacts without unloading anything: an asset ID that is already installed with an identical decoded payload is a no-op success (reported as `identical_assets`), while a differing payload under the same ID is an `AS0003` validation error and the batch installs nothing. For incremental streaming, `enqueue_cooked_asset_stream(paths)` decodes and structurally validates the batch on a background worker thread — every enqueued ID is observable as `AssetState::Loading` via `AssetRegistry::asset_state`/`pending_loads` — and `drain_cooked_asset_stream()` commits finished work additively at the frame boundary, at most `set_cooked_asset_stream_budget(n)` assets per call (default 8) so commit cost per frame is bounded. A batch that fails to decode installs nothing; a commit-time conflict discards the remainder of that batch while previously installed assets stay active. Textures commit before materials within a batch, so same-batch material → texture references resolve even when a budget splits the batch across frames. Additively installed assets join the tracked cooked set, so a later whole-directory replace load unloads them like any startup asset.

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

Project scenes remain portable RON files in this version. The Windows packager rewrites `cooked_assets` to `assets/cooked`, copies every cataloged scene, omits source assets, records each scene ID/path/hash in release metadata, and verifies the resulting package by launching it from its staging directory.

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
