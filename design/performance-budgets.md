# Performance Budgets

This document defines the first measurable performance budget for every gate. A gate may finish with better results than these numbers, but it must not merge with worse results unless the budget is deliberately changed here and the affected downstream gates are reviewed.

## Measurement Protocol

All numbers are measured from release builds unless a gate explicitly tests editor/debug behavior.

| Rule | Requirement |
|---|---|
| Build profile | `--release` for runtime budgets; debug/editor tools must label their build profile. |
| Warm-up | Run at least 120 frames before steady-frame sampling unless the test is a cold-start or package operation. |
| Sampling window | Capture at least 600 frames or 10 seconds, whichever is longer. |
| Reported values | Record p50, p95, and max for frame CPU, frame GPU, memory, and operation latency. |
| Evidence | Store command, hardware profile, build profile, scene/fixture name, logs, screenshots where visual parity matters, and machine-readable JSON or CSV when available. |
| Failure rule | p95 over budget blocks the gate; a single max spike over 2x budget blocks the gate unless the spike is a documented cold-start, compile, or one-time import operation. |

## Baseline Hardware Classes

| Class | Purpose | Minimum reference profile |
|---|---|---|
| Desktop baseline | Main gate exit target | 6-core desktop CPU, 16 GiB RAM, Vulkan-capable discrete GPU with 4 GiB VRAM, SSD. |
| Integrated GPU | Compatibility smoke target | 4-core CPU, 8 GiB RAM, integrated Vulkan-capable GPU, SSD. |
| Mobile simulator | Mobile policy and package validation | Desktop simulator profile with Android/iOS platform flags and mobile asset profile. |
| Mobile device | Release candidate target | Mid-range Android device and iOS device class selected by the release owner in Gate 19. |

Desktop baseline is mandatory from Gate 2 onward. Integrated GPU is mandatory for renderer smoke tests from Gate 3 onward. Mobile simulator is mandatory from **Gate 5** onward (per `FD-005`) and every Gate 5-19 performance report must dual-report desktop and mobile simulator numbers for its required fixture. Real mobile device numbers become mandatory in Gate 19.

## Budget Categories

| Category | Definition |
|---|---|
| Startup / load | Time from process start or command invocation to the first usable frame or completed validation result. |
| Steady frame CPU | Main-thread plus scheduled engine work for the gate's validation scene, excluding GPU wait caused by presentation throttling. |
| Steady frame GPU | GPU time for the rendered validation scene when GPU timers are available. |
| Operation latency | Blocking user-visible operation such as hot reload, package activation, scene load, cook, or editor action. |
| Peak memory | Resident memory for the process after warm-up or during the named operation. |
| Regression allowance | A later gate may consume unused budget from previous gates only if the aggregate frame budget remains under 16.6 ms p95 on desktop baseline. |

## Per-Gate Budgets (Desktop Baseline)

Desktop baseline numbers apply to every gate. Gates 5-19 also have a parallel mobile simulator budget table below (per `FD-005`).

