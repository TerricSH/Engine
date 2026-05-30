# Gate 2 Session Prompts

Each session must read the gate design documents before editing. Gate 2 proves `RHI-v0` with real Vulkan code; OpenGL and DirectX 12 remain validation stubs.

## Session 2A: Vulkan Device And Swapchain

Goal: Bring up Vulkan instance/device/surface/swapchain.

Owns:
- `crates/render-vulkan/src/instance*`
- `crates/render-vulkan/src/device*`
- `crates/render-vulkan/src/surface*`
- `crates/render-vulkan/src/swapchain*`

Must not edit:
- `render-core` without RHI owner review
- OpenGL/DX12 crates except if compile fixes are requested

Expected output:
- Validation layers and debug messenger
- Physical/logical device selection
- Swapchain create/recreate path

Validation:
- Sandbox opens a window and creates a swapchain
- Repeated resize/minimize/restore test

## Session 2B: Vulkan Frame And Pipeline

Goal: Implement frames-in-flight, command recording, shader module loading, and graphics pipeline.

Owns:
- `crates/render-vulkan/src/frame*`
- `crates/render-vulkan/src/command*`
- `crates/render-vulkan/src/pipeline*`
- early shader files

Must not edit:
- Asset pipeline or material system

Expected output:
- Triangle rendering
- Textured object rendering
- Clean validation layer output

Validation:
- Run sandbox and capture triangle/textured object
- Vulkan validation layers produce no errors

## Session 2C: Backend Stub Validation

Goal: Keep non-Vulkan backend stubs aligned with any approved RHI adjustments.

Owns:
- `crates/render-opengl`
- `crates/render-dx12`

Must not edit:
- `crates/render-core` directly
- `crates/render-vulkan`

Expected output:
- Stubs compile after Vulkan work lands

Validation:
- `cargo check --workspace --features backend-opengl`
- Windows: `cargo check --workspace --features backend-dx12`
