# Gate 16 Performance Test Report

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
| Audio scene startup | <= 6.0 s | TBD | 32 2D/3D sources with listener movement |
| Steady frame CPU p95 | <= 12.5 ms | TBD | Runtime scene with audio components |
| Steady frame GPU p95 | <= 13.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.9 GiB | TBD | After warm-up |
| Audio mix callback | No underrun | TBD | Callback must not block on asset IO |
| Decode start latency | <= 100 ms | TBD | First playback of cooked audio asset |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
