# Gate 10 Performance Test Report

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
| Gameplay foundation startup | <= 5.0 s | TBD | Physics + animation validation scene |
| Steady frame CPU p95 | <= 10.0 ms | TBD | 1k dynamic bodies and 100 animated skeletons |
| Steady frame GPU p95 | <= 12.0 ms | TBD | Debug draw off/on captured separately |
| Memory usage | <= 1.5 GiB | TBD | After warm-up |
| Physics fixed step | <= 4.0 ms | TBD | 1k dynamic bodies |
| Animation evaluation | <= 3.0 ms | TBD | 100 skeletons |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
