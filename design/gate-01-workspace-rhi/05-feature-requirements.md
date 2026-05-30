# Gate 1 Feature Requirements And Execution Boundaries

## External Contracts Consumed

None. Gate 1 is the contract origin point. Sessions must comply with these cross-cutting documents but do not consume an upstream gate contract:

| Document | Why it constrains this gate |
|---|---|
| [data-schema-contracts.md](../data-schema-contracts.md) `RHI-v0` | Authoritative field list for descriptors, capabilities, handles, and errors. The Rust skeleton in `01-code-architecture.md` mirrors this section. |
| [compatibility-error-handling.md](../compatibility-error-handling.md) | Freeze semantics (Gate 1 owns `RHI-v0`) and `Diagnostic` envelope `RhiError` maps into. |
| [performance-budgets.md](../performance-budgets.md) Gate 1 row | Startup ≤ 300 ms, peak memory ≤ 64 MiB, no required native SDK for disabled backends. |

## Gate Objective

Create the Rust workspace, crate ownership boundaries, feature flags, and `RHI-v0` contract that all renderer backend work depends on. This gate is about contracts and compile boundaries, not real rendering.

## Required Features

### G1-F01 Workspace Skeleton

Required behavior:
- Create a Cargo workspace at the repository root.
- Register all planned crate directories up front so later sessions do not edit root workspace files.
- Add workspace-level dependency, lint, and formatting conventions if chosen.

Minimum output:
- Root workspace manifest.
- Placeholder crate manifests and minimal `lib.rs` or `main.rs` files.
- Workspace compiles with placeholder crates.

Do not overbuild:
- Do not implement subsystem logic inside placeholder crates.
- Do not add heavy dependencies before the owning gate needs them.

### G1-F02 Feature Flag Baseline

Required behavior:
- Define additive feature flags for backend and optional system families.
- Include at least `backend-vulkan`, `backend-opengl`, `backend-dx12`, `tooling-editor`, `subsystem-scripting-csharp`, `tooling-hot-reload`, and `target-mobile`.
- Keep default features minimal.

Minimum output:
- Feature definitions are documented.
- Enabling unrelated feature combinations does not create immediate conflicts.

Do not overbuild:
- Do not make mutually exclusive features that break under Cargo feature unification.
- Do not require DirectX SDK on non-Windows checks.

### G1-F03 RHI-v0 Core Contract

Required behavior:
- Define the first backend-neutral RHI contract in `render-core`, mirroring the Rust skeleton in [`01-code-architecture.md` → Initial RHI Surface](./01-code-architecture.md#initial-rhi-surface).
- Include types for backend kind, adapter info, backend capabilities, resource descriptors, errors, and opaque handles.
- Include initial concepts for device, queue, surface, swapchain, command encoder, render pass, shader module, pipeline, buffer, and texture.
- The public type names, descriptor field names, trait method signatures, and `RhiError` variant set in that skeleton are the frozen public surface of `RHI-v0`. Internal layouts may differ; the public API may not.

Minimum output:
- `render-core` public API compiles.
- API names are backend-neutral.
- Backend-specific objects are not exposed above backend crates.
- `RhiError` → `Diagnostic` mapping table (see `01-code-architecture.md` → Error Model) is preserved in code (e.g. as a `code()` method or doc-tested table).

Do not overbuild:
- Do not model advanced features such as bindless resources, async compute, ray tracing, mesh shaders, or transient resource aliasing.
- Do not add renderer scene concepts to RHI.

### G1-F04 Backend Compile Stubs

Required behavior:
- Add `render-vulkan`, `render-opengl`, and `render-dx12` crates as compile stubs.
- Stubs consume `render-core` and return explicit unsupported/unimplemented errors where needed.
- DirectX 12 code is Windows-gated.

Minimum output:
- Vulkan/OpenGL/DX12 backend crates compile behind their feature flags.
- Stubs validate that `RHI-v0` can be consumed without importing another backend.

Do not overbuild:
- Do not implement actual rendering in Gate 1.
- Do not let stubs evolve `render-core` independently.

### G1-F05 Ownership And Session Rules

Required behavior:
- Document which session owns root workspace files, `render-core`, and backend crates.
- Define that root workspace edits go through integration ownership.
- Define that backend sessions consume RHI and request contract changes through integration review.

Minimum output:
- Design docs clearly state ownership boundaries.
- Future AI sessions know which files are safe to edit.

## Target Effects

- Future sessions can work in isolated crates.
- Backend work can begin from a stable `RHI-v0`.
- OpenGL and DirectX 12 stubs expose early abstraction problems.

## Explicit Non-Goals

- No Vulkan rendering.
- No editor, scripting, ECS, asset pipeline, physics, animation, UI, audio, mobile, or hot update implementation.
- No production CI/CD setup beyond minimal compile commands.

## AI Execution Rules

- Keep this gate contract-focused.
- Do not fill placeholder crates with speculative code.
- Do not add backend-native types to public RHI contracts.
- Do not add dependencies that require unavailable platform SDKs unless feature-gated.

## Completion Signal

Gate 1 is complete when the workspace and all placeholder crates compile, `RHI-v0` is documented, backend stubs compile behind feature flags, and ownership boundaries are clear enough for parallel sessions.
