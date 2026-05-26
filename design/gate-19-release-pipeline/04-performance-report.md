# Gate 19 Performance Test Report

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
| Packaged desktop startup | <= 5.0 s | TBD | Release candidate smoke suite |
| Desktop frame CPU p95 | <= 12.0 ms | TBD | Release candidate gameplay scene |
| Desktop frame GPU p95 | <= 12.0 ms | TBD | Release candidate gameplay scene |
| Desktop memory usage | <= 2.0 GiB | TBD | Packaged runtime after warm-up |
| Mobile startup/frame/memory | Recorded with release target | TBD | Gate 19 release owner sets device-specific thresholds |
| Package/QA/diagnostic reports | Required | TBD | Build, signing, QA, rollback, and diagnostic artifacts |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for desktop and selected mobile targets.
- [ ] Investigate any result over the target budget.
- [ ] Block release candidate promotion until reports and artifacts are complete.
