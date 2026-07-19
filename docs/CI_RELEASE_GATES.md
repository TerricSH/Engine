# CI and release gates

The required pull-request checks run on GitHub-hosted Windows and Ubuntu
runners and do not require a GPU or private credentials. The workflow also
listens to both the current `master` branch and a future `main` branch.

## Enforced checks

- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Windows feature wiring for Vulkan, DX12, editor, C# scripting, and gameplay.
- `cargo test --workspace --locked`.
- Linux core gates (`rust-quality-linux`, ubuntu-latest): format check,
  `cargo clippy --workspace --exclude engine-editor-host --all-targets
  --locked -- -D warnings`, and `cargo test --workspace --exclude
  engine-editor-host --locked`. Everything except `engine-editor-host` builds
  and tests headlessly on Linux; the editor-host crate is excluded because it
  requires wry/tao + webkit2gtk. Vulkan device tests, DirectX, the editor
  feature set, and the managed (C#) gates remain Windows-only.
- A fresh-directory project workflow that creates `GameProject-v0`, validates
  it, runs three GameLoop frames with visible draws, and cooks its asset set.
- The fixed `assets/models/resource-chain.gltf` resource-chain test, including
  its external buffer and texture, without creating a window or GPU device.
- A strict deterministic asset cook that emits `AssetCookReport-v0`, exercises
  the Mesh, Texture, Shader, Scene, and Logic cookers, rejects an empty fixture
  set, and fails on any invalid manifest/path/rule/type or cooker error.
- Headless sample-scene QA plus a real project check/cook/run QA with
  visible-draw and CPU-frame thresholds.
- A soak/stability run (`crates/sandbox/tests/soak.rs`, part of the workspace
  test suite on both platforms): a scripted camera patrols a triangle-wave
  path that crosses the world-origin shift threshold repeatedly while
  world-partition cells stream in and out and background cooked-asset decodes
  are in flight. The gate asserts no streaming/script errors, an entity- and
  asset-count plateau after warmup, a process working-set plateau (growth
  regression bound, not an absolute budget), and a frame-time p95 regression
  tripwire from the rolling `FrameTimingSummary`. The structured
  `SoakReport-v0` JSON lands at `target/soak/soak-report.json` and is archived
  as a CI artifact. `SOAK_FRAMES` overrides the frame count (default 2048,
  seconds of wall time); `SOAK_REPORT` overrides the report path. Longer
  runs are local/scheduled opt-ins via the env var, never the PR default.
- A determinism gate (same file): the scripted soak scenario runs twice with
  a fixed seed and frame count and must produce byte-identical state —
  entity positions, origin-shift/cell-stream event sequences, and asset
  states — with timing and memory fields excluded from the compared digest.
- A Release `engine_ffi.dll`, Release builds of every C# project, the native ABI
  rejection/acceptance smoke tests, the JSON-line script-host smoke, and the C#
  sandbox smoke.
- Fresh compilation of every Vulkan GLSL source plus `spirv-val` validation of
  both the fresh output and every checked-in `.spv` artifact.
- A locked Release build of the complete Rust workspace and the Vulkan/editor
  desktop sandbox feature set after all preceding jobs pass.
- Two independent clean-target Release builds whose staged files and final
  project-bearing Windows ZIP must have identical SHA-256 hashes.

## Environment capability policy

Tests must never conflate code correctness with environment contents:

- A test that only needs *some* child process spawns an in-repo helper — the
  current test executable re-invoked in a helper mode (see
  `child_process_helper_prints_marker_and_exits` in
  `crates/sandbox/src/editor_build_ops.rs`) — never a system tool such as
  PowerShell or `/bin/sh`.
- A test that genuinely exercises a system tool (`dotnet`, PowerShell, shader
  compilers) probes for it first and skips with a clear
  `SKIP (missing capability)` marker when it is absent. The shared probe
  lives in `crates/sandbox/tests/common/mod.rs` (`require_tool` /
  `tool_on_path`); the crate's unit tests include the same file via a
  `#[path]` module in `main.rs`, so unit and integration tests share one
  mechanism. The C# workflow tests (`subsystem-scripting-csharp`) apply this
  probe for `dotnet`.

## Toolchain and determinism pinning

Deterministic outputs (asset cook reports, soak digests, reproducible
packages) depend on pinned toolchains and algorithms:

- Rust toolchain: `rust-toolchain.toml` pins `1.94.1` (clippy, rustfmt); CI
  installs the same version on both platforms.
- Shader toolchain: the shader gate installs Vulkan SDK `1.3.296.0`
  (`glslangValidator`, `spirv-val`); in-crate shader translation uses `naga
  0.20.0` from the locked dependency graph.
- Math/physics/noise: `glam 0.27.0`, `rapier3d 0.22` — version-locked via
  `Cargo.lock` (`--locked` on every CI cargo invocation); upgrading either
  requires re-validating the determinism gates.
- Managed interop: .NET `8.0.x` for the C# gates.
- The soak determinism digest (`crates/sandbox/tests/soak.rs`) records the
  fixed scenario seed in its report; any intentional change to the scenario
  or to a pinned algorithm must note the expected hash change in the PR.

## Running the same gates locally

The script resolves the repository from its own location rather than the
caller's current directory. These examples are from the repository root; from
elsewhere, invoke the same file by its absolute path:

```powershell
./.github/scripts/ci.ps1 -Task Rust
./.github/scripts/ci.ps1 -Task ProjectWorkflow
./.github/scripts/ci.ps1 -Task Managed
./.github/scripts/ci.ps1 -Task Shaders
./.github/scripts/ci.ps1 -Task Release
./.github/scripts/ci.ps1 -Task Repro
```

`-Task All` runs the four groups in fail-fast order. Managed checks require
.NET 8 on Windows. Shader checks require `glslangValidator` and `spirv-val`,
normally provided by the Vulkan SDK. CI installs a pinned tool version. The
managed task gives Cargo and .NET a unique artifacts directory and removes it
afterward, so concurrent or abandoned test processes cannot lock the build
being verified.

The Linux gates run the same locked toolchain:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude engine-editor-host --all-targets --locked -- -D warnings
cargo test --workspace --exclude engine-editor-host --locked
```

A longer local soak (report at `target/soak/soak-report.json`):

```bash
SOAK_FRAMES=50000 cargo test -p sandbox --test soak --locked
```

## Release boundary still not closed

The tag-triggered release workflow now produces a deterministic Windows
runtime ZIP with metadata, dependency notices, checksums, a separately linked
PDB symbol bundle, a packaged headless startup smoke, a headless scene QA
report, and exportable structured logs. Gate 19 remains open until the
repository also has all of the following:

- agreed non-Windows platform packages and signing/provenance;
- native minidumps and a symbol-resolution smoke test;
- a real-GPU validation run plus controlled-hardware GPU and process-memory
  regression thresholds;
- installation/launch and rollback validation for the final package format.
