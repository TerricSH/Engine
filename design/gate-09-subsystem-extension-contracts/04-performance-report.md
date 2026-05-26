# Gate 9 Performance Test Report

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
| Extension harness startup | <= 3.0 s | TBD | Physics, animation, UI, and audio mock descriptors |
| Extension dispatch overhead | <= 1.0 ms | TBD | Per frame for registered mock systems |
| Frame GPU | N/A | TBD | Contract dispatch only |
| Extension metadata memory | <= 128 MiB | TBD | Descriptor registry |
| Register 100 descriptors | <= 10 ms | TBD | Unique ID and compatibility validation |

## Findings

- Summary:
- Bottlenecks:
- Regressions:

## Follow-Up Actions

- [ ] Capture dispatch overhead and registration latency.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