| Gate | Required benchmark fixture | Startup / load | Steady frame CPU p95 | Steady frame GPU p95 | Peak memory | Operation latency / spike budget |
|---|---|---:|---:|---:|---:|---:|
| 1 Workspace/RHI | Empty sandbox and backend enumeration | <= 300 ms | N/A | N/A | <= 64 MiB | `cargo check --workspace` must not require optional native SDKs for disabled backends |
| 2 Vulkan Renderer | Clear-color + triangle Vulkan sandbox | <= 2.0 s | <= 4.0 ms | <= 8.0 ms | <= 512 MiB | Swapchain recreate <= 250 ms |
| 3 Scene Rendering | Gate 3 static scene with camera, mesh, material, and light | <= 2.5 s | <= 5.0 ms | <= 10.0 ms | <= 640 MiB | Renderer input build <= 1.0 ms |
| 4 ECS Scene Runtime | `scene_gate04_valid.ron`, 10k entity synthetic extraction | <= 3.0 s | <= 6.0 ms | <= 10.0 ms | <= 768 MiB | Scene load/validate <= 500 ms |
| 5 Content Authoring | Minimal editor scene, asset cook, sample C# script | <= 4.0 s | <= 8.0 ms | <= 11.0 ms | <= 1.0 GiB | Incremental cook <= 1.0 s; 1k script callbacks <= 1.0 ms |
| 6 Iteration Workflow | Asset/script hot reload scene | <= 4.0 s | <= 8.0 ms | <= 11.0 ms | <= 1.1 GiB | Successful reload <= 250 ms; failed reload rollback <= 100 ms; no frame spike > 16.6 ms |
| 7 Mobile/Hot Update Contracts | 1 MiB manifest + interpreted logic schema validation | <= 500 ms | N/A | N/A | <= 128 MiB | Compatibility validation <= 50 ms |
| 8 Hot Update Package | 100 MiB local package with assets and logic payloads | <= 1.0 s validation startup | N/A | N/A | <= 256 MiB extra over base app | Verify <= 5.0 s; activate <= 250 ms; rollback <= 500 ms |
| 9 Subsystem Extension Contracts | Register physics, animation, UI, audio mock extensions | <= 3.0 s | <= 1.0 ms extension dispatch overhead | N/A | <= 128 MiB extension metadata | Register 100 descriptors <= 10 ms |
| 10 Gameplay Foundation | 1k dynamic bodies, 100 animated skeletons, debug draw off/on | <= 5.0 s | <= 10.0 ms | <= 12.0 ms | <= 1.5 GiB | Physics step <= 4.0 ms; animation eval <= 3.0 ms |
| 11 Gameplay Expansion | Joint/blend/state-machine integration scene | <= 5.5 s | <= 11.0 ms | <= 12.0 ms | <= 1.7 GiB | Batched scene queries <= 2.0 ms |
| 12 Character Controller | 32 controller agents with locomotion animation | <= 5.5 s | <= 11.0 ms | <= 12.0 ms | <= 1.7 GiB | Controller update <= 1.5 ms; no transform conflict diagnostics |
| 13 Navigation/AI | 200 agents on cooked navmesh with patrol/chase | <= 6.0 s | <= 12.0 ms | <= 12.0 ms | <= 1.8 GiB | 200 path queries <= 5.0 ms amortized |
| 14 Prefab/Scene Composition | 1k prefab instances with overrides and nested prefabs | <= 6.0 s | <= 12.0 ms | <= 12.0 ms | <= 1.8 GiB | Instantiate 1k simple prefabs <= 500 ms; diff/apply <= 100 ms |
| 15 Runtime UI | Gameplay scene with 500 UI nodes and text | <= 6.0 s | <= 12.5 ms | <= 13.0 ms | <= 1.9 GiB | Layout 500 nodes <= 2.0 ms; hit test <= 0.5 ms |
| 16 Audio System | 32 2D/3D sources with listener movement | <= 6.0 s | <= 12.5 ms | <= 13.0 ms | <= 1.9 GiB | Audio mix callback must not underrun; asset decode start <= 100 ms |
| 17 Production Editor Tools | Production editor scene with gizmos, browser, prefab diff, inspector | <= 8.0 s editor startup | <= 14.0 ms editor idle frame | <= 14.0 ms | <= 2.5 GiB | Selection/inspector update <= 50 ms; search 10k assets <= 200 ms |
| 18 Gameplay Framework | Menu -> load -> play -> pause -> save -> game-over loop | <= 7.0 s | <= 13.0 ms | <= 13.0 ms | <= 2.0 GiB | State transition <= 100 ms; checkpoint save <= 250 ms |
| 19 Release Pipeline | Packaged desktop and mobile candidate smoke suite | Desktop <= 5.0 s; mobile target recorded | Desktop <= 12.0 ms; mobile target recorded | Desktop <= 12.0 ms; mobile target recorded | Desktop <= 2.0 GiB; mobile target recorded | Package build, signing, QA, diagnostics, and rollback must emit regression reports |

## Per-Gate Budgets (Mobile Simulator, Gates 5-19)

Mobile simulator numbers are mandatory from Gate 5 onward (`FD-005`). The simulator runs the same release build with the `target-mobile` feature combination (per `FD-010`) and the mobile asset profile. Real device numbers are still owned by Gate 19; these simulator numbers are policy/cost guardrails that catch the largest mobile regressions early.

