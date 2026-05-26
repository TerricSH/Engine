# Gate 6 Test Plan

## Test Strategy

Gate 6 tests prove hot reload works as a staged pipeline and never corrupts the last valid runtime state.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G6-F01 File Watch Or Manual Reload Trigger | Change watched texture/material/shader/scene | Integration | Reload request is created after debounce |
| G6-F02 Incremental Recook | Change dependency source | Unit/Integration | Changed asset and reverse dependencies recook |
| G6-F02 Incremental Recook | Broken source file | Integration | Recook fails and old asset remains active |
| G6-F03 Reload Queue | State transition test | Unit | Detected -> recooking -> queued -> applied/failed states are visible |
| G6-F04 Runtime Resource Reload | Texture/material reload | Runtime | Visible scene updates without restart |
| G6-F04 Runtime Resource Reload | Shader compile failure | Runtime | Old shader/pipeline remains active |
| G6-F04 Runtime Resource Reload | Repeated GPU reload stress | Runtime | No validation-layer lifetime/sync errors |
| G6-F05 Diagnostics | Check editor/sandbox diagnostics | Manual | Path, asset ID, status, error, latency visible |

## Gate Integration Tests

1. Full hot reload path
   - Edit texture.
   - Watch, recook, queue, apply, render new result.
2. Shader failure rollback
   - Introduce invalid shader.
   - Confirm old pipeline continues rendering.
3. Scene reload policy
   - Modify scene file while editor has dirty state.
   - Confirm selected policy is enforced.
4. Repeated reload stress
   - Reload texture/material/shader repeatedly under Vulkan validation layers.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp,hot-reload`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp,hot-reload`
- `cargo run -p sandbox --features backend-vulkan,editor,scripting-csharp,hot-reload -- hot-reload-scene`

## Required Evidence

- Reload state logs.
- Failure fallback logs.
- Vulkan validation output from repeated reload stress.

## Failure Criteria

- Failed reload replaces a valid resource.
- GPU resource is destroyed while in-flight.
- Runtime and editor disagree about reload state.
- Auto scene reload overwrites unsaved edits without policy.

## Test Fixtures

- `assets/source/gate06/reload_texture.png`: texture with visible version marker.
- `assets/source/gate06/reload_material.ron`: material parameter file.
- `shaders/gate06/reload_shader.vert/frag`: shader pair with valid and invalid variants.
- `scenes/gate06_reload.scene`: scene containing reloadable texture/material/shader references.
- Dirty editor scene fixture for scene reload conflict policy.

## Executable Integration Cases

### IT-G6-01 Texture Material Reload

Steps:
1. Start sandbox in hot reload mode.
2. Modify texture marker.
3. Modify material parameter.
4. Wait for reload state to become applied.

Expected:
- Visible scene updates without restart.
- Reload diagnostics show changed asset IDs and success.

Evidence:
- Before/after screenshots.
- Reload state log.

### IT-G6-02 Shader Failure Fallback

Steps:
1. Start sandbox with valid shader.
2. Replace shader with invalid variant.
3. Wait for reload failure.
4. Restore valid shader.

Expected:
- Invalid shader reports compile error.
- Old pipeline remains active.
- Restored shader reloads successfully.

Evidence:
- Compile error log.
- Validation-layer log.

### IT-G6-03 Scene Reload Dirty Policy

Steps:
1. Open scene in editor mode.
2. Modify scene without saving.
3. Externally modify scene file.
4. Trigger scene reload.

Expected:
- Selected dirty-scene policy is enforced: block, prompt, or manual reload only.
- No silent data loss.

Evidence:
- Editor diagnostics log.
