# Gate 1: Workspace And RHI Foundation

## Purpose

Create the project skeleton and freeze the first rendering hardware interface contract. This gate prevents later sessions from fighting over root workspace files or inventing incompatible backend APIs.

## Entry Sync Point

- Workspace is empty or clean.
- One integration session owns root workspace files.

## Parallel Workstreams

1. Workspace Skeleton
   - Owns root `Cargo.toml`, shared dependencies, feature flags, lint/format config, and crate registration.
   - Creates placeholder crates as defined by `FD-029` in [foundation-decisions.md](../foundation-decisions.md#fd-029-workspace-crate-layout): `platform`, `engine-core`, `render-core`, `render-vulkan`, `render-opengl`, `render-dx12`, `engine-serialize`, `sandbox` (implementation-active in Gate 1) plus `engine-scene`, `engine-renderer`, `engine-asset`, `engine-script`, `engine-editor`, `engine-hot-update`, `engine-physics`, `engine-animation`, `engine-audio`, `engine-ui`, `engine-nav`, `engine-character` (placeholders for future gates). All 20 crates must exist in the workspace at the end of Gate 1.
2. RHI Contract
   - Owns `crates/render-core`.
   - Defines backend traits, resource handles, descriptors, errors, and capability queries.
3. Backend Stub Shells
   - Owns minimal compile shells for Vulkan, OpenGL, and DirectX 12.
   - OpenGL and DirectX 12 consume `render-core`; they do not shape it.

## Contracts To Freeze

- `RHI-v0`
- Backend feature flags
- Workspace crate layout
- Root ownership rules

## Exit Condition

- Workspace builds.
- `RHI-v0` is documented and frozen.
- Vulkan/OpenGL/DX12 stubs compile behind feature flags.

## Parallel Safety Notes

- No non-integration session edits root `Cargo.toml`.
- No backend implementation starts before `RHI-v0` exists.
- Any needed RHI change is routed to the RHI owner.
