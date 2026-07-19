# Procedural generation primitives (PROCGEN-v1)

`crates/engine-procgen` provides the deterministic seed and noise primitives
that every procedural system (terrain, placement, variation) builds on. The
crate is game-agnostic and dependency-light (serde only).

## What is provided

| Primitive | Type | Notes |
| --- | --- | --- |
| Seed derivation | `Seed`, `derive_seed(parent, key)` | versioned FNV-1a-64 + splitmix64 |
| 2D gradient noise | `GradientNoise2D` | Perlin-style, output `[-1, 1]` |
| 3D gradient noise | `GradientNoise3D` | Perlin-style, output `[-1, 1]` |
| fBm | `Fbm2D` / `Fbm3D` + `FbmParams` | octaves, lacunarity, gain, amplitude, frequency, offset, normalize |
| Domain warp | `WarpedFbm2D` / `WarpedFbm3D` + `WarpParams` | warp channels derived from the base seed |

Every sampler has `sample(...)` plus `sample_batch(coords, out)` for
chunk-generation throughput; batch results are bit-identical to per-coordinate
sampling (tested).

## Determinism guarantee

For a given `PROCGEN_SCHEMA` version, seed, recipe, and input coordinates,
every sampler returns `f32` values with **identical bit patterns on every
platform, endianness, compiler, and process** — in Rust and in the C# port
(`Engine.ProcGen`). This holds because the implementation uses only:

- wrapping integer arithmetic (`u64`/`i64`; `unchecked` `ulong`/`long` in C#),
- exact IEEE-754 binary32 operations (`+`, `-`, `*` and one final `/` in the
  fBm accumulation) with **no FMA contraction** and **no transcendental
  functions** (`sin`/`cos`/`pow`/`sqrt` are never used),
- exact integer→`f32` conversions (all magnitudes < 2^24) and exact
  power-of-two scaling.

Concretely, coordinates snap to a Q24.8 fixed-point lattice
(`floor(c * 256)`): an `i32` lattice cell and an in-cell fraction in
`0..=255`. Corner gradients come from an integer lattice hash (FNV-1a-style
wrapping mixing + splitmix64 finalizer), dot products are small exact
integers, and the quintic fade/interpolation runs entirely in `i64` fixed
point. Only the final scale by 2^-9 is floating point, and it is exact.

### Input domain

- Supported coordinate range: `|c| < 2^23` world units (after recipe
  offset/frequency scaling). Larger *finite* magnitudes saturate
  deterministically to the lattice extremes.
- Non-finite coordinates (NaN, ±∞) make the base noise sample exactly `0.0`;
  samplers never return NaN.
- Recipes are validated at construction: octaves `1..=32`, frequency in
  `(0, 65536]`, amplitude in `[0, 65536]`, lacunarity in `(0, 16]`, gain in
  `[0, 1]`, finite offsets. `0` octaves is a validation error, not a silent
  zero.

## Versioning policy

- `PROCGEN_SCHEMA` (currently `PROCGEN-v1`) tags the whole algorithm family:
  `PROCGEN-SEED-v1` (derive_seed), `PROCGEN-NOISE-v1` (gradient noise),
  `PROCGEN-FBM-v1` (fBm), `PROCGEN-WARP-v1` (domain warp).
- **Any** change to hashing, noise, fBm, or warp math must bump the schema,
  regenerate the golden vectors in the same commit, and treat the vector
  diff as the review artifact for the algorithm change.
- Recipes and save data should record the schema they were authored against;
  a schema mismatch means "different output", never "upgrade in place".

## Golden vectors (CI determinism gate)

`crates/engine-procgen/tests/golden_vectors.json` records fixed
seed/recipe/coordinate sets with expected outputs as exact IEEE-754 bit
patterns (`f32::to_bits`). `tests/golden_vectors.rs` regenerates every vector
and compares the full document; the same file also gates the C# port (see
below). To regenerate after an intentional, schema-bumping change:

```sh
cargo run -p engine-procgen --example generate_golden_vectors \
  > crates/engine-procgen/tests/golden_vectors.json
```

## Recipe data model

All parameters are plain serde structs so recipes can be authored as data
files (JSON/RON):

```json
{
  "seed": 42,
  "params": {
    "octaves": 4,
    "frequency": 0.03125,
    "amplitude": 1.0,
    "lacunarity": 2.0,
    "gain": 0.5,
    "offset": [0.0, 0.0, 0.0],
    "normalize": true
  }
}
```

## C# script exposure

The gameplay SDK exposes `Engine.ProcGen` with `DeriveSeed(parent, key)`,
`Noise2D(seed, x, y)`, `Noise3D(seed, x, y, z)`, `Fbm2D/Fbm3D(...)`, and
`WarpedFbm2D/WarpedFbm3D(...)` (with `ProcGenFbmParams`/`ProcGenWarpParams`).

**Decision: a verified C# port, not a protocol query.** Noise is a pure,
high-throughput function (terrain chunks sample thousands of points per
frame), so routing it through the frame-context/command protocol as a
deferred query would add protocol surface and a full frame of latency for no
correctness benefit. Instead the SDK contains a bit-exact C# port of the
Rust implementation, evaluated synchronously in the script process with zero
protocol churn. Parity is enforced, not assumed:
`crates/sandbox/tests/procgen_parity.rs` compiles the materialized SDK
source with `scripts/csharp/ProcGenParity/Program.cs` and checks the port
against the **same** `golden_vectors.json` as the Rust tests (requires the
.NET SDK; reports a capability SKIP otherwise).

## Verified platforms

- Windows 11, x86-64, Rust 1.94.1 (MSVC): unit + golden-vector tests green.
- The algorithm is platform-independent by construction (see the guarantee
  above); the golden-vector gate is the regression tripwire for any future
  platform (Linux, ARM64) or runtime (.NET) that fails to reproduce the
  exact bits.
