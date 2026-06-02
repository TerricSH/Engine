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

### Desktop Baseline

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Gameplay loop startup | <= 7.0 s | TBD | Menu -> load -> play -> pause -> save -> game-over |
| Steady frame CPU p95 | <= 13.0 ms | TBD | Complete gameplay loop scene |
| Steady frame GPU p95 | <= 13.0 ms | TBD | Desktop baseline |
| Memory usage | <= 2.0 GiB | TBD | After warm-up |
| State transition | <= 100 ms | TBD | Menu/pause/game-over transitions |
| Checkpoint save | <= 250 ms | TBD | Local save fixture |

### Mobile Simulator

Per `FD-005`, mobile simulator numbers are mandatory from Gate 5 onward. The simulator runs the same release build with the `target-mobile` feature combination (per `FD-010`) and a reduced loop fixture.

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Gameplay loop startup | <= 9.0 s | TBD | Reduced loop fixture, mobile profile |
| Steady frame CPU p95 | <= 16.0 ms | TBD | Mobile simulator profile |
| Steady frame GPU p95 | <= 16.0 ms | TBD | Mobile simulator profile |
| Memory usage | <= 1.3 GiB | TBD | After warm-up, mobile profile |
| State transition | <= 150 ms | TBD | Mobile profile Menu/pause/game-over transitions |
| Checkpoint save | <= 400 ms | TBD | Local save fixture, mobile profile |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
