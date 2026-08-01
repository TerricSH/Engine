# Runtime quality gates

The workspace distinguishes recoverable runtime failures from test assertions
and documented foreign-API invariants. Text counts alone do not make that
distinction, so the executable gates below are authoritative.

## Panic-prone convenience methods

Production library and binary targets are checked with
`clippy::unwrap_used`. Unit and integration tests may use `unwrap` and
`expect` as concise assertions; `clippy.toml` records that exception. Runtime
code must instead do one of the following:

- propagate a typed `Result`;
- use an `Option` branch with an explicit fallback;
- recover a poisoned synchronization primitive while preserving its data;
- use `expect` only for a locally proven invariant, with a message that states
  the invariant rather than restating the operation.

The CI Clippy task runs all workspace targets with warnings denied, then runs a
second all-feature library/binary pass with `unwrap_used` denied. Keeping the
panic-safety pass on production targets avoids classifying integration-test
helpers as shipping runtime code. A raw source count includes `#[cfg(test)]`
modules and is therefore not a release risk measurement.

## Unsafe contracts

The Vulkan and audio crates deny undocumented unsafe blocks. Every unsafe
operation or `unsafe impl` in those crates must have an adjacent `SAFETY:`
contract describing pointer lifetime, aliasing, synchronization, ownership,
and foreign API preconditions as applicable. This documentation is part of the
review surface; it is not a substitute for validation layers or tests.

## Cross-crate contracts

`tests/workspace-contracts` is a real workspace package rather than a fixture
directory. It verifies public-API composition across independently versioned
crates, currently including:

- f64 cube-sphere terrain query, CDLOD selection, and spherical navigation on
  the same displaced surface;
- three-dimensional space navigation through authored obstacles.

Run it with:

```powershell
cargo test -p engine-workspace-contracts
```

## Performance regression coverage

Two dependency-free stable-Rust benchmark executables cover the new planetary
runtime hot paths:

```powershell
cargo bench -p engine-terrain --bench planet_runtime
cargo bench -p engine-nav --bench spatial_navigation
```

They report elapsed time and nanoseconds per iteration for cube-sphere surface
queries, terrain LOD selection, 3D A*, and spherical A*. They deliberately do
not enforce machine-independent wall-clock thresholds: developer laptops and
hosted CI runners are not comparable. CI compiles every benchmark through the
all-targets Clippy gate so benchmark code cannot rot.

The deterministic headless QA task is the automated performance regression
gate. It writes a structured report and fails when its configured average CPU
frame budget or rendered-work minimums are violated:

```powershell
./.github/scripts/ci.ps1 -Task Qa
```
