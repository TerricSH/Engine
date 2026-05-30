# Gate 12 Test Plan

## Test Strategy

Gate 12 tests prove a character can move through one authoritative controller that uses physics for collision and animation for visual state.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G12-F01 Character Controller Component | Component save/load | Unit | Controller fields persist |
| G12-F02 Movement Command API | Input and C# movement commands | Integration | Same controller API handles both |
| G12-F03 Physics Collision Resolution | Walk into wall/slope/step | Integration | Movement resolves without penetration beyond tolerance |
| G12-F03 Physics Collision Resolution | Jump/fall/land | Integration | Grounded state changes correctly |
| G12-F04 Locomotion Animation Parameters | Movement state drives animation | Integration | Idle/walk/run/jump/fall/land transitions occur |
| G12-F05 C# Character API | C# sample moves/jumps/query state | Integration | C# facade works without backend handles |

## Gate Integration Tests

1. Player movement scene
   - Walk, run, jump, fall, land, slope, step.
2. Animation sync scene
   - Verify animation state follows controller state.
3. Save/load scene
   - Persist controller settings and resume expected state.
4. Transform ownership test
   - Ensure input/AI/animation do not write transform directly.

## Required Evidence

- Character movement video/log.
- Grounded/slope/step test output.
- C# sample log.
- Transform ownership review note.

## Failure Criteria

- AI/input bypasses controller movement.
- Physics and controller both author transform independently.
- Animation root motion causes double movement.
- Controller state cannot serialize.

## Test Fixtures

- `scenes/gate12_character_flat.scene`: flat ground, controllable character, locomotion clips.
- `scenes/gate12_character_slope_step.scene`: slopes, steps, ledges within supported limits.
- `scenes/gate12_character_blockers.scene`: walls and collision blockers.
- `scripts/csharp/Gate12CharacterControllerSample`.
- Input script with deterministic movement/jump sequence.

## Executable Integration Cases

### IT-G12-01 Deterministic Movement Sequence

Steps:
1. Load flat character scene.
2. Replay deterministic input: move forward, stop, jump, wait for landing.
3. Record transform, velocity, grounded state, and movement mode each frame.

Expected:
- Character moves through controller API only.
- Grounded/falling/landing transitions occur in expected order.
- Animation state follows controller state.

Evidence:
- Movement trace JSON.
- Animation state trace.

### IT-G12-02 Slope Step Wall Behavior

Steps:
1. Load slope/step/blocker fixtures.
2. Move character across supported slope.
3. Move over supported step.
4. Move into wall.

Expected:
- Supported slopes/steps are traversed.
- Wall blocks movement without invalid penetration.
- Ground normal and grounded state are correct.

Evidence:
- Physics/controller trace.
- Debug draw capture.

### IT-G12-03 C# Controller Facade

Steps:
1. Run C# movement sample.
2. Call move, jump, query grounded/state/velocity.
3. Save and reload scene.

Expected:
- C# uses facade only.
- Controller settings persist.

Evidence:
- C# log.
- Scene round-trip report.
