# Gate 3: Scene Rendering Contract

## Purpose

Move from raw rendering samples to a renderer scene input contract that future ECS, editor, scripting, and gameplay systems can feed without touching backend internals.

## Entry Sync Point

- Vulkan renderer foundation works.

## Parallel Workstreams

1. Renderer Scene Input
   - Owns camera, meshes, materials, lights, renderable instances, bounds, culling output, and draw statistics.
2. Material And Lighting
   - Owns forward material path, shader descriptors, light data, and basic shadow pass.
3. Static Scene Sandbox
   - Owns a small static lit scene proving rendering flows through a high-level `draw_scene`-style API.

## Contracts To Freeze

- `RendererInput-v0`
- High-level `draw_scene`-style renderer input
- Material binding expectations
- Camera and light parameter formats

## Exit Condition

- Static lit scene renders through high-level renderer API.
- `RendererInput-v0` is frozen.
- ECS/editor/scripting sessions can consume renderer input without backend access.

## Parallel Safety Notes

- ECS and editor work should not start deep integration until this gate exits.
- Material and lighting changes must remain backend-independent above the renderer layer.
