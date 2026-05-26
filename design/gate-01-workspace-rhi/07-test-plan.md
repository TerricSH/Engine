# Gate 1 Test Plan

## Test Strategy

Gate 1 tests prove that the workspace, feature flags, RHI contract, and backend stubs are usable by parallel sessions. Most tests are compile-time, contract, and documentation checks.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result (concrete) |
|---|---|---|---|
| G1-F01 Workspace Skeleton | `cargo metadata --no-deps` membership check | Contract | `packages[].name` set equals the 15-crate list in IT-G1-01 exactly |
| G1-F01 Workspace Skeleton | `cargo check --workspace` with default features | Compile | Exit code 0, no `error:` lines in stderr |
| G1-F02 Feature Flag Baseline | Independent `--features backend-{vulkan,opengl,dx12}` checks | Compile | Each: exit 0; `backend-dx12` builds only on Windows or behind cfg gate |
| G1-F02 Feature Flag Baseline | Multi-feature `--features backend-vulkan,backend-opengl` | Compile | Exit 0; package set identical (modulo feature flags) to union of singles |
| G1-F03 RHI-v0 Core Contract | Dummy backend implementation under `crates/render-core/tests/` | Contract | Test compiles and runs; uses no `ash::`, `gl::`, `d3d12::` paths |
| G1-F03 RHI-v0 Core Contract | Public API scan via `rg` patterns in IT-G1-03 | Review | `0 matches` for every leak pattern |
| G1-F03 RHI-v0 Core Contract | `RhiError → Diagnostic.code` mapping table | Doc-test | Every variant in the skeleton's error table is reachable from a `code()` or const map |
| G1-F04 Backend Compile Stubs | Instantiate stub `Backend::enumerate_adapters` and `create_device` | Unit | Returns `Err(RhiError::UnsupportedBackend)` or `Err(RhiError::Backend{detail:"unimplemented"})` — never panics |
| G1-F05 Ownership Rules | Grep design docs for the strings "root Cargo.toml" and "render-core ownership" | Review | Both phrases appear and identify owning session |

## Gate Integration Tests

1. Fresh workspace bootstrap test
   - Start from a clean checkout.
   - Run formatting and workspace compile checks.
   - Confirm all placeholder crates compile together.
2. Backend contract integration test
   - Compile Vulkan, OpenGL, and DirectX 12 features independently.
   - Compile Vulkan + OpenGL together.
   - On Windows, compile Vulkan + DirectX 12 together.