| Gate | Mobile fixture / profile | Startup / load | Steady frame CPU p95 | Steady frame GPU p95 | Peak memory | Operation latency / spike budget |
|---|---|---:|---:|---:|---:|---:|
| 5 Content Authoring | Cooked mobile asset profile + sample C# script (NativeAOT trim per `FD-001`) | <= 6.0 s | <= 12.0 ms | <= 14.0 ms | <= 768 MiB | Incremental mobile cook <= 1.5 s; 1k script callbacks <= 2.0 ms |
| 6 Iteration Workflow | Asset hot reload mobile profile (no script hot reload, per `FD-001`) | <= 6.0 s | <= 12.0 ms | <= 14.0 ms | <= 800 MiB | Successful asset reload <= 350 ms; rollback <= 150 ms |
| 7 Mobile/Hot Update Contracts | 1 MiB mobile manifest validation | <= 800 ms | N/A | N/A | <= 96 MiB | Compatibility validation <= 75 ms |
| 8 Hot Update Package | 100 MiB package on mobile profile | <= 1.5 s validation startup | N/A | N/A | <= 200 MiB extra | Verify <= 7.5 s; activate <= 400 ms; rollback <= 700 ms |
| 9 Subsystem Extension Contracts | Same as desktop, with `target-mobile` feature | <= 4.0 s | <= 1.5 ms dispatch overhead | N/A | <= 96 MiB | Register 100 descriptors <= 15 ms |
| 10 Gameplay Foundation | Reduced fixture: 250 dynamic bodies, 25 skeletons | <= 7.0 s | <= 14.0 ms | <= 16.0 ms | <= 1.1 GiB | Physics step <= 6.0 ms; animation eval <= 5.0 ms |
| 11 Gameplay Expansion | Reduced joint/blend scene | <= 7.5 s | <= 15.0 ms | <= 16.0 ms | <= 1.2 GiB | Batched scene queries <= 3.0 ms |
| 12 Character Controller | 8 controller agents | <= 7.5 s | <= 15.0 ms | <= 16.0 ms | <= 1.2 GiB | Controller update <= 2.5 ms |
| 13 Navigation/AI | 50 agents on cooked navmesh | <= 8.0 s | <= 16.0 ms | <= 16.0 ms | <= 1.3 GiB | 50 path queries <= 5.0 ms amortized |
| 14 Prefab/Scene Composition | 250 prefab instances | <= 8.0 s | <= 16.0 ms | <= 16.0 ms | <= 1.3 GiB | Instantiate 250 prefabs <= 600 ms |
| 15 Runtime UI | 200 UI nodes mobile scene | <= 8.0 s | <= 16.0 ms | <= 17.0 ms | <= 1.3 GiB | Layout 200 nodes <= 3.0 ms; hit test <= 0.75 ms |
| 16 Audio System | 16 2D/3D sources, mobile sample rate (cpal+symphonia per `FD-017`) | <= 8.0 s | <= 16.0 ms | <= 17.0 ms | <= 1.3 GiB | Audio mix callback must not underrun on mobile; asset decode start <= 150 ms |
| 17 Production Editor Tools | N/A (editor is desktop-only per `FD-011`) | N/A | N/A | N/A | N/A | Mobile column is intentionally empty; report N/A in `04-performance-report.md` |
| 18 Gameplay Framework | Reduced loop fixture | <= 9.0 s | <= 16.0 ms | <= 16.0 ms | <= 1.3 GiB | State transition <= 150 ms; checkpoint save <= 400 ms |
| 19 Release Pipeline | Packaged mobile candidate smoke suite | Recorded per device class | Recorded per device class | Recorded per device class | Recorded per device class | Real device numbers replace simulator numbers; both reported |

The exact mobile simulator hardware profile is recorded under `OFQ-002` in `foundation-decisions.md`; the Gate 5 owner is responsible for choosing it and pinning it before the first dual-report.

## Per-Subsystem Sub-Budgets

