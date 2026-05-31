# Gate 15 Validation And Acceptance

## Gate Exit Principle

Gate 15 is accepted only when runtime UI can render, receive input, serialize, and trigger script callbacks without depending on editor UI internals.

## Required Results

- `engine-ui` runtime crate exists.
- Canvas hierarchy and layout model work.
- UI rendering extraction emits renderer commands.
- Pointer, mouse, keyboard, and touch routing are represented.
- Basic widgets exist.
- UI hierarchy can save/load.
- C# callbacks can respond to UI events.

## Acceptance Criteria

- [ ] Runtime UI renders over gameplay scene.
- [ ] Button click triggers a callback.
- [ ] Text/image/panel widgets render correctly.
- [ ] Basic layout/anchoring works across at least two window sizes.
- [ ] UI input can capture events and prevent gameplay input when appropriate.
- [ ] UI scene saves and reloads.
- [ ] UI system does not import editor UI internals.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,subsystem-scripting-csharp`
- `cargo test --workspace --features backend-vulkan,subsystem-scripting-csharp`
- Layout calculation tests.
- UI input hit-test tests.
- UI scene serialization tests.

## Manual Validation

- Run UI validation scene at multiple window sizes.
- Click/touch widgets and verify callbacks.
- Confirm UI layer draw order and clipping behavior.
- Save/reload a scene containing UI.

## Blocking Conditions

- Runtime UI depends on editor UI code.
- UI events bypass platform input routing.
- UI render extraction calls Vulkan directly.
- UI hierarchy cannot serialize.

## Required Evidence

- UI scene screenshot or capture.
- Callback log output.
- Layout test results.

## Exit Decision

> **Implementation status:**
> - P0 items: 6/6 resolved (core controls, text rendering, Canvas
>   serialization/registration).
> - P1 items: keyboard focus, capture ownership, touch tracking — addressed.
> - Performance measurements: pending (see [`04-performance-report.md`](04-performance-report.md)).

- Gate owner: *TBD — assign before gate exit*
- Date: *TBD*
- Approved to proceed to Gate 16: *pending*

