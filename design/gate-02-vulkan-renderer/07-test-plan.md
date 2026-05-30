# Gate 2 Test Plan

## Test Strategy

Gate 2 tests prove that Vulkan works as the first real backend and that the frame lifecycle is stable under normal window operations. GPU tests are required; compile tests alone are not sufficient.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G2-F01 Vulkan Instance And Debug Setup | Start sandbox with validation layers | Runtime | Instance and debug messenger initialize; validation output is captured |
| G2-F02 Surface, Adapter, Device, And Queues | Create window and device | Runtime | Supported adapter selected; queue families logged |
| G2-F03 Swapchain And Frame Resources | Resize/minimize/restore loop | Integration | Swapchain recreates or pauses safely without crash |
| G2-F03 Swapchain And Frame Resources | Frames-in-flight fence reuse test | Runtime | Frame resources are not reused before fences complete |
| G2-F04 Basic Pipeline And Triangle | Render triangle scene | Visual | Triangle appears and validation layers stay clean |
| G2-F05 Textured Object Path | Render textured quad/cube | Visual | Texture appears with correct orientation and sampling |
| G2-F05 Textured Object Path | Destroy resources after GPU use | Runtime | No validation errors on shutdown |
| G2-F06 Backend Stub Preservation | Compile OpenGL/DX12 stubs after Vulkan changes | Compile | Stubs still compile and return unsupported paths |

## Gate Integration Tests

1. Vulkan smoke scene
   - Launch sandbox.
   - Render triangle for at least 300 frames.
   - Exit cleanly.
2. Textured object scene
   - Render textured quad/cube for at least 300 frames.
   - Confirm no validation errors.
3. Window lifecycle test
   - Resize repeatedly.
   - Minimize and restore.
   - Close the window.
4. Backend regression test
   - Run backend stub compile checks after Vulkan implementation merges.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo run -p sandbox --features backend-vulkan -- triangle`
- `cargo run -p sandbox --features backend-vulkan -- textured-object`
- `cargo check --workspace --features backend-opengl`
- Windows: `cargo check --workspace --features backend-dx12`

## Required Evidence

- Screenshot or capture of triangle.
- Screenshot or capture of textured object.
- Vulkan validation log excerpt showing no errors.
- Resize/minimize/restore test notes.

## Failure Criteria

- Any validation-layer error during normal operation.
- Swapchain recreation crash or device loss caused by normal resize.
- Backend stubs fail because Vulkan work changed `render-core` incompatibly.

## Test Fixtures

- `sandbox` executable with scene modes: `triangle`, `textured-object`, and `resize-smoke`.
- Minimal embedded shaders for triangle and textured object.
- A deterministic test texture with visible orientation markers.
- Vulkan validation layers installed on the test machine.

## Executable Integration Cases

### IT-G2-01 Triangle Smoke

Setup:
- Enable Vulkan validation layers.

Steps:
1. Run `cargo run -p sandbox --features backend-vulkan -- triangle --frames 300`.
2. Capture one screenshot after the first successful present.
3. Save validation-layer log.

Expected:
- Triangle is visible.
- Process exits cleanly after requested frames.
- Validation log has zero errors.

Evidence:
- Screenshot: `target/test-evidence/gate-02/triangle.png`.
- Log: `target/test-evidence/gate-02/triangle-validation.log`.

### IT-G2-02 Textured Object Smoke

Setup:
- Use deterministic texture fixture with top/left/right markers.

Steps:
1. Run `cargo run -p sandbox --features backend-vulkan -- textured-object --frames 300`.
2. Capture screenshot.
3. Verify orientation markers are visible.

Expected:
- Textured quad/cube renders.
- Texture orientation is correct.
- No validation errors on shutdown.

Evidence:
- Screenshot and validation log.

### IT-G2-03 Swapchain Lifecycle

Setup:
- Run sandbox in resize smoke mode.

Steps:
1. Resize window to several sizes, including small dimensions.
2. Minimize and restore.
3. Close normally.

Expected:
- Swapchain recreates or pauses safely.
- No crash, panic, validation error, or device loss during normal operations.

Evidence:
- Resize event log with swapchain recreation count.

### IT-G2-04 Backend Stub Regression

Steps:
1. Run `cargo check --workspace --features backend-opengl`.
2. On Windows, run `cargo check --workspace --features backend-dx12`.

Expected:
- Backend stubs remain compatible with `render-core`.

Evidence:
- Compile logs.
