# Gate 1 Performance Test Report

## Status

Partially measured during the Gate 1 implementation pass. Compile/feature checks passed and the release sandbox startup smoke is within budget. Peak memory still needs a reliable harness before the performance row can be considered fully closed.

## Test Environment

- OS: Windows
- CPU: Not captured in this pass
- GPU: N/A for Gate 1
- RAM: Not captured in this pass
- Driver/runtime versions: N/A for Gate 1 backend stubs
- Build profile: `--release` for sandbox startup; `dev`/`test` profiles for compile and unit validation

## Benchmarks

| Test | Target | Result | Notes |
|---|---:|---:|---|
| Empty sandbox startup | <= 300 ms | 267.4109 ms | Measured with `.\target\release\sandbox.exe workspace` via `System.Diagnostics.Stopwatch`, output redirected |
| Frame CPU/GPU | N/A | N/A | Gate 1 has no renderer frame budget |
| Memory usage | <= 64 MiB | Not captured | `Start-Process` / `System.Diagnostics.Process` peak working set returned `0`; needs a better Windows harness |
| Backend-disabled workspace check | No optional native SDK required | Pass | `cargo check --workspace` and backend feature checks passed with stub backends |

## Findings

- Summary: Workspace and backend stub checks passed; release sandbox startup meets the Gate 1 target in the captured run.
- Bottlenecks: None observed at Gate 1 scope.
- Regressions: None observed; memory measurement remains open.

## Follow-Up Actions

- [ ] Capture p50/p95/max where runtime sampling exists.
- [ ] Add a reliable Windows peak-memory harness for short-lived sandbox commands.
- [ ] Investigate any result over the target budget.
- [ ] Update the global budget only if the gate's scope changes.
