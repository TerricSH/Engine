# Gate 11 Feature Requirements And Execution Boundaries

## Gate Objective

Expand physics and animation just enough to support character controller work and seed prefab/UI/audio expansion without destabilizing core scene or subsystem contracts.

## Required Features

### G11-F01 Physics Constraint Baseline

Required behavior:
- Implement selected joint/constraint descriptors at engine level.
- Include limits, motors, connected body references, and breakage/feedback where scoped.

Minimum output:
- Joint/constraint validation scene runs and save/loads.

### G11-F02 Batched Physics Queries

Required behavior:
- Add batched raycast/overlap/sweep API for gameplay and editor tools.

Minimum output:
- Batched query tests pass and produce debug output.

### G11-F03 Animation State Runtime

Required behavior:
- Implement animation parameters, state machine asset/runtime, transitions, blend duration/mode, layers, and notifies/events at minimum scope.

Minimum output:
- Character-like animation state validation scene runs.

### G11-F04 Root Motion Policy

Required behavior:
- Define how root motion is represented and who owns transform application.

Minimum output:
- Root motion is disabled, routed through controller, or limited to non-character objects by documented rule.

### G11-F05 Prefab Seed

Required behavior:
- If included, define minimal prefab source asset ID, hierarchy snapshot, component defaults, and version field.

Minimum output:
- Prefab seed does not break base scene serialization.

## Target Effects

- Gate 12 can build character controller without patching physics or animation internals.
- Expanded subsystem data remains serializable and debuggable.

## Explicit Non-Goals

- No character controller.
- No full animation timeline editor.
- No full prefab override system unless explicitly moved from Gate 14.
- No ragdoll, IK, crowd simulation, or production UI/audio.

## AI Execution Rules

- Expose engine-owned descriptors, not backend handles.
- Root motion policy must be explicit.
- Keep prefab seed versioned if implemented.
- Do not start character controller inside this gate.

## Completion Signal

Gate 11 is complete when expanded physics/animation APIs are stable, root motion policy is documented, and combined validation scene passes.
