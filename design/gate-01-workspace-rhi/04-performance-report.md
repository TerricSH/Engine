# Gate 1 Performance Test Report

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
| Empty sandbox startup | <= 300 ms | TBD | First usable no-op frame or command completion |
| Frame CPU/GPU | N/A | TBD | Gate 1 has no renderer frame budget |
| Memory usage | <= 64 MiB | TBD | Empty workspace/sandbox process |
| Backend-disabled workspace check | No optional native SDK required | TBD | Disabled backends must compile as stubs |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max where runtime sampling exists.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
