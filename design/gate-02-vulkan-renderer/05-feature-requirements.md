# Gate 2 Feature Requirements And Execution Boundaries

## Gate Objective

Implement the first real Vulkan backend path and prove `RHI-v0` with visible rendering, stable frame lifecycle, validation-layer cleanliness, and backend stub compatibility.

## Required Features

### G2-F01 Vulkan Instance And Debug Setup

Required behavior:
- Load Vulkan entry points.
- Create instance with required platform extensions.
- Enable validation layers in development builds.
- Create debug messenger/callback path.

Minimum output:
- Startup log identifies enabled layers/extensions.
- Validation messages are routed to diagnostics/logging.

Do not overbuild:
- Do not add a custom validation framework beyond basic routing.

### G2-F02 Surface, Adapter, Device, And Queues

Required behavior:
- Create Vulkan surface from platform raw window handle.
- Select physical device based on required queue families and swapchain support.
- Create logical device and retrieve queues.
- Store adapter/device/queue information behind `render-vulkan` types.

Minimum output:
- Device selection succeeds on a supported machine.
- Missing Vulkan support reports a clear error.

Do not overbuild:
- No multi-GPU selection UI.
- No compute/transfer queue optimization beyond what is needed for rendering.

### G2-F03 Swapchain And Frame Resources

Required behavior:
- Create swapchain, image views, format, present mode, and extent.
- Create per-frame command buffers, semaphores, and fences.
- Implement frames-in-flight.
- Handle resize/out-of-date/suboptimal swapchain states.

Minimum output:
- Window resize does not crash.
- Minimize/restore is handled safely.
- Shutdown releases resources in valid order.

Do not overbuild:
- No advanced swapchain latency controls.
- No render graph ownership yet.

### G2-F04 Basic Pipeline And Triangle

Required behavior:
- Load shader modules.
- Create pipeline layout and graphics pipeline.
- Record command buffer for a triangle.
- Submit and present through frame lifecycle.

Minimum output:
- Visible triangle in sandbox.
- Validation layers clean.

Do not overbuild:
- No material system.
- No dynamic shader pipeline database.

### G2-F05 Textured Object Path

Required behavior:
- Create vertex/index buffers as needed.
- Upload a texture through staging or equivalent upload path.
- Bind descriptors or early binding model needed for texture sampling.
- Render a textured quad or cube.

Minimum output:
- Visible textured object in sandbox.
- Texture upload and resource cleanup are deterministic.

Do not overbuild:
- No full asset registry.
- No texture compression pipeline.

### G2-F06 Backend Stub Preservation

Required behavior:
- Keep OpenGL and DirectX 12 stubs compiling after Vulkan-driven RHI adjustments.

Minimum output:
- Feature-gated checks pass for stubs.

## Target Effects

- Vulkan backend proves the RHI can drive real rendering.
- Frame lifecycle becomes stable enough for later resources, render graph, and hot reload.
- Stubs keep the abstraction honest.

## Explicit Non-Goals

- No ECS scene runtime.
- No asset cooking.
- No editor or scripting.
- No production material system or full render graph.
- No real OpenGL/DX12 backend implementation.

## AI Execution Rules

- Keep all Vulkan-specific code inside `render-vulkan`.
- Treat validation-layer errors as blockers.
- Do not use `device_wait_idle` as routine frame flow.
- Do not promote sandbox sample functions into stable engine APIs.

## Completion Signal

Gate 2 is complete when the sandbox renders a triangle and textured object through Vulkan, resize/minimize/shutdown are stable, validation layers are clean, and OpenGL/DX12 stubs still compile.
