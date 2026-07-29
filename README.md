# Engine

Architecture boundaries and feature-gating rules are documented in
[`docs/ENGINE_CORE_ARCHITECTURE.md`](docs/ENGINE_CORE_ARCHITECTURE.md).
The JRPG rendering baseline and backend parity limits are documented in
[`docs/RENDERING_ARCHITECTURE.md`](docs/RENDERING_ARCHITECTURE.md).

This repository contains a Rust game engine, project player, editor, asset
cooker, and Windows release pipeline. The product boundary is an installed
engine application plus an external game-project workspace: the engine
installation is treated as read-only, while project source, generated outputs,
and packages stay under the directory that contains `game.project.json`.

## Installed editor workflow

An assembled Windows installation contains `bin/EngineEditor.exe`, a
precompiled game runtime and asset cooker, the managed gameplay SDK and script
host, and `engine.installation.json`. Open a project outside that installation
by passing either its directory or manifest:

```powershell
$engine = "C:\Engine\v0.1.0\windows-x86_64"
$project = "D:\Games\MyGame"

& "$engine\bin\EngineEditor.exe" project new $project --name MyGame --with-csharp
& "$engine\bin\EngineEditor.exe" project editor $project
```

For a scripted project, opening it validates the installation and project
contract, then deploys the prebuilt SDK and host to
`build/script-sdk/EngineGameplay.dll` and `build/script-host/`. It does not
build game-authored C# merely because the project was opened; Rebuild, Play,
Build, or `project build-scripts` performs that work. Project content and
generated game outputs remain in `$project`; Windows packages default to
`$project\Dist`.

Installed authoring and packaging do not require the engine source tree, Cargo,
Rust, or Git. C# projects do require a .NET 8 SDK to compile game scripts, and
their framework-dependent script host requires a .NET 8 runtime. See
[`docs/ENGINE_INSTALLATION.md`](docs/ENGINE_INSTALLATION.md) for the installed
layout and trust boundary.

## Source-tree development

The following commands are for engine maintainers working from this checkout.
They compile the engine with Cargo; they are not the installed-user workflow.

```powershell
cargo run -p sandbox -- project new .\MyGame --name MyGame
cargo run -p sandbox -- project import .\MyGame .\checker.ppm --id checker
cargo run -p sandbox -- project import .\MyGame .\character.glb --id character
cargo run -p sandbox -- project scene new .\MyGame level_two --name "Level Two"
cargo run -p sandbox -- project scene set-startup .\MyGame level_two
cargo run -p sandbox -- project check .\MyGame
cargo run -p sandbox -- project cook .\MyGame
cargo run -p sandbox -- game .\MyGame --headless --frames 3
```

Create a project with a C# lifecycle script and build all project outputs:

```powershell
cargo run -p sandbox --features subsystem-scripting-csharp -- `
  project new .\ManagedGame --name ManagedGame --with-csharp
cargo run -p sandbox --features subsystem-scripting-csharp -- `
  project build .\ManagedGame
cargo run -p sandbox --features subsystem-scripting-csharp -- `
  game .\ManagedGame --headless --frames 3
