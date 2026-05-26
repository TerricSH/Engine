# Gate 3 Feature Requirements And Execution Boundaries

## Gate Objective

Create the backend-neutral renderer scene contract that future ECS, editor, asset, and script systems will feed. Rendering must move from raw Vulkan sample code to a high-level renderer input flow.

## Required Features

### G3-F01 RendererInput-v0

Required behavior:
- Define a frame-local renderer input data structure.
- Include camera, renderable instances, mesh references, material references, lights, bounds, visibility/culling data, and render options.
- Keep it independent from ECS storage and backend-native objects.

Minimum output:
- Static sandbox data can be converted into `RendererInput-v0`.
- Renderer consumes this input without querying sandbox internals.

Do not overbuild:
- No full scene graph.
- No ECS dependency.

### G3-F02 High-Level Draw Entry

Required behavior:
- Add `draw_scene` or equivalent renderer facade.
- Validate renderer input before submitting backend work.
- Collect render statistics.

Minimum output:
- Static lit scene renders through this facade.
- Stats include visible object count, culled object count, and submitted draw count.

### G3-F03 Material Descriptor Baseline

Required behavior:
- Define typed material descriptor structure.
- Include shader reference, parameter layout, texture bindings, sampler expectations, and render state.

Minimum output:
- Renderer can draw at least one material type through descriptor data.
- Material data does not expose Vulkan descriptors.

Do not overbuild:
- No PBR material system.
- No material graph or editor tooling.

### G3-F04 Lighting And Shadow Baseline

Required behavior:
- Add forward lighting data structures.
- Support at least one basic directional light or equivalent simple light path.
- Add depth/shadow pass representation if included in this gate scope.

Minimum output:
- Static scene visibly responds to lighting.
- Light data is typed and serializable later.

### G3-F05 Minimal Render Graph

Required behavior:
- Represent passes and resources at a minimal level.
- Include pass names for debugging.
- Track color/depth/shadow resource dependencies at agreed scope.

Minimum output:
- Renderer no longer relies entirely on hardcoded pass order hidden in backend code.

## Target Effects

- Future ECS/editor/script systems can target `RendererInput-v0`.
- Renderer can be debugged and measured at the scene-input level.
- Material/light/camera data becomes stable for Gate 4 extraction.

## Explicit Non-Goals

- No ECS runtime.
- No asset registry.
- No full render graph optimizer.
- No deferred, clustered, ray-traced, or PBR renderer.
- No editor/scripting integration.

## AI Execution Rules

- Do not put backend handles in `RendererInput-v0`.
- Do not let renderer query ECS or asset registry in this gate.
- Keep material descriptors typed.
- Keep render graph minimal.

## Completion Signal

Gate 3 is complete when a static lit scene renders through high-level renderer input, statistics are exposed, and later systems can consume the renderer contract without backend access.
