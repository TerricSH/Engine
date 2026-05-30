# Gate 17 Performance Test Report

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
| Production editor startup | <= 8.0 s | TBD | Production scene with gizmos, browser, prefab diff, inspector |
| Editor idle CPU p95 | <= 14.0 ms | TBD | Editor build/profile labeled |
| Editor GPU p95 | <= 14.0 ms | TBD | Editor viewport |
| Memory usage | <= 2.5 GiB | TBD | Editor scene after warm-up |
| Selection/inspector update | <= 50 ms | TBD | Complex entity fixture |
| Search 10k assets | <= 200 ms | TBD | Asset browser fixture |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for editor CPU and GPU.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