```

Run the native Vulkan player or editor:

```powershell
cargo run -p sandbox --features backend-vulkan,target-desktop -- game .\MyGame
cargo run -p sandbox --features backend-vulkan,tooling-editor,target-desktop -- editor .\MyGame
```

For a managed source-development project, add `subsystem-scripting-csharp` to
the editor feature list. The editor rebuilds C# scripts before every Play
session, supports the
project scene catalog, protects dirty documents during scene switches and
window close, and can recook project assets from the Asset Browser. Generated
C# scripts can query persistent scene entities, create Transform-bearing
runtime entities, edit or destroy target entities, request scene changes, and
consume per-frame collision/trigger data through `Physics.Events`. Deferred
physics queries plus validated linear/angular force commands and persistent
`Physics.CreateJoint` / `Physics.Grab` constraints provide the native base for
pushable props, hinges, sliders, and gravity-gun-style interaction.
Frame-accurate input edges are available to managed gameplay scripts through
`Input.WasPressed(...)` and `Input.WasReleased(...)`.
Turn-based tactical games additionally get a deterministic grid/cover/visibility
domain, turn command queues, combat and utility-AI strategies, renderer-consistent
pointer rays, data-driven Logic assets, and script checkpoint slots. See
[`docs/TACTICAL_GAME_PLATFORM.md`](docs/TACTICAL_GAME_PLATFORM.md).
Party-based JRPG projects additionally get progression, roster/inventory,
ATB/command combat, status effects, encounters, quests, dialogue, localization,
cutscene sequences, and typed script audio/animation helpers. See
[`docs/JRPG_GAME_PLATFORM.md`](docs/JRPG_GAME_PLATFORM.md).

The desktop/editor/scripting feature sets also enable the registered runtime
UI, audio, animation, and navigation component and asset extensions. The normal
GameLoop now lays out and renders retained Canvases, evaluates skinned
animation, advances navigation-driven characters, and validates their typed
cooked assets. The desktop target adds `runtime-audio-output`: scene
`AudioSource` and `AudioListener` components drive the cpal output stream every
frame, while headless builds retain device-free component and asset support.
Importing a glTF/GLB character creates deterministic mesh assets plus cooked
skeleton and animation companions; multi-primitive files and relative external
buffer/image files are handled in the same transaction.
See the detailed status and limits in
[`docs/GAME_PROJECTS.md`](docs/GAME_PROJECTS.md). The engine-versus-Half-Life-2
readiness audit and remaining production gaps are tracked in
[`docs/HL2_READINESS.md`](docs/HL2_READINESS.md).
Joint authoring and the managed API are documented in
[`docs/PHYSICS_JOINTS.md`](docs/PHYSICS_JOINTS.md).
Destructible props, damage events, and prefab fracture replacement are
documented in
[`docs/PHYSICS_DESTRUCTION.md`](docs/PHYSICS_DESTRUCTION.md).
Skeletal ragdoll authoring, pose ownership, recovery, and persistence are
documented in [`docs/RAGDOLLS.md`](docs/RAGDOLLS.md).
Portable opaque, masked, alpha-blended, additive, and double-sided material surfaces are
documented in [`docs/MATERIAL_SURFACES.md`](docs/MATERIAL_SURFACES.md).
CPU particle emitters and mesh-based lifetime decals are documented in
[`docs/VFX.md`](docs/VFX.md).
Bounded character commands and the project-facing use/grab convention are
documented in [`docs/INTERACTION.md`](docs/INTERACTION.md).

The ready-to-run sample is at [`examples/minimal-game`](examples/minimal-game). It uses cooked mesh, texture, and material data in its startup scene.

## Package a project from an installation

```powershell
$engine = "C:\Engine\v0.1.0\windows-x86_64"
& "$engine\tools\package-windows.ps1" `
  -EngineInstallRoot $engine `
  -ProjectPath "D:\Games\MyGame\game.project.json"
```

The default output is
`D:\Games\MyGame\Dist\<version>\`. The package contains the project manifest,
every cataloged scene, cooked assets, runtime executable, reports, checksums,
and symbols. Scene paths and hashes are recorded in release metadata. Its smoke
test launches the packaged project for three headless frames and requires
visible indexed geometry. Existing version directories are not overwritten.

Maintainers can assemble the installation itself, and can still run the
source-tree release path, as documented in
[`docs/RELEASE_PACKAGING.md`](docs/RELEASE_PACKAGING.md).

See [`docs/GAME_PROJECTS.md`](docs/GAME_PROJECTS.md),
[`docs/GAME_ENGINE_BOUNDARY.md`](docs/GAME_ENGINE_BOUNDARY.md),
[`docs/PHYSICS_JOINTS.md`](docs/PHYSICS_JOINTS.md),
[`docs/PHYSICS_DESTRUCTION.md`](docs/PHYSICS_DESTRUCTION.md),
[`docs/RAGDOLLS.md`](docs/RAGDOLLS.md),
[`docs/MATERIAL_SURFACES.md`](docs/MATERIAL_SURFACES.md),
[`docs/VFX.md`](docs/VFX.md),
[`docs/INTERACTION.md`](docs/INTERACTION.md),
[`docs/TACTICAL_GAME_PLATFORM.md`](docs/TACTICAL_GAME_PLATFORM.md),
[`docs/JRPG_GAME_PLATFORM.md`](docs/JRPG_GAME_PLATFORM.md),
[`docs/RENDERING_ARCHITECTURE.md`](docs/RENDERING_ARCHITECTURE.md),
[`docs/HL2_READINESS.md`](docs/HL2_READINESS.md),
[`docs/CI_RELEASE_GATES.md`](docs/CI_RELEASE_GATES.md),
[`docs/ENGINE_INSTALLATION.md`](docs/ENGINE_INSTALLATION.md), and
[`docs/RELEASE_PACKAGING.md`](docs/RELEASE_PACKAGING.md) for details.
