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

### Desktop Baseline

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Packaged desktop startup | <= 5.0 s | TBD | Release candidate smoke suite |
| Desktop frame CPU p95 | <= 12.0 ms | TBD | Release candidate gameplay scene |
| Desktop frame GPU p95 | <= 12.0 ms | TBD | Release candidate gameplay scene |
| Desktop memory usage | <= 2.0 GiB | TBD | Packaged runtime after warm-up |
| Package/QA/diagnostic reports | Required | TBD | Build, signing, QA, rollback, and diagnostic artifacts |

### Mobile Simulator

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Packaged mobile startup | Recorded per device class | TBD | Gate 19 release owner sets device-specific thresholds |
| Mobile frame CPU p95 | Recorded per device class | TBD | Per-device-class measurement |
| Mobile frame GPU p95 | Recorded per device class | TBD | Per-device-class measurement |
| Mobile memory usage | Recorded per device class | TBD | Per-device-class measurement |

### Real Device (mandatory in Gate 19 per FD-005)

Real device numbers replace simulator numbers. Both are reported. The release owner selects the device classes and records thresholds here before gate exit.

| Device class | Startup / load | Frame CPU p95 | Frame GPU p95 | Peak memory | Notes |
|---|---:|---:|---:|---:|---|
| Mid-range Android | TBD | TBD | TBD | TBD | |
| Mid-range iOS | TBD | TBD | TBD | TBD | |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture p50/p95/max for desktop and selected mobile targets.
- [ ] Investigate any result over the target budget.
- [ ] Block release candidate promotion until reports and artifacts are complete.
