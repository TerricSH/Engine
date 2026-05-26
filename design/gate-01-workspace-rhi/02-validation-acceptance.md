# Gate 1 Validation And Acceptance

## Gate Exit Principle

Gate 1 is accepted only when the workspace skeleton, crate ownership boundaries, feature flags, and `RHI-v0` contract are stable enough for backend sessions to start without editing root workspace files.

## Verification Goals

- Prove the workspace can support multi-session development without root file conflicts.
- Prove `RHI-v0` is defined clearly enough for Vulkan, OpenGL, and DirectX 12 backend sessions to compile against it.
- Prove future subsystem crates exist as placeholders so later gates do not fight over workspace registration.

## Required Results

- Root workspace builds with all placeholder crates registered.
- `render-core` exposes the initial RHI contract: backend traits, device/surface/swapchain/resource handles, command model, descriptors, capabilities, and errors.
- `render-vulkan`, `render-opengl`, and `render-dx12` compile as stubs behind feature flags.
- Workspace ownership rules are documented in design docs.

## Acceptance Checklist

- [ ] Root `Cargo.toml` contains all planned workspace members.
- [ ] Feature flags exist for `backend-vulkan`, `backend-opengl`, `backend-dx12`, `editor`, `scripting-csharp`, `hot-reload`, and `mobile`.
- [ ] `render-core` has no Vulkan/OpenGL/DX12 concrete types in public engine-level APIs.
- [ ] Backend stubs compile without implementing real rendering.
- [ ] Future crates exist for ECS, serialization, assets, editor, scripting, hot update, physics, animation, and sandbox.
- [ ] Ownership rules define who may edit root workspace files and `render-core`.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo check --workspace --features backend-opengl`
- Windows only: `cargo check --workspace --features backend-dx12`

## Manual Validation

- Inspect `render-core` public API for backend leakage.
- Confirm new sessions can work inside their own crates without editing root workspace files.
- Confirm design docs name the frozen `RHI-v0` contract and known non-goals.

## Blocking Issues

- Root workspace does not compile.
- Backend stubs require editing `render-core` from backend sessions.
- RHI public API exposes concrete Vulkan/OpenGL/DX12 types.
- Workspace members are missing for planned parallel workstreams.

## Required Evidence

- Command outputs from automated checks.
- Short RHI contract review note.
- List of frozen feature flags and workspace members.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 2: yes/no

