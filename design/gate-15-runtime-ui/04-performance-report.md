# Gate 15 Performance Test Report

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
| Runtime UI scene startup | <= 6.0 s | TBD | Gameplay scene with 500 UI nodes and text |
| Steady frame CPU p95 | <= 12.5 ms | TBD | Includes layout and input dispatch |
| Steady frame GPU p95 | <= 13.0 ms | TBD | UI over gameplay |
| Memory usage | <= 1.9 GiB | TBD | After warm-up |
| Layout 500 nodes | <= 2.0 ms | TBD | Responsive layout fixture |
| Hit test | <= 0.5 ms | TBD | Pointer/focus fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
