# Gate 8 Performance Test Report

## Status

Not measured yet. Gate exit is blocked if measured results exceed the target budgets below. Follow [Performance Budgets](../performance-budgets.md) for hardware classes, sampling, evidence, and regression rules.

## Test Environment

- OS:
- CPU:
- GPU:
- RAM:
- Driver/runtime versions:
- Build profile:

## Benchmarks

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Package validation startup | <= 1.0 s | TBD | Local package harness |
| Frame CPU/GPU | N/A | TBD | Package operation gate; runtime impact measured by consuming gates |
| Memory overhead | <= 256 MiB extra | TBD | During verify/install over base app |
| Verify 100 MiB package | <= 5.0 s | TBD | Hash, signature, version, and platform checks |
| Atomic activation | <= 250 ms | TBD | Metadata pointer switch |
| Rollback | <= 500 ms | TBD | Previous known-good package restore |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture validation, activation, rollback, and peak memory.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
