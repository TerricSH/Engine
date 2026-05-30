# Implementation Review - 2026-05-26

## Current Implementation State

- Gate 1 is implemented as a compile-validated Rust workspace with the 20 crates frozen by `FD-029`.
- `render-core` owns the first `RHI-v0` public surface: backend-neutral descriptors, capabilities, generational typed handles, backend traits, and `RhiError::code()` mappings.
- `render-vulkan`, `render-opengl`, and `render-dx12` are Gate 1 compile stubs only. Gate 2 still owns real Vulkan instance/device/swapchain/rendering work.
- `engine-renderer` contains a thin `RendererInput-v0.2.0` contract skeleton and validates version, view presence, duplicate view IDs, and overlay-base references.
- `engine-scene` contains a thin `ECSScene-v0.1.0` logical scene model, scene validation, and a sample ECS-to-renderer extraction path.
- `sandbox gate04-scene` proves the first vertical contract path: sample scene -> `RendererInput-v0` -> renderer validation.

## Validation Evidence

- `cargo metadata --no-deps --format-version=1`: 20 packages.
- `cargo fmt --all --check`: pass.
- `cargo check --workspace`: pass.
- `cargo check --workspace --features backend-vulkan`: pass.
- `cargo check --workspace --features backend-opengl`: pass.
- `cargo check --workspace --features backend-dx12`: pass on Windows without native SDK dependency.
- `cargo test --workspace`: pass.
- `cargo run -p sandbox -- gate04-scene`: pass.

## Design Review

The overall gate plan remains sound: the contract-first split is helping. Gate 1 can now support parallel backend and runtime work without changing root workspace membership. The most important improvement made during implementation was aligning feature names with `FD-010`; the old bare `editor`, `hot-reload`, `mobile`, and `scripting-csharp` names would have created command drift across later gates.

The main design risk is still scope size, not architecture direction. Gate 2 should focus only on turning `render-vulkan` from stub to real Vulkan backend. Gate 3 and Gate 4 already have thin contract scaffolds, but they are not complete gate implementations yet: renderer graph, actual draw submission, real ECS storage/query APIs, serialization fixtures, and deterministic replay remain to be built.

## Next Risks To Resolve

- Gate 2 must introduce real Vulkan carefully without leaking `ash`/Vulkan handles into `render-core`.
- The diagnostic registry must stay synchronized with code; new `RV0012`-`RV0014` and `SC0015`-`SC0018` were registered in this pass.
- Peak-memory measurement for short-lived Windows commands still needs a reliable harness.
- Gate 5+ still has open foundation questions (`OFQ-002`, `OFQ-003`, `OFQ-004`) that should be resolved before implementation reaches those gates.