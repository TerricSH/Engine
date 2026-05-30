# Gate 1 Session Prompts

Use these prompts to launch isolated coding sessions for Gate 1. Every session must read `README.md`, `01-code-architecture.md`, `02-validation-acceptance.md`, `03-best-practices.md`, and `05-feature-requirements.md` before editing.

## Session 1A: Workspace Integration Owner

Goal: Create the workspace skeleton and shared configuration.

Owns:
- Root `Cargo.toml`
- Root formatting/lint config
- Crate registration

Must not edit:
- Backend implementation internals beyond minimal stubs
- Any future subsystem logic

Expected output:
- All planned crates registered
- Additive feature flags created
- Workspace compiles with placeholder crates

Validation:
- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo check --workspace --features backend-opengl`
- Windows: `cargo check --workspace --features backend-dx12`

Merge checklist:
- Root ownership rules documented
- No heavy dependencies added unnecessarily
- Placeholder crates contain no speculative logic

## Session 1B: RHI Contract Owner

Goal: Define `RHI-v0` in `render-core`.

Owns:
- `crates/render-core`

Must not edit:
- `render-vulkan` beyond compile-driven interface checks
- `engine-core` public renderer facade beyond minimal placeholder needs

Expected output:
- Backend-neutral descriptors, handles, capabilities, and errors
- No backend-native public types
- Compile-friendly backend contract

Validation:
- Check all backend stubs compile against `render-core`
- Manual API review for Vulkan/OpenGL/DX12 type leakage

Merge checklist:
- Contract is small enough for Gate 2
- Unsupported features can be represented explicitly

## Session 1C: Backend Stub Owner

Goal: Add minimal backend crates that consume `RHI-v0`.

Owns:
- `crates/render-vulkan`
- `crates/render-opengl`
- `crates/render-dx12`

Must not edit:
- `crates/render-core` except through integration request
- Future subsystem crates

Expected output:
- Stub constructors and explicit unsupported/unimplemented errors
- Windows cfg for DirectX 12

Validation:
- Feature-gated backend compile checks

Merge checklist:
- Stubs do not evolve RHI independently
