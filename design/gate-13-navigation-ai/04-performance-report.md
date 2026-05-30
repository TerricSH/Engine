# Gate 13 Performance Test Report

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
| Navigation scene startup | <= 6.0 s | TBD | 200 agents on cooked navmesh |
| Steady frame CPU p95 | <= 12.0 ms | TBD | Patrol/chase behavior through character controller |
| Steady frame GPU p95 | <= 12.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.8 GiB | TBD | After warm-up |
| Path query batch | <= 5.0 ms amortized | TBD | 200 path queries |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
