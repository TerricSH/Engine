# Windows engine installation and project packaging

The Windows pipeline now has two distinct products:

1. an engine installation directory assembled by maintainers from the source
   repository; and
2. a game package exported from an external project workspace by that installed
   toolchain.

The installed path is the application boundary. It consumes immutable,
precompiled tools described by `engine.installation.json`; it does not locate
the engine checkout and does not invoke Cargo, Rust, or Git. The source-tree
path remains available for engine development and release verification.

The current deliverable is a versioned, relocatable directory, not an MSI,
installer UI, or automatic updater.

## Package an external project with an installed engine

The project may live anywhere outside the engine installation. From any
PowerShell working directory:

```powershell
$engine = "C:\Engine\v0.1.0\windows-x86_64"
$project = "D:\Games\MyGame\game.project.json"

& "$engine\tools\package-windows.ps1" `
  -EngineInstallRoot $engine `
  -ProjectPath $project
```

`-ProjectPath` is required in installed mode and accepts either
`game.project.json` or its containing directory. Unless `-OutputRoot` is
specified, output is written to the project's `Dist` directory; an explicit
installed-mode output must still be a dedicated non-reparse-point directory
inside that project workspace. The version is chosen from `-Version`, then
`RELEASE_VERSION`, then the installation's `engine_version`.

```text
D:\Games\MyGame\
  game.project.json
  assets/
  scripts/
  build/                         # cooked assets and managed build output
  Dist/
    <version>/
      windows-x86_64/
      windows-x86_64-symbols/
      windows-x86_64.zip
      windows-x86_64.zip.sha256
      windows-x86_64-symbols.zip
      windows-x86_64-symbols.zip.sha256
```

Packaging stages into a unique temporary directory below `Dist`, validates and
smoke-tests the staged result, then moves the complete version directory into
place. An existing `Dist\<version>` is never overwritten.

Installed packaging rejects `-CargoTargetDir`, `-SkipBuild`, and `-AllowDirty`
because it uses the installation's prebuilt runtime, editor command host, asset
tooling, symbols, notices, managed SDK, and script host. Before use, every file
listed by `engine.installation.json` is SHA-256 verified; required tools and
all top-level script-host files must also be covered by the manifest.

The operations execute with the project workspace as their working directory:

- validate the project and every cataloged scene;
- for scripted projects, deploy the installed SDK/host and compile only the
  game-authored C# assembly;
- cook source assets through the installed editor's complete runtime asset
  registry into the project's `build/cooked`, then mirror that batch into
  package staging;
- copy the prebuilt player and matching PDB;
- run the packaged project for three headless frames; and
- emit manifests, checksums, runtime and symbol ZIPs.

No project content or game build output is written back into the engine
installation. Scripted authoring/package builds need a .NET 8 SDK, and the
framework-dependent script host needs a .NET 8 runtime on the target machine.
Non-scripted projects do not need .NET.

## Runtime package contents

The runtime package contains the executable, validated game project manifest,
every scene declared by the project, assets produced by the strict cooker,
configuration, project/run/release/asset/cook manifests, dependency notices,
and SHA-256 checksums. The MSVC PDB is published as a version-matched sidecar
symbol package. Neither package copies source assets, the editor, authoring
`.csproj` files, nor the per-machine `pso_cache`.

For scripted projects, packaging copies the game assembly and managed
dependencies to `scripts/`, copies the complete framework-dependent host to
`binaries/script-host/`, and requires the packaged smoke report to show
loaded/started script instances with zero script errors.

Packaging validates `GameProject-v0`. Installed mode uses `project cook` so
the cooker and packaged runtime share the same subsystem asset registry, then
records an `AssetCookReport-v0` summary while copying the verified project
batch into staging. Source mode runs the standalone `asset-cook` directly
against the configured `asset_source`. Manifest parse/version errors, unsafe
source paths, duplicate IDs, unsupported cook rules/types, and individual
cooker failures fail the package command. Source assets are never shipped
implicitly.

Projects may declare a scene catalog whose keys are portable scene IDs and
whose values are project-relative `.scene.ron` paths. `startup_scene` may name
one of those IDs (or the matching catalog path). The packager validates every
ID and path, rejects case-insensitive ID collisions on Windows, requires every
scene file to exist, and copies the complete catalog while preserving its
project-relative layout. Manifests without `scenes` remain supported as legacy
single-scene projects with the synthesized ID `main`.

`manifests/release.json` records the resolved scene catalog and, in installed
mode, the engine installation schema, version, and manifest SHA-256. The older
`startup_scene` and `startup_scene_sha256` fields remain as resolved-path
aliases for release-tool compatibility.

The packaged layout is:

```text
windows-x86_64/
  game.project.json
  binaries/sandbox.exe
  binaries/script-host/       # scripted projects only
  scripts/                    # scripted projects only
  assets/cooked/
  assets/scenes/*.scene.ron
  config/runtime.json
  manifests/project-check.json
  manifests/project-run.json
  manifests/asset-cook.json
  manifests/assets.json
  manifests/release.json
  manifests/NOTICES.txt
  checksums/SHA256SUMS.txt
```

