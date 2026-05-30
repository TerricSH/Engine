# Gate 3 Performance Test Report

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
| Static scene startup | <= 2.5 s | TBD | Camera, mesh, material, and light fixture |
| Steady frame CPU p95 | <= 5.0 ms | TBD | Includes renderer input build |
| Steady frame GPU p95 | <= 10.0 ms | TBD | Desktop baseline |
| Memory usage | <= 640 MiB | TBD | After warm-up |
| Renderer input build | <= 1.0 ms | TBD | Deterministic snapshot fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
