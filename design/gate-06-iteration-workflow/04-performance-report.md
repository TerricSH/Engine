# Gate 6 Performance Test Report

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
| Hot reload scene startup | <= 4.0 s | TBD | Asset/script hot reload fixture |
| Steady frame CPU p95 | <= 8.0 ms | TBD | During idle and after reload |
| Steady frame GPU p95 | <= 11.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.1 GiB | TBD | After repeated reloads |
| Successful reload | <= 250 ms | TBD | From file change detection to active resource swap |
| Failed reload rollback | <= 100 ms | TBD | No frame spike over 16.6 ms |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
