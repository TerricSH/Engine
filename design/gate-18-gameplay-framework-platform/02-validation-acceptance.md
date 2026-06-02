# Gate 18 Validation And Acceptance

## Gate Exit Principle

Gate 18 is accepted only when the engine can run a complete gameplay loop using public APIs across UI, audio, input, scripting, prefabs, character, AI, and platform adaptation.

## Required Results

- Game state manager exists.
- Input action maps exist and support agreed desktop/mobile inputs.
- Gameplay event bus exists.
- Platform adaptation layer exists.
- Lightweight telemetry hooks exist.
- Complete gameplay loop works in a validation project.

## Acceptance Criteria

- [ ] Menu -> load scene -> play -> pause -> save/checkpoint -> game-over -> return to menu works.
- [ ] Input actions work for keyboard/mouse and at least one controller or touch path.
- [ ] Input rebinding or action remapping works at agreed scope.
- [ ] UI, audio, and gameplay communicate through event bus without direct crate-internal access.
- [ ] Platform capability checks return expected values.
- [ ] Telemetry hooks can log local events without backend service integration.
- [ ] C# scripts can query game state and input actions.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- `cargo check --workspace --features backend-vulkan,subsystem-scripting-csharp,target-mobile`
- `cargo test --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- `cargo test --workspace --features backend-vulkan,subsystem-scripting-csharp,target-mobile`
- Game state transition tests.
- Input action map tests.
- Event bus delivery tests.
- Platform capability facade tests.

## Manual Validation

- Play through the complete validation gameplay loop.
- Test pause/resume and checkpoint save/load.
- Test desktop and mobile-style input mappings.
- Inspect local telemetry/event logs.

## Blocking Conditions

- Gameplay framework accesses subsystem internals instead of public APIs.
- Event bus is used for low-level engine scheduling.
- Input actions cannot be serialized or rebound.
- Mobile adaptation requires changes to core gameplay code.

## Required Evidence

- Gameplay loop capture or log.
- Input mapping test result.
- Event bus integration notes.
- Platform capability test output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 19: yes/no

