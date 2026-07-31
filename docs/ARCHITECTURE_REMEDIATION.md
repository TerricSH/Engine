# Architecture remediation

This document records the ownership rules enforced after the engine-wide
architecture audit. It separates measured defects from name-only similarities
so that future clean-ups do not merge unrelated domain models.

## Composition roots

- `engine-core` composes subsystem lifetimes and exposes backend-neutral runtime
  operations. Its root and game-loop files are facades; domain work lives in
  owned child modules or subsystem crates.
- `sandbox` is the executable/application composition root. Editor, project CLI,
  and managed-script tooling are grouped behind bounded facade files.
- Native graphics device creation belongs to each backend crate. Applications
  select a backend and install its `BackendRenderer`; `engine-core` never
  imports Vulkan, DX12, winit, Tao, or raw native handles.

The architecture test suite enforces facade line/cfg budgets, required module
boundaries, dependency allowlists, and the absence of commented-out Rust
implementations.

Every production Rust source file is also held below a hard 1,000-line
giant-file threshold. The audit's named composition files use tighter facade
budgets, while large test suites are split by contract domain. This is a
ratchet: new work must create or extend an owned module instead of growing a
new monolith.

Managed scripting owns one feature-gated `ScriptRuntimeState` inside
`EngineRuntime`. Host state, input snapshots, deferred commands, queries, and
result queues no longer add one conditional field apiece to the composition
object.

## Canonical type ownership

| Contract | Canonical owner | Compatibility surface |
| --- | --- | --- |
| `IndexFormat` | `render-core` | re-exported by `engine-renderer` |
| RHI `ShaderStage`, `TextureFormat`, `VertexLayout`, `VertexAttribute` | `render-core` | consumed directly by concrete backends |
| typed mesh-binding layout | `engine-renderer` (`MeshVertexLayout`) | legacy `VertexLayout` name is a compatibility alias |
| `LightKind` | `engine-serialize` | re-exported by scene/renderer APIs |
| logical asset schema | `engine-serialize` | asset cooking consumes and re-exports the schema |
| vertex/render extraction types | `engine-renderer` | backend crates consume them |
| asset texture source format | `engine-asset` | converted to the renderer/RHI format at the asset boundary |

Similar names are not automatically duplicates. Combat damage and physics
destruction damage have different invariants; asset-cook and navmesh-bake
errors have different recovery paths; editor performance snapshots are a UI
projection of renderer statistics; animation asset data and runtime skeleton
state have different ownership and mutation rules. Those contracts use
domain-qualified canonical names instead of being incorrectly collapsed.

## Rendering policy

Backend-neutral frame policy lives in `engine-renderer::backend_shared`:
environment selection, UI preparation, extraction statistics, frame planning,
and tone-map parameters are computed once. Vulkan and DX12 retain only
API-specific resource ownership, synchronization, descriptor/pipeline setup,
command recording, submission, and presentation.

The shared tone-map plan also prevents backend drift: exposure, bloom, colour
grading, vignette, and planetary-lens parameters have one CPU-side contract.

## Platform boundaries

Direct `winit` access is owned by `platform` and is not re-exported.
Applications receive only `PlatformWindow`, normalized platform events, and an
opaque `PlatformSurface`. A concrete backend consumes the platform-owned
`PlatformSurfaceSnapshot` adapter; `sandbox` has no direct winit or
raw-window-handle dependency.

Tao and raw native handles remain intentionally confined to
`engine-editor-host`, because that native adapter owns the Wry/Tao window and,
on Linux, the GTK child surface. The host converts that native surface to the
same opaque platform contract before it reaches editor application code. Raw
handles are otherwise allowed only inside `platform` and concrete graphics
backend crates, and are forbidden in ordinary subsystems and `engine-core`.

The project player facade declares `run`, `transitions`, `assets`, `headless`,
and `windowed` as real Rust modules. Textual `include!` assembly is not used for
production ProjectApp behavior, so privacy and dependency boundaries are
checked by the compiler.

## Dead code policy

Production implementations may not be hidden with dead-code attributes in the
guarded engine/editor/Vulkan paths, and commented-out Rust items are rejected by
an architecture test. Test-only helpers are compiled with `#[cfg(test)]`.

The former unreferenced renderer shader compiler was removed. The public
ILRuntime host is no longer a permanent stub: it loads a native bridge, resolves
its documented C ABI exports, retains the library for the lifetime of all
instances, and destroys instances before unloading.

CI compiles and validates every checked-in Vulkan SPIR-V artifact, including
the macro-generated VFX fragment variant. Managed CI also executes the real
project script lifecycle with the physics component registry enabled, rather
than only compiling the standalone managed projects.