ZIP entries are sorted and stamped with the engine source commit time recorded
in the installation manifest.

The packaged project player currently supports Vulkan only. The script rejects
`-Backend dx12` because DX12 does not yet provide the windowed project player
or a real windowed release smoke test.

## Assemble an engine installation (maintainers)

This step intentionally runs from the engine repository and requires the
pinned Rust toolchain, Cargo, Git, and .NET:

```powershell
Set-Location E:\project\engine
$env:RELEASE_VERSION = "v0.1.0-rc1"

.\.github\scripts\build-engine-installation.ps1 `
  -Version $env:RELEASE_VERSION
```

The script always rejects a dirty worktree. It builds the player/asset cooker
and editor into separate Cargo target directories, builds the managed SDK and
framework-dependent host once from engine-owned sources, creates notices and
provenance metadata, hashes every installation file, and atomically moves the
complete staged version directory into:

```text
artifacts/engine-installation/<version>/
  windows-x86_64/
    bin/EngineEditor.exe
    runtime/windows-x86_64/GameRuntime.exe
    runtime/windows-x86_64/GameRuntime.pdb
    tools/asset-cook.exe
    tools/package-windows.ps1
    sdk/EngineGameplay.dll
    sdk/script-host/*
    THIRD_PARTY_NOTICES.txt
    engine.installation.json
  engine.installation.json.sha256
```

An existing installation version is not overwritten. The assembly script is a
source-maintainer operation; installed game projects never run it.

## Source-tree project packaging (maintainers)

The original source mode remains useful for release engineering:

```powershell
E:\project\engine\.github\scripts\ci.ps1 -Task Rust
E:\project\engine\.github\scripts\qa.ps1 -Configuration Release
$env:RELEASE_VERSION = "v0.1.0-rc1"
E:\project\engine\.github\scripts\package-windows.ps1 `
  -ProjectPath E:\project\engine\examples\minimal-game\game.project.json
```

Without `-EngineInstallRoot`, the packager treats its repository as the engine
source root. It checks Git cleanliness, builds the runtime and cooker through
locked Cargo dependencies, gathers notices from Cargo metadata, and defaults
to `artifacts/release` rather than the external project's `Dist`. `-AllowDirty`
is only for local dry runs and records `dirty: true` in release metadata.

This mode is not evidence that installed projects need the repository. CI
separately creates a C# project under the runner's external temporary
directory, verifies SDK/host deployment and game-DLL compilation, restricts
`PATH` so Cargo/Git/Rust are unavailable, and exports it through the assembled
installed toolchain.

## Reproducibility

The stronger source release gate runs two builds from empty Cargo target
directories and compares both the final ZIP hash and every staged file hash:

```powershell
E:\project\engine\.github\scripts\verify-package-reproducibility.ps1 `
  -Version v0.1.0-rc1
```

It writes `PackageReproducibilityReport-v0` under
`artifacts/reproducibility/<release-id>/report.json`. Executable, cooked asset,
manifest, checksum, or ZIP metadata differences fail the release workflow.
Every sidecar manifest must also match the runtime executable and its PDB.

MSVC PDB stream layout is not byte-for-byte stable across otherwise identical
links, so the PDB is deliberately excluded from the deterministic runtime ZIP.
The report records both PDB hashes and whether they happen to match; a mismatch
is visible but does not weaken the runtime reproducibility gate. The stable
release ID and executable SHA-256 in `SymbolManifest-v0` prevent a symbol
sidecar from being paired with the wrong runtime.

## QA and diagnostics

`qa.ps1` retains the low-level `engine_scene::sample_scene` contract baseline
and also checks, cooks, and runs the configured game project through the normal
GameLoop without creating a window or GPU. It writes `QaReport-v0` and
`ProjectRunReport-v0`, and fails when lifecycle, draw-count, visibility, or
indexed-triangle checks fail.

A packaged runtime reads `config/runtime.json`, writes JSON-line logs under
`logs/`, and writes a release-tagged panic report before the normal panic hook
runs. Export a support bundle with:

```powershell
E:\project\engine\.github\scripts\export-diagnostics.ps1 `
  -PackageRoot D:\Games\MyGame\Dist\v0.1.0-rc1\windows-x86_64
```

The diagnostic bundle contains runtime logs, manifests, checksums, config, and
the validated sidecar symbol manifest with its PDB hash reference. Keep the
runtime ZIP, symbol ZIP, and diagnostic bundles together under the same release
ID.

## Rollback and current limitations

Release directories are immutable. To roll back, verify a prior ZIP against
its adjacent `.sha256`, extract it to a new directory, run
`binaries/sandbox.exe game game.project.json`, then atomically switch the
deployment pointer or launcher. Do not overwrite an existing version
directory.

Current limitations:

- Windows x86-64 is the only automated package target.
- SHA-256 integrity is implemented; Authenticode signing/provenance is not.
- The engine distribution is a relocatable directory; there is no
  installer/updater-level rollback automation yet.
- Headless CPU thresholds are enforced, but controlled-hardware GPU and process
  memory baselines are not yet collected.
- Panic reports and symbols are exported, but native minidump generation and a
  symbol-resolution smoke are not yet implemented.
