# Gate 3 Session Prompts

Gate 3 moves rendering above raw Vulkan samples. Sessions here must preserve backend independence and avoid ECS coupling.

## Session 3A: RendererInput-v0 Owner

Goal: Define the renderer scene input contract.

Owns:
- Renderer-facing data structs in `engine-core` or renderer module
- Renderer validation helpers

Must not edit:
- ECS crates
- Asset registry crates
- Backend internals except compile-driven type integration

Expected output:
- Camera, renderable, material ref, light, bounds, stats data shapes
- Validation logic for renderer input

Validation:
- Static scene can be converted into `RendererInput-v0`
- API review confirms no backend-native types

## Session 3B: Material And Lighting Baseline

Goal: Add the first typed material and light path.

Owns:
- Material descriptor structs
- Light render data
- Basic forward lighting shaders

Must not edit:
- Asset cookers
- Editor UI

Expected output:
- Static lit scene visibly responds to material/light data
- Material descriptors are typed and backend-neutral

Validation:
- Lighting/material visual check
- Shader compile/check path if available

## Session 3C: Minimal Render Graph And Stats

Goal: Introduce minimal pass/resource structure and renderer statistics.

Owns:
- Minimal render graph modules
- Draw/culling statistics

Must not edit:
- Full renderer scheduling beyond gate scope

Expected output:
- Named color/depth/shadow pass structure at agreed scope
- Draw stats exposed for diagnostics

Validation:
- Static scene renders through `draw_scene`-style API
- Stats are visible in logs/tests
