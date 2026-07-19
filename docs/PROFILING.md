# Frame-Time Profiling (ENG-04)

The engine attributes per-frame CPU and GPU time to individual stages and
render passes. Results are consumable by tooling (editor diagnostics) and by
headless CI (the project run report).

## Where time is measured

### CPU stages (all backends)

The engine runtime records wall-clock time at stage boundaries — clock reads
only at `begin`/`end`, no per-draw overhead:

| Stage                | Recorded in | Covers |
| -------------------- | ----------- | ------ |
| `update`             | `GameLoop::update` | Physics step, character controllers, navigation, scripts, animation, audio |
| `script_tick`        | nested inside `update` | Script `OnUpdate` ticking only (excluded from `update`'s own time) |
| `extraction`         | `EngineRuntime::render_frame_submission` | ECS → `RenderFrameInput` extraction, render-extension producers, UI batch merge |
| `sync_render_assets` | `EngineRuntime::render_frame_submission` | Mesh/texture/material upload validation and backend sync |
| `render_submit`      | `EngineRuntime::render_frame_submission` | `Renderer::draw_scene` — render-graph build, pass execution, submit/present |

Nested stages pause their parent, so the sum of stage times equals
`total_cpu_ms` and approximates the measured frame path. A failed render
discards the partially recorded frame instead of polluting statistics.

### GPU passes (Vulkan only)

The Vulkan `SceneRenderer` brackets every render-graph pass with timestamp
queries: start at `TOP_OF_PIPE`, end at `BOTTOM_OF_PIPE`. Pass names are the
render-graph node names (`directional_shadow_pass`,
`opaque_pbr_forward_pass`, `tone_map_pass`, `present`, plus registered custom
pass names). With multiple views the names emitted are those of the base
camera's passes, as produced by the graph.

Design:

- One pre-allocated query pool per frame-in-flight slot (currently 2), each
  with capacity for 32 passes (64 timestamps). Excess passes are skipped from
  GPU attribution (CPU timing still covers the frame).
- **Asynchronous read-back**: results for the frame recorded on a slot are
  collected the next time that slot's in-flight fence is waited — i.e. with a
  frames-in-flight delay — via non-blocking `vkGetQueryPoolResults` (no
  `WAIT` flag). The pipeline is never stalled for profiling.
- **Calibration**: raw ticks are converted with the device's
  `timestampPeriod` limit (nanoseconds per tick).
- **Graceful degradation**: when `timestampComputeAndGraphics` or a valid
  `timestampPeriod` is missing, or pool creation fails, the backend reports
  `unavailable` (with a static reason in logs) instead of failing the frame.
  A failed read-back drops that frame's GPU samples and keeps going
  (`tracing::warn`, counted as a lost frame).

Because read-back is asynchronous, the GPU samples attached to a frame's
statistics describe an earlier frame; `FrameStats.gpu_pass_frame_index` /
`FrameTimings.gpu_frame_index` name the frame the samples were recorded on.

## Stats contract (backend-neutral, `engine-renderer::frame_timing`)

```rust
pub struct PassTiming {
    pub name: String,
    pub cpu_ms: Option<f32>,   // CPU stages
    pub gpu_ms: Option<f32>,   // GPU passes (backend-reported)
}

pub struct FrameTimings {
    pub frame_index: u64,
    pub passes: Vec<PassTiming>,
    pub total_cpu_ms: f32,              // sum of CPU stage times
    pub total_gpu_ms: Option<f32>,      // sum when any GPU sample exists
    pub gpu_status: GpuTimingStatus,    // available | pending | unavailable | disabled
    pub gpu_frame_index: Option<u64>,   // frame the GPU samples belong to
}
```

`GpuTimingStatus`:

- `available` — GPU timestamps recorded and read back.
- `pending` — backend records timestamps but the first async read-back has
  not landed yet.
- `unavailable` — backend/device cannot provide GPU timestamps (also the
  state for the OpenGL, DX12, recording, and headless QA backends).
- `disabled` — turned off by configuration (see below).

### Rolling statistics

`FrameTimingTracker` keeps the last 120 frames (`DEFAULT_TIMING_WINDOW`) and
produces a `FrameTimingSummary` with per-pass `avg_ms` / `p95_ms`
(nearest-rank) / `max_ms` / `samples`, plus total-CPU and total-GPU
aggregates. Access it via:

- `EngineRuntime::frame_timing_summary()` / `EngineRuntime::last_frame_timings()`
- `GameLoop::frame_timing_summary()`

## Configuration

`EngineConfig::gpu_timestamps: bool` (default `true`) is forwarded to the
backend when it is installed (`EngineRuntime::set_renderer_backend` →
`BackendRenderer::set_gpu_timing_enabled`). `false` disables GPU timestamp
recording entirely (status `disabled`, nothing measured); CPU stage timing is
unaffected. The Vulkan backend re-evaluates the switch on the next frame.

## Consumption

### Headless run report

`run_headless` (sandbox `project_app.rs`) emits a `frame_timing` section in
the run report JSON — the serialized `FrameTimingSummary`:

```json
"frame_timing": {
  "window_frames": 3,
  "window_capacity": 120,
  "gpu_status": "unavailable",
  "passes": [
    { "name": "update",
      "cpu": { "samples": 3, "avg_ms": 0.42, "p95_ms": 0.61, "max_ms": 0.70 } },
    { "name": "render_submit",
      "cpu": { "samples": 3, "avg_ms": 1.10, "p95_ms": 1.30, "max_ms": 1.40 },
      "gpu": { "samples": 3, "avg_ms": 0.90, "p95_ms": 1.00, "max_ms": 1.10 } }
  ],
  "total_cpu": { "samples": 3, "avg_ms": 2.01, "p95_ms": 2.40, "max_ms": 2.50 }
}
```

The `gpu` aggregate and `total_gpu` are **absent** (not null) when no GPU
samples exist; `gpu_status` tells apart `unavailable`, `pending`, `disabled`,
and `available`. The headless QA backend always reports `unavailable`, so CI
soak tests must treat GPU fields as optional.

### Editor diagnostics

`EngineRuntime::runtime_diagnostics()` (`RuntimeDiagnostics.frame_timing`)
carries the rolling summary, and the sandbox-level `SandboxDiagnostics`
snapshot copies it every frame (`SandboxDiagnostics.frame_timing`). Both are
read-only typed snapshots. Surfacing the numbers in the editor UI (performance
panel / React shell) is a documented follow-up: consume
`SandboxDiagnostics::frame_timing` — no additional engine plumbing required.

## Limits

- **OpenGL / DX12 GPU timing is not implemented.** Those backends report CPU
  stages through the shared runtime path and `gpu_status: unavailable`.
- GPU samples lag by the frames-in-flight count (currently 2 frames); check
  `gpu_frame_index` when correlating with a specific CPU frame.
- Per-pass GPU times measure queue execution between the bracketing
  timestamps; they overlap where passes overlap in the pipeline and may not
  sum exactly to wall GPU frame time.
- At most 32 passes are timestamped per frame.
- `FrameStats.gpu_frame_ms` predates this system and is still reported
  separately by the device; the per-pass breakdown lives in the new
  `gpu_pass_times` field.
