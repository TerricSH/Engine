# Gate 12: Character Controller And Physics-Animation Integration

## Purpose

Unify physics and animation into a character movement layer. This gate creates the foundation for player movement and AI-controlled agents.

## Entry Sync Point

- Gate 10 physics and animation foundations are complete.
- Gate 11 physics/animation expansion is stable enough to expose public APIs for controllers, animation state, and character movement.

## Parallel Workstreams

1. Character Controller Foundation
   - Owns `crates/engine-character` if split out, or character modules inside `engine-core`.
   - Adds capsule-based movement, walking, jumping, falling, landing detection, slope handling, and air control.
   - Uses physics public APIs; does not modify physics internals.
2. Physics-Animation Synchronization
   - Owns glue systems that map physics movement states to animation state requests.
   - Handles landing events, movement speed parameters, grounded state, and simple hit reaction hooks.
   - Does not implement ragdoll yet.
3. Locomotion Animator Layer
   - Owns idle/walk/run/jump/fall/land locomotion state machine data and runtime evaluation.
   - Uses animation expansion APIs; does not edit animation core directly unless coordinated.
4. C# Character API
   - Adds `MoveCharacter`, `Jump`, `IsGrounded`, `GetMoveState`, and movement parameter fields.
   - Keeps gameplay script API backend-independent.

## Exit Condition

- Character test scene supports moving, jumping, falling, landing, and animation transitions.
- Physics and animation do not both author the same transform in conflicting ways.
- C# can drive character movement and query controller state.
- Character controller state saves/loads with scene.

## Non-Goals

- Swimming, climbing, parkour, ledge grabs, advanced IK, multiplayer prediction, and ragdoll.

## Parallel Safety Notes

- Controller uses physics/animation public APIs only.
- AI should later drive the controller, not write transforms directly.
- Any change to physics or animation core must be a separate integration change.
