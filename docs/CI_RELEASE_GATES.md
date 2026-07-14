# CI and release gates

The required pull-request checks run on a GitHub-hosted Windows runner and do
not require a GPU or private credentials. The workflow also listens to both the
current `master` branch and a future `main` branch.

## Enforced checks

- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Windows feature wiring for Vulkan, DX12, editor, C# scripting, and gameplay.
- `cargo test --workspace --locked`.
- The fixed `assets/models/resource-chain.gltf` resource-chain test, including
  its external buffer and texture, without creating a window or GPU device.
- A Release `engine_ffi.dll`, Release builds of every C# project, the native ABI
  rejection/acceptance smoke tests, the JSON-line script-host smoke, and the C#
  sandbox smoke.
- Fresh compilation of every Vulkan GLSL source plus `spirv-val` validation of
  both the fresh output and every checked-in `.spv` artifact.
- A locked Release build of the complete Rust workspace and the Vulkan/editor
  desktop sandbox feature set after all preceding jobs pass.

## Running the same gates locally

The script resolves the repository from its own location rather than the
caller's current directory. These examples are from the repository root; from
elsewhere, invoke the same file by its absolute path:

```powershell
./.github/scripts/ci.ps1 -Task Rust
./.github/scripts/ci.ps1 -Task Managed
./.github/scripts/ci.ps1 -Task Shaders
./.github/scripts/ci.ps1 -Task Release
```

`-Task All` runs the four groups in fail-fast order. Managed checks require
.NET 8 on Windows. Shader checks require `glslangValidator` and `spirv-val`,
normally provided by the Vulkan SDK. CI installs a pinned tool version. The
managed task gives Cargo and .NET a unique artifacts directory and removes it
afterward, so concurrent or abandoned test processes cannot lock the build
being verified.

## Release boundary still not closed

The `release-build` job proves that locked source builds, but it is not a
shipping package. Gate 19 remains open until the repository has all of the
following:

- deterministic asset cooking and a packaged game that does not read source
  assets;
- a tag-triggered Windows package (and agreed additional platforms), release
  metadata, third-party notices, checksums, and signing/provenance;
- archived PDBs/symbols and a crash-diagnostic symbol-resolution smoke test;
- two-build reproducibility comparison of the distributable artifacts;
- headless scene QA, a real-GPU validation run on controlled hardware, and
  performance regression thresholds;
- installation/launch and rollback validation for the final package format.
