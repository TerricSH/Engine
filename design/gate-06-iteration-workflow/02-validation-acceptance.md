# Gate 6 Validation And Acceptance

## Gate Exit Principle

Gate 6 is accepted only when the running sandbox can update textures, materials, shaders, and scenes safely enough for fast development iteration.

## Verification Goals

- Prove developers can iterate on textures, materials, shaders, and scenes without restarting the sandbox.
- Prove reloads happen safely at frame boundaries and preserve the last valid asset on failure.
- Prove editor asset assignment and C# script diagnostics work against frozen Gate 5 contracts.

## Required Results

- File watcher or explicit reload command detects changed assets.
- Incremental recook selects changed assets and reverse dependencies.
- Runtime reload queue applies updates at safe frame boundaries.
- Texture, material, shader, and scene reload are supported in Vulkan path.
- Editor can assign assets and script components and report build/runtime errors.

## Acceptance Checklist

- [ ] Texture reload updates visible scene without restart.
- [ ] Material reload updates affected objects only.
- [ ] Shader reload recreates dependent pipelines safely.
- [ ] Scene reload validates before committing new ECS state.
- [ ] Failed reload preserves last valid asset or scene.
- [ ] Reload diagnostics show changed files, recook status, errors, and reload latency.
- [ ] OpenGL/DX12 stubs compile against hot-reload-facing contracts.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- Watcher batching tests
- Incremental recook dependency selection tests
- Registry reload state transition tests
- Failed reload fallback tests

## Manual Validation

- Run sandbox with auto-reload enabled.
- Modify a texture, material, shader, and scene file independently.
- Repeat each reload type many times under Vulkan validation layers.
- Introduce a shader compile error and confirm old pipeline stays active.

## Blocking Issues

- Reload mutates GPU resources while in-flight frames still reference them.
- Broken assets replace working assets.
- Scene reload silently overwrites unsaved editor changes without a clear policy.
- Reload path changes `AssetRegistry-v0` or `ScriptAPI-v0` unexpectedly.

## Required Evidence

- Reload logs for success and failure cases.
- Validation-layer output from repeated reload tests.
- Manual test notes for each supported asset type.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 7: yes/no
