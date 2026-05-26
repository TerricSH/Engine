# Gate 2 Performance Test Report

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
| Vulkan sandbox startup | <= 2.0 s | TBD | Clear-color + triangle fixture |
| Steady frame CPU p95 | <= 4.0 ms | TBD | Desktop baseline |
| Steady frame GPU p95 | <= 8.0 ms | TBD | Desktop baseline GPU timers where available |
| Memory usage | <= 512 MiB | TBD | After warm-up |
| Swapchain recreate | <= 250 ms | TBD | Resize/minimize recovery path |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
