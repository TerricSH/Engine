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

- [x] Root `Cargo.toml` contains all planned workspace members.
- [x] Feature flags exist for `backend-vulkan`, `backend-opengl`, `backend-dx12`, `tooling-editor`, `subsystem-scripting-csharp`, `tooling-hot-reload`, and `target-mobile`.
- [x] `render-core` has no Vulkan/OpenGL/DX12 concrete types in public engine-level APIs.
- [x] Backend stubs compile without implementing real rendering.
- [x] Future crates exist for ECS, serialization, assets, editor, scripting, hot update, physics, animation, and sandbox.
- [x] Ownership rules define who may edit root workspace files and `render-core`.

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

- Command outputs captured in the implementation session:
	- `cargo metadata --no-deps --format-version=1` reported 20 packages.
	- `cargo fmt --all --check` passed.
	- `cargo check --workspace` passed.
	- `cargo check --workspace --features backend-vulkan` passed.
	- `cargo check --workspace --features backend-opengl` passed.
	- `cargo check --workspace --features backend-dx12` passed on Windows without SDK linkage.
	- `cargo test --workspace` passed.
	- `cargo run -p sandbox -- gate04-scene` passed the ECS-to-renderer contract smoke path.
- RHI contract review note: `render-core` exposes backend-neutral traits, descriptors, generational typed handles, and `RhiError::code()` mappings only; backend crates contain no native SDK dependency yet.
- Frozen workspace members: `platform`, `render-core`, `render-vulkan`, `render-opengl`, `render-dx12`, `engine-core`, `engine-serialize`, `engine-renderer`, `engine-scene`, `engine-asset`, `engine-script`, `engine-editor`, `engine-hot-update`, `engine-physics`, `engine-animation`, `engine-audio`, `engine-ui`, `engine-nav`, `engine-character`, `sandbox`.

## Exit Decision

- Gate owner: Copilot implementation session
- Date: 2026-05-26
- Approved to proceed to Gate 2: yes for contract/backend implementation work; Gate 2 still must replace the Vulkan stub with a real backend and collect its own performance evidence.

