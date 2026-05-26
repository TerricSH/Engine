# Gate 11 Validation And Acceptance

## Gate Exit Principle

Gate 11 is accepted only when expanded physics and animation APIs are stable enough for character controller work, and early prefab/UI/audio expansion does not destabilize scene serialization or editor/script integration.

## Required Results

- Physics expansion includes the selected initial joints/constraints, scene query batching, and improved debug visualization.
- Animation expansion includes blending, layers, state machines, events/notifies, root motion, and transition APIs at the agreed minimum scope.
- Prefab/scene composition seed exists if included in this gate.
- Optional UI/audio expansion stays isolated if started.

## Acceptance Criteria

- [ ] Physics expansion APIs are documented for Gate 12 controller use.
- [ ] Animation state APIs are documented for locomotion use.
- [ ] Root motion ownership rules are documented.
- [ ] Expanded physics and animation scenes save/load correctly.
- [ ] Prefab seed does not break base scene schema.
- [ ] Optional UI/audio work uses separate crates and extension surfaces.
- [ ] Combined sandbox scene runs with expanded physics and animation features together.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp`
- Physics expansion tests.
- Animation blending/state tests.
- Scene round-trip tests including expanded components.

## Manual Validation

- Run a combined scene with expanded physics and animation.
- Verify root motion or animation movement does not conflict with physics ownership.
- Inspect debug visualization for joints/constraints and animation states.

## Blocking Conditions

- Gate 12 would need to patch physics or animation internals to build a controller.
- Expanded components break scene save/load.
- Optional UI/audio work changes editor or renderer internals directly.

## Required Evidence

- Combined sandbox scene output.
- API review notes for physics and animation expansion.
- Root motion/transform ownership note.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 12: yes/no