3. Parallel-session safety test
   - Simulate a backend session editing only its backend crate.
   - Confirm root files and `render-core` are untouched.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace`
- `cargo check --workspace --features backend-vulkan`
- `cargo check --workspace --features backend-opengl`
- Windows: `cargo check --workspace --features backend-dx12`

## Failure Criteria

- Any backend feature requires unrelated platform SDKs without cfg gates.
- Any public RHI type exposes backend-native objects.
- Any placeholder crate contains speculative subsystem implementation.

## Test Fixtures

- Workspace root with all planned crate directories present.
- Minimal backend stub implementations for Vulkan, OpenGL, and DirectX 12.
- A small compile-only dummy backend module under tests or examples, if needed, to prove `render-core` can be consumed externally.

## Executable Integration Cases

### IT-G1-01 Workspace Bootstrap

Setup:
- Clean checkout with no generated `target/` assumptions.

Steps:
1. Run `cargo metadata --no-deps --format-version=1 > target/test-evidence/gate-01/workspace-metadata.json`.
2. Parse `packages[].name` and verify the set is **exactly** equal to:
   ```
   engine-core, platform, render-core, render-vulkan, render-opengl,
   render-dx12, engine-scene, engine-serialize, engine-asset, engine-editor,
   engine-script, engine-hot-update, engine-physics, engine-animation,
   engine-renderer, engine-audio, engine-ui, engine-nav, engine-character,
   sandbox
   ```
   (15 crates, order-insensitive). Extra or missing names are a failure.
3. Run `cargo check --workspace` and capture stdout+stderr.

Expected (concrete assertions):
- `cargo metadata` exit code = `0`.
- The 15-crate set above matches with zero deltas.
- `cargo check --workspace` exit code = `0` and stderr contains no line matching the regex `error(\[E\d+\])?:` (warnings are allowed at Gate 1 but logged).
- No line in stderr matches `(SDK|sdk).*not.*found` or `linking with .* failed` for default-features builds.

Evidence:
- `target/test-evidence/gate-01/workspace-metadata.json`.
- `target/test-evidence/gate-01/cargo-check-default.log`.

### IT-G1-02 Backend Feature Matrix

Setup:
- Use the same clean workspace.

Steps:
1. Run, capturing logs to `target/test-evidence/gate-01/feature-<feature>.log`:
   - `cargo check --workspace --features backend-vulkan`
   - `cargo check --workspace --features backend-opengl`
   - `cargo check --workspace --features backend-vulkan,backend-opengl`
2. On Windows additionally run:
   - `cargo check --workspace --features backend-dx12`
   - `cargo check --workspace --features backend-vulkan,backend-dx12`
3. On non-Windows additionally run `cargo check --workspace --features backend-dx12` and confirm it either compiles as a no-op stub OR fails *only* with a clear cfg gate diagnostic (no missing-SDK error).

Expected (concrete assertions):
- Every command on its target OS: exit code = `0`.
- No stderr line matches `error(\[E\d+\])?:`.
- No stderr line contains `vulkan-sdk`, `directx-sdk`, `LIBCLANG`, or any SDK-not-found pattern when only `backend-opengl` is enabled (i.e. disabling a backend must disable its SDK requirement).
- Multi-feature combination produces identical crate graph as the union of individual features (verified by `cargo metadata --features ...` returning the same package set, minus feature flags).

Evidence:
- All `feature-*.log` files under `target/test-evidence/gate-01/`.
- A short `target/test-evidence/gate-01/feature-matrix-summary.md` listing per-row exit code + duration.

### IT-G1-03 RHI Leakage Review

Setup:
- The `render-core` crate is built and `cargo doc -p render-core --no-deps` has been run.

Steps:
1. Search the public source surface of `render-core` for backend-native type references. Treat any of the following patterns in `crates/render-core/src/**/*.rs` outside `#[cfg(test)]` and outside doc comments as a leak (use `rg --no-heading --line-number`):
   ```
   ash::                # Vulkan binding
   vk::                 # Vulkan namespace
   gl::                 # OpenGL binding
   windows::Win32::Graphics::Direct3D12
   ID3D12                # D3D12 COM types
   HRESULT
   GLuint|GLint|GLenum  # GL handle aliases
   ```
2. Confirm `cargo doc` output for `render-core` mentions zero of the above identifiers in public items (parse `target/doc/render_core/all.html` or use `cargo-public-api` if available).
3. Walk every `pub trait`, `pub struct`, `pub enum`, `pub fn`, and `pub type` in `render-core` and verify each one appears in the Rust skeleton table in [`01-code-architecture.md` → Initial RHI Surface](./01-code-architecture.md#initial-rhi-surface), or is a clearly-documented support type.

Expected (concrete assertions):
- `rg` produces **zero matches** for each of the listed patterns inside non-test, non-doc-comment lines.
- The `cargo-public-api` diff (if used) shows no symbols whose path contains `ash`, `vk`, `gl::`, `d3d12`, or `windows::`.
- Every public symbol is traceable to the architecture skeleton or to one of: `RhiError`, `BackendKind`, `BackendCapabilities`, `ResourceLimits`, `ValidationMode`, `PresentMode`, `MemoryHint`, `ShaderFormat`, `TextureFormat`, `BufferUsage`, `TextureUsage`, `ResourceHandle<_>`, `Backend`, `Device`.

Evidence:
- `target/test-evidence/gate-01/rhi-leakage-rg.log` (must contain a line `0 matches` per pattern).
- `target/test-evidence/gate-01/rhi-public-api.txt` (output of `cargo public-api` or manual export).
- Short review note at `target/test-evidence/gate-01/rhi-review.md` linking each public symbol to the architecture skeleton.
