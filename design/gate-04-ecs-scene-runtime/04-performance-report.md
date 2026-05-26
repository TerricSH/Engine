# Gate 4 Performance Test Report

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
| ECS scene startup | <= 3.0 s | TBD | `scene_gate04_valid.ron` |
| Steady frame CPU p95 | <= 6.0 ms | TBD | Includes ECS extraction |
| Steady frame GPU p95 | <= 10.0 ms | TBD | Visual parity scene |
| Memory usage | <= 768 MiB | TBD | After warm-up |
| Scene load and validation | <= 500 ms | TBD | Includes 10k entity synthetic extraction fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
