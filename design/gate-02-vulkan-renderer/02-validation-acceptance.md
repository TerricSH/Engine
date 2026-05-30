# Gate 2 Validation And Acceptance

## Gate Exit Principle

Gate 2 is accepted only when Vulkan proves `RHI-v0` with a visible triangle and textured object, while OpenGL and DirectX 12 stubs continue compiling against the same contract.

## Verification Goals

- Prove the Vulkan backend can create a window surface, select a device, create a swapchain, submit command buffers, and present frames.
- Prove the first rendered outputs are visible and stable: triangle first, then textured cube or quad.
- Prove OpenGL and DirectX 12 stubs still compile against `RHI-v0` while Vulkan becomes real.

## Required Results

- Sandbox opens a native window using the platform layer.
- Vulkan renders a triangle and a textured object.
- Resize, minimize/restore, and shutdown do not crash.
- Vulkan validation layers report no errors during startup, draw, resize, and shutdown.

## Acceptance Checklist

- [ ] Vulkan instance, debug messenger, surface, physical device, logical device, queues, swapchain, command buffers, synchronization, shader modules, and graphics pipeline are implemented.
- [ ] Triangle rendering path is visible.
- [ ] Textured cube or quad rendering path is visible.
- [ ] Swapchain recreation handles resize without panic or leaked resources.
- [ ] OpenGL and DirectX 12 stubs compile after any approved RHI adjustment.
- [ ] Sandbox code does not become the public engine API.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo run -p sandbox --features backend-vulkan`
- `cargo check --workspace --features backend-opengl`
- Windows only: `cargo check --workspace --features backend-dx12`

## Manual Validation

- Capture screenshot or short recording of triangle and textured object.
- Run with Vulkan validation layers enabled and attach relevant log excerpt.
- Resize the window repeatedly for at least 30 seconds.
- Minimize, restore, and close the window.

## Blocking Issues

- Any Vulkan validation error during normal startup/draw/resize/shutdown.
- Rendering only works through direct ad hoc sample code that bypasses `render-core`.
- Backend stubs fail after Vulkan-driven RHI changes.
- Swapchain recreation is unstable.

## Required Evidence

- Screenshot or capture of triangle and textured object.
- Validation layer log excerpt.
- Command outputs from all check commands.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 3: yes/no

