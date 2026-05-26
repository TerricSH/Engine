# Gate 7 Performance Test Report

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
| Contract validation startup | <= 500 ms | TBD | 1 MiB manifest + interpreted logic fixture |
| Frame CPU/GPU | N/A | TBD | Contract-only gate |
| Memory usage | <= 128 MiB | TBD | Validation process |
| Compatibility validation | <= 50 ms | TBD | Desktop, Android, and iOS profile checks |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture validation latency and peak memory.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
