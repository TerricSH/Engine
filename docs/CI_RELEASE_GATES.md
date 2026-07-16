# CI and release gates

The required pull-request checks run on a GitHub-hosted Windows runner and do
not require a GPU or private credentials. The workflow also listens to both the
current `master` branch and a future `main` branch.

## Enforced checks

- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Windows feature wiring for Vulkan, DX12, editor, C# scripting, and gameplay.
- `cargo test --workspace --locked`.
- A fresh-directory project workflow that creates `GameProject-v0`, validates
  it, runs three GameLoop frames with visible draws, and cooks its asset set.
- The fixed `assets/models/resource-chain.gltf` resource-chain test, including
  its external buffer and texture, without creating a window or GPU device.
- A strict deterministic asset cook that emits `AssetCookReport-v0`, exercises
  the Mesh, Texture, Shader, Scene, and Logic cookers, rejects an empty fixture
  set, and fails on any invalid manifest/path/rule/type or cooker error.
- Headless sample-scene QA plus a real project check/cook/run QA with
  visible-draw and CPU-frame thresholds.
- A Release `engine_ffi.dll`, Release builds of every C# project, the native ABI
  rejection/acceptance smoke tests, the JSON-line script-host smoke, and the C#
  sandbox smoke.
- Fresh compilation of every Vulkan GLSL source plus `spirv-val` validation of
  both the fresh output and every checked-in `.spv` artifact.
- A locked Release build of the complete Rust workspace and the Vulkan/editor
  desktop sandbox feature set after all preceding jobs pass.
- Two independent clean-target Release builds whose staged files and final
  project-bearing Windows ZIP must have identical SHA-256 hashes.

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
