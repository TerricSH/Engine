# Gate 2: Vulkan Renderer Foundation

## Purpose

Prove the RHI contract by implementing the first real Vulkan rendering path. This gate keeps OpenGL and DirectX 12 as boundary validators while Vulkan establishes the first working renderer.

## Entry Sync Point

- `RHI-v0` is frozen.

## Parallel Workstreams

1. Vulkan MVP
   - Owns `crates/render-vulkan`, early shaders, and renderer sandbox.
   - Implements Vulkan instance, device, surface, swapchain, queue, command buffers, synchronization, pipeline creation, and presentation.
   - Renders a triangle, then a textured cube or quad.
2. Backend Boundary Validation
   - Owns `crates/render-opengl` and `crates/render-dx12` stubs.
   - Keeps stubs compiling after approved RHI adjustments.
3. Renderer Diagnostics
   - Owns validation scenes, smoke tests, resize tests, and validation-layer notes.

## Contracts To Preserve

- `RHI-v0`
- Backend isolation
- No Vulkan type leakage into engine-level APIs

## Exit Condition

- Vulkan renders triangle and textured object.
- Resize and shutdown are stable.
- Vulkan validation layers are clean.
- Backend stubs still compile.

## Parallel Safety Notes

- OpenGL/DX12 sessions do not edit `render-core`.
- Vulkan session owns only Vulkan implementation and renderer validation samples.
- Any RHI mismatch should be recorded as an integration issue before merging.
