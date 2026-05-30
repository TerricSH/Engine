# Gate 11 Session Prompts

Gate 11 expands physics and animation just enough to unblock character controller and later systems.

## Session 11A: Physics Expansion Owner

Goal: Add selected joints/constraints, query batching, and richer debug views.

Owns:
- `engine-physics` expansion modules

Must not edit:
- character controller code
- animation internals

Expected output:
- engine-owned joint descriptors
- batched query API

Validation:
- constraint validation scene
- query batch tests

## Session 11B: Animation Expansion Owner

Goal: Add animation state runtime, parameters, transitions, layers, notifies, and root motion policy.

Owns:
- `engine-animation` expansion modules

Must not edit:
- character controller code
- physics internals

Expected output:
- state machine runtime data
- root motion policy documented

Validation:
- animation state validation scene

## Session 11C: Prefab Seed Owner

Goal: Add minimal prefab seed only if it remains versioned and safe.

Owns:
- prefab seed schema/tests

Must not edit:
- base scene schema in breaking ways

Expected output:
- optional source asset ID + component defaults + version field

Validation:
- seed schema round-trip
