# Gate 18 Performance Test Report

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
| Gameplay loop startup | <= 7.0 s | TBD | Menu -> load -> play -> pause -> save -> game-over |
| Steady frame CPU p95 | <= 13.0 ms | TBD | Complete gameplay loop scene |
| Steady frame GPU p95 | <= 13.0 ms | TBD | Desktop baseline |
| Memory usage | <= 2.0 GiB | TBD | After warm-up |
| State transition | <= 100 ms | TBD | Menu/pause/game-over transitions |
| Checkpoint save | <= 250 ms | TBD | Local save fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
