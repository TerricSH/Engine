# Gate 5 Performance Test Report

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
| Minimal editor startup | <= 4.0 s | TBD | Editor scene, asset registry, sample C# script |
| Steady frame CPU p95 | <= 8.0 ms | TBD | Editor idle/runtime validation scene |
| Steady frame GPU p95 | <= 11.0 ms | TBD | Desktop baseline |
| Memory usage | <= 1.0 GiB | TBD | Editor + runtime validation |
| Incremental cook | <= 1.0 s | TBD | One changed mesh/material/script input |
| Script callback batch | <= 1.0 ms | TBD | 1k simple callbacks |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for frame CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
