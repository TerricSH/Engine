# Game projects

Every authoring and runtime command is rooted by `game.project.json`. Paths in the manifest are relative to the manifest directory and may not be absolute or contain `..` traversal.

## Layout

```text
MyGame/
  game.project.json
  assets/
    scenes/main.scene.ron
    scenes/level-two.scene.ron
    source/game.manifest
  config/input.actions.json
  scripts/GameScripts/       # optional C# authoring source
  build/cooked/
  build/scripts/             # optional compiled game assembly
  build/script-host/         # optional .NET protocol host
```

`project new` creates the core layout and `main.scene.ron` with a camera and a visible renderable; additional scene files appear after `project scene new`. It also creates an empty source manifest. `build/` is generated and ignored by Git.

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

## Commands

```powershell
sandbox project new <directory> [--name NAME] [--with-csharp]
sandbox project import <project> <source-file> --id <asset-id> [--type mesh|texture|material|audio|animation|skeleton|navmesh]
sandbox project check <project> [--report PATH]
sandbox project scene list <project>
sandbox project scene new <project> <scene-id> [--name NAME]
sandbox project scene set-startup <project> <scene-id>
sandbox project cook <project>
sandbox project build-scripts <project>
sandbox project build <project>
sandbox project run <project> [--headless] [--frames N] [--report PATH]
sandbox project editor <project>
```

`sandbox game` and `sandbox editor` are short aliases for the last two commands. A project path may be either the project directory or its manifest.

`project import` copies a supported source into the configured `asset_source`, adds it to `game.manifest`, cooks the project in an isolated staging directory, and installs the validated `<asset-id>.cooked` artifact. Mesh imports accept glTF/GLB, textures accept supported image formats such as PNG, JPEG, and PPM, and materials accept `MaterialSource-v0` JSON. Runtime subsystem assets accept WAV/MP3/OGG/FLAC audio, bincode `.anim` animation clips, bincode `.skel` skeletons, and bincode `.navmesh`/`.nav` navigation meshes. Type inference is used only for unambiguous extensions; `--type material` can identify a plain `.json` material. Asset IDs are portable and case-insensitively unique. Existing source or cooked files are never overwritten, and a failed cook restores the manifest and removes the copied source.

The editor Asset Browser exposes **Reimport Project Assets**. It recooks the
configured source tree and refreshes the live typed asset registry without restarting
the editor. Importing a brand-new external file still uses `project import`,
which provides the explicit asset ID and transactional copy/manifest update.

`project scene list` prints the catalog and marks its startup entry.
`project scene new` creates a visible starter scene at
`assets/scenes/<scene-id>.scene.ron` and adds it to the catalog without
overwriting an existing ID or file. `project scene set-startup` accepts only an
existing catalog ID and stores that ID in `startup_scene`.

`project check` validates the manifest and safe paths, then loads **every**
cataloged scene. For each scene it checks the scene schema, script references,
strict ECS restoration, and declared asset dependencies; it also validates the
project input map, source manifests, duplicate IDs, and source files. Its JSON
report also records validated cooked render/extension counts. Its other fields
include `startup_scene_id`, the scene count, and an entity count per scene.
`game --headless` runs the normal GameLoop update/render path and fails
if the active scene produces no visible draw calls.

`--with-csharp` creates a .NET 8 class library and an `engine.script` attachment. `build-scripts` compiles the declared DLL, runs the managed gameplay-bridge self-test, and publishes the engine-owned process host; authoring `game` and `editor` launches rebuild configured scripts automatically. Runtime reports expose loaded assemblies, attached/started instances, script-entity translations, and script errors.

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

    if (UI.WasClicked("start-game"))
        Scene.Load("level_one");

    foreach (var uiEvent in UI.Events)
        Console.WriteLine($"UI click: {uiEvent.CanvasId}/{uiEvent.ElementId} ({uiEvent.CallbackId})");
}
```

`Scene.Entities`, `Scene.Exists(id)`, `Scene.FindEntity(id)`, and `Scene.GetEntity(id)` operate on the current frame's persistent-entity snapshot. `Scene.CreateEntity(id)` and `Scene.CreateEntity(id, translation)` enqueue a new persistent entity with an identity Transform or the requested translation. Creation is validated and committed at the frame boundary, so the entity becomes queryable in the next frame; duplicate same-frame IDs use deterministic first-wins semantics. `Scene.DestroySelf()`, `Scene.Destroy(id)`, and `Entity.Destroy()` use the same deferred mutation boundary. Scripts never receive raw ECS handles.

`Physics.Events` contains the owning entity's collision and trigger events for the current frame. Event kinds are `collision_entered`, `collision_stayed`, `collision_exited`, `trigger_entered`, `trigger_stayed`, and `trigger_exited`; `OtherEntityId` and `Other` identify the other persistent scene entity. The native physics queue is drained every frame even when no script consumes it, so events cannot accumulate indefinitely.

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

This bridge does not yet provide prefab instantiation, arbitrary component access for newly-created entities, named callback methods such as `OnCollisionEnter`, physics-aware movement commands, or in-process `Engine.API` calls. Runtime creation currently supplies a Transform; collision and trigger data is consumed through `Physics.Events`. `ProcessHost` scripts must use this IPC gameplay API; direct `engine-ffi` P/Invoke remains intentionally rejected across processes.

The editor scene panel lists the catalog, creates and opens scenes, and can set
the startup scene. Switching away from a dirty scene requires an explicit
**Save & Switch**, **Discard & Switch**, or **Cancel** decision. Play mode also
honours `Scene.Load` at the same safe frame boundary as the standalone player;
Stop restores the open authoring document rather than saving runtime changes.

## Cooked assets

The cooker writes validated `.cooked` artifacts. A formal `project cook` builds a complete staging directory and swaps it into place only after every declared asset succeeds, so a failed rebuild preserves the previous playable batch. `project check` validates declared cooker/loader mappings and any existing cooked artifact against its manifest kind.

The player verifies each header, payload length, compression marker, and SHA-256 before committing a complete load batch. Mesh, RGBA8 texture, the current opaque PBR material subset, audio clips, animation clips, skeletons, and navigation meshes are installed into the shared typed runtime asset registry. If any artifact or subsystem loader fails, the previous runtime batch remains active. Shader, scene, and logic artifacts remain available for their dedicated consumers and are reported as skipped.

## Runtime UI, animation, and navigation

Scene `engine.canvas` components retain their complete element tree in the
scene file, including element IDs, kind data, layout, Z order, enabled state,
and child links. Image textures use typed asset references. The loader repairs
invalid IDs and unsafe child graphs deterministically, and computed rectangles
are recalculated after loading. Every render frame lays out all persistent
canvases in stable entity-ID order and submits their UI batches together with
the 3D scene. Routed runtime clicks are exposed to managed gameplay through
`UI.Events` and `UI.WasClicked(callbackId)`, with the originating canvas and
element retained. This is a click-event bridge only; Toggle, Checkbox, and
Slider values are not mutated automatically.

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