These sub-budgets carve specific operations out of the per-gate latency rows above. They exist because the gate-level rows aggregate many activities (full cook, hot-reload, device-init, frame work) and a regression in a single sub-operation can hide inside an aggregate budget that still passes. Each row below MUST also satisfy its owning gate's aggregate budget.

### Shader Cook & PSO (`FD-004`, `FD-037`–`FD-042`)

| Operation | Desktop p95 | Mobile simulator p95 | Owning gate | Notes |
|---|---:|---:|---|---|
| Single-pipeline cook (1 vert+frag pair, 1 variant, fresh `shaderc` invocation) | <= 200 ms | <= 350 ms | Gate 5 | Mobile adds `naga` SPIR-V → GLSL cross-compile when `backend-opengl` is enabled. `backend-dx12` adds a further `naga` SPIR-V → HLSL + DXC HLSL → DXIL pass measured separately. |
| Per-pipeline DXIL emission (`backend-dx12` enabled) | <= 250 ms | N/A (no DX12 on mobile) | Gate 5 | DXC compile only; excluded when `backend-dx12` is disabled. |
| Variant fan-out per pipeline | warn at 256, hard cap 1024 | warn at 128, hard cap 512 | Gate 5 | Soft warning is a cook diagnostic (`SH00xx` to be allocated when implemented); the hard cap is enforced before the Cartesian expansion begins and reuses the bit-budget check in `FD-040`. Exceeding the cap blocks the cook. |
| Total shader cook share of Gate 5 incremental cook (`<= 1.0 s` desktop, `<= 1.5 s` mobile) | <= 600 ms | <= 900 ms | Gate 5 | Shader cook MUST NOT consume the full per-gate incremental cook budget; remaining time is reserved for other asset kinds. |
| Hot-reload per shader (recook + reflection + PSO recreate) | <= 300 ms | <= 500 ms | Gate 6 | Must also satisfy the Gate 6 `Successful reload <= 250 ms` aggregate when the reload touches a single small shader; the 300 ms / 500 ms ceiling here applies to larger shaders with many variants. |
| PSO cache warm-up at device init (read disk + `vkCreatePipelineCache`) | <= 100 ms | <= 200 ms | Gate 2 (consumer) / Gate 1 (RHI) | Cold-start (cache miss / fresh machine) is excluded; it falls under the Gate 2 / Gate 6 first-frame PSO creation cost, which itself MUST NOT cause a single frame > 16.6 ms (per the Gate 6 spike rule above). |
| PSO cache file size on disk | <= 32 MiB per backend | <= 16 MiB per backend | Gate 2 | Soft budget; cache is per-machine and never shipped (`FD-042`). A larger cache is a hint that the cook is producing too many distinct pipelines and should be reviewed against the variant fan-out cap. |

### Other Subsystems

Future sub-budgets (asset cook by type, prefab instantiation, navmesh build, UI layout per node count, audio decode latency by codec) will be added here by their owning gates as those gates freeze. Owning gate is responsible for defining the measurement protocol and the desktop / mobile pair.

## Regression Rules

1. A gate may only relax a budget by editing this file and explaining which fixture, platform, or requirement changed.
2. If a later gate exceeds an earlier subsystem budget, the failure belongs to the later gate unless evidence proves the earlier gate's benchmark was invalid.
3. Editor-only overhead must not be hidden inside runtime budgets. Runtime and editor measurements are reported separately.
4. Mobile policy gates must report validation latency even before real device runtime benchmarks exist.
5. Gate 19 owns final platform-specific thresholds. Earlier mobile numbers are compatibility and validation budgets, not shipping FPS promises.

## Per-Gate Report Format

Every `04-performance-report.md` should copy the relevant target rows from this document and fill `Result` with measured values. For Gate 5 and later, the report must include both desktop and mobile simulator rows (per `FD-005`). The report must include:

- hardware class and exact machine details (for both desktop and mobile simulator from Gate 5 onward);
- build command and feature flags (mobile rows include the `target-mobile` feature combination per `FD-010`);
- fixture or scene name (mobile fixtures may be a reduced variant of the desktop fixture; document the reduction);
- p50/p95/max for frame metrics on each profile;
- operation-specific latency for cook, reload, package, prefab, editor, or QA steps on each profile;
- blocking regressions and owner for follow-up.
