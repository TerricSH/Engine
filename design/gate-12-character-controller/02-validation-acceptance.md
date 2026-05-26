# Gate 12 Validation And Acceptance

## Gate Exit Principle

Gate 12 is accepted only when a character can move through physics-safe controller APIs and drive locomotion animation without conflicting transform ownership.

## Required Results

- Character controller component exists.
- Walking, jumping, falling, landing, slope handling, and air control work.
- Physics-animation synchronization drives locomotion states.
- C# character API exists.
- Character controller state saves/loads with scene.

## Acceptance Criteria

- [ ] Player-controlled character can move, jump, fall, and land in a test scene.
- [ ] Grounded state is correct on flat surfaces and slopes within supported limits.
- [ ] Animation transitions occur for idle, walk, run, jump, fall, and land.
- [ ] C# can call `MoveCharacter`, `Jump`, `IsGrounded`, and `GetMoveState`.
- [ ] Controller does not directly patch physics or animation internals.
- [ ] Transform ownership rules are documented and enforced.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp`
- Controller state transition tests.
- Scene save/load tests for controller components.
- C# API binding tests.

## Manual Validation

- Run character controller test scene.
- Test walking, jumping, falling, landing, and slope behavior.
- Watch animation transitions under controller input.
- Save and reload a scene containing a character controller.

## Blocking Conditions

- Controller and physics both author transform in conflicting ways.
- AI in Gate 13 would need to bypass controller to move agents.
- Character controller state cannot be serialized.
- C# controller API is backend- or physics-backend-specific.

## Required Evidence

- Character controller test video or log.
- Transform ownership review note.
- C# movement script sample output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 13: yes/no

