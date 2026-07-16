# Windows release packaging

The Windows release path produces a versioned, checksumed runtime package from
a clean checkout. Rust `1.94.1` is pinned by `rust-toolchain.toml`; Cargo uses
the committed lockfile. The shipping sandbox excludes editor tooling and only
enables the C# runtime feature when the selected project declares scripts.

## Local release candidate

From any PowerShell working directory:

```powershell
E:\project\engine\.github\scripts\ci.ps1 -Task Rust
E:\project\engine\.github\scripts\qa.ps1 -Configuration Release
$env:RELEASE_VERSION = "v0.1.0-rc1"
E:\project\engine\.github\scripts\package-windows.ps1 `
  -ProjectPath E:\project\engine\examples\minimal-game\game.project.json
```

Packaging rejects a dirty worktree. `-AllowDirty` exists only for local dry
runs and records `dirty: true` in `ReleaseMetadata-v0`; CI never uses it.

The packaged project player currently supports Vulkan only. The scripts reject
`-Backend dx12` up front because DX12 does not yet provide the windowed project
player or a real windowed release smoke test; accepting it would produce a
package that passes headless QA but cannot launch a game window.

The runtime package contains the executable, validated game project manifest,
every scene declared by the project, assets produced by the strict cooker, configuration,
project/run/release/asset/cook manifests, dependency notices, and SHA-256
checksums. The MSVC PDB is published as a version-matched sidecar symbol
package. Neither package copies source assets, the editor, or the per-machine
`pso_cache`.

For scripted projects, packaging runs `project build-scripts`, copies the game
assembly and managed dependencies to `scripts/`, copies the complete
framework-dependent host publish to `binaries/script-host/`, removes the
authoring `.csproj` path from the staged manifest, and requires the packaged
smoke report to show loaded/started script instances with zero script errors.
This host currently requires the .NET 8 runtime on the target machine.

Packaging validates `GameProject-v0`, then builds and runs `asset-cook` from the
project's configured `asset_source` directly into the staging directory.
Manifest parse/version errors, unsafe source paths, duplicate IDs, unsupported
cook rules/types, and individual cooker failures all produce
`AssetCookReport-v0` and fail the package command. Source assets are never
shipped implicitly.

Projects may declare a scene catalog whose keys are portable scene IDs and
whose values are project-relative `.scene.ron` paths. `startup_scene` may name
one of those IDs (or the matching catalog path). The packager validates every
ID and path, rejects case-insensitive ID collisions on Windows, requires every
scene file to exist, and copies the complete catalog while preserving its
project-relative layout. Manifests without `scenes` remain supported as legacy
single-scene projects with the synthesized ID `main`.

`manifests/release.json` records `startup_scene_id`,
`startup_scene_path`, and a deterministic `scenes` array containing each ID,
path, and SHA-256. The older `startup_scene` and `startup_scene_sha256` fields
remain as resolved-path aliases for release-tool compatibility.

`examples/minimal-game/assets/source/game.manifest` is the default release
project's minimum content set. It cooks one Mesh, Texture, Material, Shader,
Scene, and Logic asset, so CI also rejects an accidentally empty or partially
successful cook.

## Artifact layout

```text
artifacts/release/<release-id>/
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
  windows-x86_64-symbols/
    sandbox.pdb
    symbols.json
  windows-x86_64.zip
  windows-x86_64.zip.sha256
  windows-x86_64-symbols.zip
  windows-x86_64-symbols.zip.sha256
```

ZIP entries are sorted and stamped with the source commit time. Running the
packager twice without rebuilding must produce the same archive SHA-256.

The stronger release gate runs two builds from empty Cargo target directories
and compares both the final ZIP hash and every staged file hash:

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
  -PackageRoot E:\project\engine\artifacts\release\v0.1.0-rc1\windows-x86_64
```

The diagnostic bundle contains runtime logs, manifests, checksums, config, and
the validated sidecar symbol manifest with its PDB hash reference. Keep the
runtime ZIP, symbol ZIP, and diagnostic bundles together under the same release
ID.

## Rollback

Release directories are immutable. To roll back, verify the prior ZIP against
its adjacent `.sha256`, extract it to a new directory, run
`binaries/sandbox.exe game game.project.json`, then atomically switch the deployment pointer
or launcher to that directory. Do not overwrite an existing version directory.

## Remaining release limitations

- Windows x86-64 is the only automated package target.
- SHA-256 integrity is implemented; Authenticode signing/provenance is not.
- Headless CPU thresholds are enforced, but controlled-hardware GPU and process
  memory baselines are not yet collected.
- Panic reports and symbols are exported, but native minidump generation and a
  symbol-resolution smoke are not yet implemented.
- There is no installer/updater-level rollback automation yet.
