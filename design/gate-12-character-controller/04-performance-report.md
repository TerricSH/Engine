# Gate 12 Performance Test Report

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
| Character scene startup | <= 5.5 s | TBD | 32 controller agents with locomotion animation |
| Steady frame CPU p95 | <= 11.0 ms | TBD | Movement, physics, and animation synchronization |
| Steady frame GPU p95 | <= 12.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.7 GiB | TBD | After warm-up |
| Controller update | <= 1.5 ms | TBD | 32 agents; no transform conflict diagnostics |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
