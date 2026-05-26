# Gate 3 Test Plan

## Test Strategy

Gate 3 tests prove that rendering is driven through `RendererInput-v0`, not raw Vulkan sample code. Tests must cover data validation, material/light behavior, render graph basics, and renderer statistics.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G3-F01 RendererInput-v0 | Build input from static scene data | Unit | Input contains camera, renderables, materials, lights, bounds |
| G3-F01 RendererInput-v0 | Reject invalid input | Unit | Missing camera/material/mesh reports a clear validation error |
| G3-F02 High-Level Draw Entry | Draw static scene through `draw_scene` | Integration | Scene renders without direct sandbox Vulkan calls |
| G3-F02 High-Level Draw Entry | Statistics collection | Unit/Runtime | Draw count, visible count, culled count are reported |
| G3-F03 Material Descriptor Baseline | Validate material descriptor | Unit | Invalid shader/texture bindings are rejected |
| G3-F04 Lighting And Shadow Baseline | Light toggles affect scene | Visual | Lighting changes visible output |
| G3-F05 Minimal Render Graph | Pass dependency validation | Unit | Invalid pass/resource dependency reports error |

## Gate Integration Tests

1. Renderer input scene test
   - Create static scene data.
   - Build `RendererInput-v0`.
   - Render through `draw_scene`.
2. Culling/statistics test
   - Move camera or objects.
   - Confirm visible/cull counts change.
3. Material/light integration test
   - Use at least two materials and one light.
   - Confirm renderer routes data through descriptors.
4. Backend isolation test
   - Search/test that no system above renderer imports `render-vulkan`.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- `cargo run -p sandbox --features backend-vulkan -- static-lit-scene`

## Required Evidence

- Static lit scene screenshot.
- Renderer statistics log.
- API review note confirming backend independence.

## Failure Criteria

- `RendererInput-v0` includes backend-native handles.
- Static scene still bypasses `draw_scene` for normal rendering.
- Material/light data cannot be validated before rendering.

## Test Fixtures

- `static_lit_scene` fixture containing at least one camera, two renderables, two materials, one light, and one object outside the camera frustum.
- Material descriptors for a solid color material and a textured material.
- Expected renderer statistics fixture: visible count, culled count, submitted draw count.

## Executable Integration Cases

### IT-G3-01 Renderer Input Build

Steps:
1. Build `RendererInput-v0` from `static_lit_scene` fixture.
2. Validate the input.
3. Serialize debug dump of renderer input.

Expected:
- Input contains exactly one active camera.
- Renderables reference typed mesh/material IDs.
- No backend-native handles are present.

Evidence:
- `target/test-evidence/gate-03/renderer-input.json`.

### IT-G3-02 Draw Scene Integration

Steps:
1. Run sandbox static lit scene.
2. Confirm rendering goes through `draw_scene` path.
3. Capture screenshot and stats.

Expected:
- Scene renders.
- Draw/culling stats match fixture expectations within documented tolerance.
- No direct sandbox Vulkan draw path is used for the main scene.

Evidence:
- Screenshot.
- Stats log.
- Code path note or test assertion.

### IT-G3-03 Invalid Material/Light Data

Steps:
1. Remove material shader reference.
2. Provide invalid light parameters.
3. Run renderer input validation.

Expected:
- Validation fails before backend rendering.
- Error names the invalid material or light entry.

Evidence:
- Validation error log.
