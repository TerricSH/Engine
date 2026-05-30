# Gate 3 Validation And Acceptance

## Gate Exit Principle

Gate 3 is accepted only when rendering is driven through a stable scene-level input contract instead of raw Vulkan sample calls.

## Verification Goals

- Prove rendering is driven by a stable scene-level renderer input instead of raw Vulkan sample code.
- Prove later ECS/editor/script systems can target `RendererInput-v0` without touching backend internals.
- Prove material, lighting, culling, and draw statistics have stable shapes.

## Required Results

- Static lit sandbox scene renders through a `draw_scene`-style API.
- `RendererInput-v0` contains camera, renderable instances, mesh/material references, lights, bounds, culling output, and draw statistics.
- PBR Metallic-Roughness material path (per `FD-026`) renders correctly with at least one directional + one point light on a textured sphere fixture.
- HDR offscreen target + ACES tone-mapping pass produce a correctly exposed image; no manual gamma in shaders.
- Single directional shadow map pass (per `FD-028`) casts shadows for the lit fixture; `ShadowMode::Hard` for point/spot lights is downgraded with a diagnostic.

## Acceptance Checklist

- [ ] Renderer scene input contract is documented.
- [ ] Renderer accepts high-level scene data and produces draw submissions internally.
- [ ] Sandbox no longer manually assembles all raw Vulkan draw state outside the renderer layer.
- [ ] Draw statistics expose at least submitted draw count, visible object count, and culled object count.
- [ ] Material/light/camera parameter structures are stable enough for ECS extraction.
- [ ] OpenGL/DX12 stubs still compile against the renderer/RHI expectations.
- [ ] PBR-MR shading fixture (textured sphere + directional + 2 point lights) matches the reference golden image within a small tolerance.
- [ ] Render graph contains exactly `directional_shadow_pass -> opaque_pbr_forward_pass -> tone_map_pass -> present` and emits a `tracing` span per pass (per `FD-014`).
- [ ] Submitting `LightKind::Area` is rejected at validation; submitting `LightItem.intensity` with a unit mismatching its kind emits a diagnostic and drops the light.
- [ ] Submitting `ShadowMode::Hard` for a point/spot light produces exactly one diagnostic per asset and never aborts the frame.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- `cargo run -p sandbox --features backend-vulkan`

## Manual Validation

- Move or configure the camera and confirm the scene updates correctly.
- Verify culling/draw statistics change when objects enter/leave view.
- Confirm lighting/material changes are visible.
- Inspect code paths to ensure systems above the renderer do not call `render-vulkan` directly.

## Blocking Issues

- `RendererInput-v0` still exposes Vulkan-specific types.
- Static scene cannot render through the high-level renderer API.
- Culling or draw statistics are missing, making later diagnostics impossible.
- Material/light data shapes are still too unstable for Gate 4 ECS extraction.
- Lighting pass writes directly to the swapchain instead of an HDR offscreen target (violates `FD-026`).
- A deferred G-buffer is introduced (violates `FD-027`).
- Per-light point/spot shadow maps are implemented (deferred to Gate 10/11 by `FD-028`).

## Required Evidence

- Static lit scene screenshot.
- Renderer input API review note.
- Check command outputs.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 4: yes/no

