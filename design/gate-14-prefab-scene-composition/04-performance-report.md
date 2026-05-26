# Gate 14 Performance Test Report

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
| Prefab scene startup | <= 6.0 s | TBD | 1k prefab instances with overrides and nested prefabs |
| Steady frame CPU p95 | <= 12.0 ms | TBD | Runtime prefab scene |
| Steady frame GPU p95 | <= 12.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.8 GiB | TBD | After warm-up |
| Instantiate 1k prefabs | <= 500 ms | TBD | Simple prefab fixture |
| Diff/apply overrides | <= 100 ms | TBD | Editor/runtime override fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
