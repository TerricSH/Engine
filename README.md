# Engine

This repository contains a Rust game engine, project player, editor, asset cooker, and Windows release pipeline. The supported workflow starts from a `game.project.json` and uses the `project`, `game`, or `editor` command families.

## Create and run a game

```powershell
cargo run -p sandbox -- project new .\MyGame --name MyGame
cargo run -p sandbox -- project import .\MyGame .\checker.ppm --id checker
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

For a managed project, add `subsystem-scripting-csharp` to the editor feature
list. The editor rebuilds C# scripts before every Play session, supports the
project scene catalog, protects dirty documents during scene switches and
window close, and can recook project assets from the Asset Browser. Generated
C# scripts can query persistent scene entities, create Transform-bearing
runtime entities, edit or destroy target entities, request scene changes, and
consume per-frame collision/trigger data through `Physics.Events`.
Frame-accurate input edges are available to managed gameplay scripts through
`Input.WasPressed(...)` and `Input.WasReleased(...)`.

The desktop/editor/scripting feature sets also enable the registered runtime
UI, audio, animation, and navigation component and asset extensions. The normal
GameLoop now lays out and renders retained Canvases, evaluates skinned
animation, advances navigation-driven characters, and validates their typed
cooked assets. The desktop target adds `runtime-audio-output`: scene
`AudioSource` and `AudioListener` components drive the cpal output stream every
frame, while headless builds retain device-free component and asset support.
See the detailed status and limits in
[`docs/GAME_PROJECTS.md`](docs/GAME_PROJECTS.md).

The ready-to-run sample is at [`examples/minimal-game`](examples/minimal-game). It uses cooked mesh, texture, and material data in its startup scene.

## Package a project

```powershell
.\.github\scripts\package-windows.ps1 `
  -ProjectPath .\examples\minimal-game\game.project.json
```

The package contains the project manifest, every cataloged scene, cooked assets, runtime executable, reports, checksums, and symbols. Scene paths and hashes are recorded in release metadata. Its smoke test launches the packaged project for three headless frames and requires visible indexed geometry.

See [`docs/GAME_PROJECTS.md`](docs/GAME_PROJECTS.md),
[`docs/GAME_ENGINE_BOUNDARY.md`](docs/GAME_ENGINE_BOUNDARY.md),
[`docs/CI_RELEASE_GATES.md`](docs/CI_RELEASE_GATES.md), and
[`docs/RELEASE_PACKAGING.md`](docs/RELEASE_PACKAGING.md) for details.
