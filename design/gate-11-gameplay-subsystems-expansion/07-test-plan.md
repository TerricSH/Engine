# Gate 11 Test Plan

## Test Strategy

Gate 11 tests prove expanded physics and animation APIs are ready for the character controller, and that optional prefab/UI/audio seeds do not destabilize core contracts.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G11-F01 Physics Constraint Baseline | Create selected joint/constraint | Integration | Bodies constrained according to descriptor |
| G11-F02 Batched Physics Queries | Batch raycast/overlap | Unit/Integration | Results match individual queries |
| G11-F03 Animation State Runtime | Transition between states | Integration | Blend/transition behaves as configured |
| G11-F04 Root Motion Policy | Root motion ownership test | Review/Integration | Transform owner remains unambiguous |
| G11-F05 Prefab Seed | Seed prefab round-trip | Unit | Versioned seed saves/loads if included |

## Gate Integration Tests

1. Physics expansion scene
   - Test selected constraints and batched queries.
2. Animation state scene
   - Test state transitions, layers, notifies, and root motion policy.
3. Combined prep scene
   - Confirm APIs are ready for Gate 12 without patching internals.

## Required Evidence

- Constraint test output.
- Animation state transition logs.
- Root motion ownership decision note.

## Failure Criteria

- Gate 12 would need to edit physics/animation internals.
- Root motion ownership is unclear.
- Constraint descriptors expose backend handles.

## Test Fixtures

- `scenes/gate11_constraints.scene`: bodies connected by selected joint types.
- `scenes/gate11_animation_state.scene`: simple state machine with idle/walk/jump transitions.
- `scenes/gate11_root_motion.scene`: clip with root delta or simulated root motion output.
- Optional `prefabs/gate11_seed.prefab` if prefab seed is included.

## Executable Integration Cases

### IT-G11-01 Constraint And Batched Query Validation

Steps:
1. Load constraint scene.
2. Run physics for fixed steps.
3. Execute batched raycast/overlap/sweep queries.
4. Inspect joint debug data.

Expected:
- Constraints behave within tolerance.
- Batched query results match individual queries.
- No backend handles appear in serialized descriptors.

Evidence:
- Constraint simulation log.
- Query comparison report.

### IT-G11-02 Animation State Machine Validation

Steps:
1. Load animation state scene.
2. Set parameters for idle -> walk -> jump -> land.
3. Capture transitions, blend state, and notifies.

Expected:
- Transitions occur according to data.
- Notifies fire at expected normalized times.
- Root motion policy is applied exactly as documented.

Evidence:
- Animation state trace.
- Root motion ownership note.

### IT-G11-03 Gate 12 Readiness Review

Steps:
1. List physics APIs required by character controller.
2. List animation APIs required by locomotion.
3. Confirm both are public and documented.

Expected:
- Gate 12 can start without modifying physics/animation internals.

Evidence:
- Readiness checklist.
