# Foundation Decisions

This document records cross-cutting engineering decisions that have been frozen for the entire engine. Each decision is identified by an `FD-###` ID and is referenced from gate documents instead of being re-debated per gate.

These decisions are **frozen** in the same sense as the `-v0` contracts: they may evolve through the contract change workflow at the end of [compatibility-error-handling.md](compatibility-error-handling.md), but no gate may silently violate them.

If a gate document and this document disagree, this document wins and the gate document must be updated.

## How To Use

1. When reading a gate's `01-code-architecture.md` or `03-best-practices.md`, treat the listed `FD-###` references as additional binding constraints.
2. When proposing a new technology or pattern in any gate, first check whether an `FD` already decides it.
3. When changing an `FD`, follow the contract change workflow: update this file, then every gate that references it, then add a migration note to the affected `02-validation-acceptance.md` files.

## Decision Index

| ID | Topic | Owner gate(s) | Status |
|---|---|---|---|
| FD-001 | .NET hosting strategy | Gate 5, Gate 7 | Frozen |
| FD-002 | Engine threading model | Gate 1, Gate 4, Gate 6, Gate 16 | Frozen |
| FD-003 | iOS graphics backend | Gate 1, Gate 2, Gate 19 | Frozen |
| FD-004 | Shader toolchain | Gate 1, Gate 2, Gate 3, Gate 5, Gate 6, Gate 19 | Frozen |
| FD-005 | Mobile budget reporting timing | Gate 5-19 | Frozen |
| FD-006 | Cooked asset binary format | Gate 5, Gate 8 | Frozen |
| FD-007 | Skinned render input shape | Gate 9, Gate 10 | Frozen |
| FD-008 | IO and async runtime model | All gates | Frozen |
| FD-009 | Schema migration mechanism | Gate 4-14 | Frozen |
| FD-010 | Cargo feature flag taxonomy | All gates | Frozen |
| FD-011 | Editor vs runtime crate split | Gate 5, Gate 17 | Frozen |
| FD-012 | Determinism policy | Gate 4+ | Frozen |
| FD-013 | Platform layer scope | Gate 1, Gate 7 | Frozen |
| FD-014 | Logging and tracing | All gates | Frozen |
| FD-015 | Vulkan GPU memory allocator | Gate 2 | Frozen |
| FD-016 | Physics backend | Gate 10 | Frozen |
| FD-017 | Audio backend | Gate 16 | Frozen |
| FD-018 | ECS implementation | Gate 4 | Frozen |
| FD-019 | File watcher | Gate 6 | Frozen |
| FD-020 | Networking scope | Entire engine | Frozen (out of scope) |
| FD-021 | CI/CD provider | Gate 19 | Frozen |
| FD-022 | Image and texture importer | Gate 5 | Frozen |
| FD-023 | 3D model import format | Gate 5 | Frozen |
| FD-024 | Rust edition and MSRV | All gates | Frozen |
| FD-025 | Source license | All gates | Frozen |
| FD-026 | Shading model and color pipeline | Gate 3, Gate 10/11, Gate 17 | Frozen |
| FD-027 | Shading pipeline (forward / forward+) | Gate 3, Gate 10/11 | Frozen |
| FD-028 | Shadow algorithm | Gate 3, Gate 10/11 | Frozen |
| FD-029 | Workspace crate layout | Gate 1, all gates | Frozen |
| FD-030 | Math library | Gate 1, all gates | Frozen |
| FD-031 | Coordinate system, units, NDC | Gate 1, all gates | Frozen |
| FD-032 | Error handling crate split | Gate 1, all library gates | Frozen |
| FD-033 | Cross-thread channel crate | All gates | Frozen |
| FD-034 | Camera component minimum field set | Gate 3, Gate 4 | Frozen |
| FD-035 | Camera stack / multi-view composition | Gate 3, Gate 4, Gate 15 | Frozen |
| FD-036 | Frustum culling ownership | Gate 3, Gate 4 | Frozen |
| FD-037 | Shader source file layout and stage convention | Gate 1, Gate 2, Gate 3, Gate 5 | Frozen |
| FD-038 | Shader include and preprocessor | Gate 5, Gate 6 | Frozen |
| FD-039 | Backend shader translation pipeline | Gate 2, Gate 5 | Frozen |
| FD-040 | Shader variant / permutation model | Gate 3, Gate 5 | Frozen |
| FD-041 | Descriptor set / bind layout convention | Gate 1, Gate 2, Gate 3, Gate 5 | Frozen |
| FD-042 | CookedShader-v0 and PSO cache | Gate 5, Gate 6 | Frozen |

Open questions are listed at the end of this document.

---

## FD-001: .NET hosting strategy

**Decision.** The engine hosts .NET in a hybrid mode: **CoreCLR on desktop**, **NativeAOT on iOS**. Android script delivery follows the Gate 7 mobile package policy and uses the same NativeAOT-compiled subset where executable payloads are not allowed; otherwise it uses CoreCLR like desktop.

**Rationale.** CoreCLR gives the editor and desktop builds full hot-reload, reflection, and JIT diagnostics. iOS forbids downloaded JIT or interpreted executable payloads, so the iOS player must use NativeAOT for all script assemblies it ships with.

**Downstream impact.**

- `gate-05-content-authoring-base/01-code-architecture.md`: hosting wrapper exposes a `ScriptHost` trait with two backends.
- `gate-06-iteration-workflow/01-code-architecture.md`: script hot reload is documented as desktop-only via CoreCLR; mobile reload of script behavior is content-only (data, hot-update assets).
- `gate-07-mobile-hot-update-contracts/05-feature-requirements.md`: G7-F02 mobile script API subset is the **NativeAOT-compilable** subset of `ScriptAPI-v0`.
- `gate-19-release-pipeline`: iOS player build must publish AOT, with reflection trimming warnings treated as build errors.

**Not decided.** Choice of specific NativeAOT trim profile per platform; that is a Gate 7/19 implementation detail.

## FD-002: Engine threading model

**Decision.** The runtime uses four logical threads, each with a defined ownership boundary:

| Thread | Owns | Forbidden |
|---|---|---|
| Main | ECS world mutation, script callbacks, frame scheduling, gameplay tick | Direct GPU work, blocking IO, audio mixing |
| Render | `RendererInput-v0` -> RHI submission, swapchain present | ECS world mutation, script invocation, blocking asset IO |
| Audio | Mixer callback, decode submission | Asset load synchronously, ECS mutation, allocate via main allocator (must use pre-sized pools) |
| IO pool | Asset loading, file watcher dispatch, package download/install IO | Touching ECS storage, calling renderer, calling scripts |

Cross-thread communication uses lock-free queues or double-buffered snapshots; locks are only allowed during init/teardown.

**Rationale.** Avoids tokio dependency (see FD-008) while still letting the renderer pipeline a frame against ECS extraction, and protects audio from frame stalls.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: workspace defines the thread topology and ownership rules from day one.
- `gate-04-ecs-scene-runtime/01-code-architecture.md`: extraction copies render data into a frame-snapshot buffer that the render thread reads without locks.
- `gate-06-iteration-workflow`: hot-reload swaps happen at frame boundaries on the main thread.
- `gate-16-audio-system`: mixer must use pre-allocated buffers and lock-free command queue.

**Not decided.** IO pool size policy (cores-based vs fixed); deferred to Gate 5 when first asset loads land.

## FD-003: iOS graphics backend

**Decision.** iOS uses **MoltenVK**. `RHI-v0` remains a single Vulkan abstraction; there is no separate `MetalRHI`.

**Rationale.** Keeps `RHI-v0` and Gate 2 backend code unified; MoltenVK is mature enough for Gate 19 target.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: `BackendCapabilities` must enumerate MoltenVK's feature subset (e.g. no geometry shaders, limited descriptor indexing).
- `gate-02-vulkan-renderer/01-code-architecture.md`: iOS surface creation goes through MoltenVK's `VK_MVK_macos_surface`/`VK_EXT_metal_surface`.
- `gate-19-release-pipeline`: iOS packaging includes the MoltenVK library and license attribution.

**Not decided.** Whether to ship MoltenVK as a static lib (smaller IPA) or dynamic; Gate 19 owns this.

## FD-004: Shader toolchain

**Decision.** Shaders are authored in **GLSL** and compiled to SPIR-V via **shaderc (glslang)**. Reflection uses **spirv-reflect**.

**Rationale.** GLSL has wider editor and learning support than HLSL for this codebase; shaderc + spirv-reflect are the standard combo and both have Rust bindings.

**Elaborated by.** `FD-037` (source file layout and stage convention), `FD-038` (include and preprocessor), `FD-039` (backend translation pipeline; `naga` cook-time SPIR-V → GLSL / HLSL and DXC HLSL → DXIL for `backend-opengl` / `backend-dx12`), `FD-040` (variant / permutation model), `FD-041` (descriptor set / bind layout convention), `FD-042` (`CookedShader-v0` cooked artifact + PSO cache). Together these six entries cover every step from `.glsl` source on disk to a runtime `vk::Pipeline`.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: `ShaderFormat` enum's Active vs Reserved disposition is pinned by `FD-039`.
- `gate-01-workspace-rhi/03-best-practices.md`: "Global Shader Strategy" table is frozen by this FD plus `FD-037..042`.
- `gate-02-vulkan-renderer/01-code-architecture.md`: open question "shader toolchain" is now frozen; the Vulkan backend additionally owns the PSO cache per `FD-042` and consumes the four-set `reflected_layout` per `FD-041`.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: `MaterialBinding` references shaders through a `Pipeline` `AssetId` per `FD-042`; extraction resolves the runtime `variant_key` per `FD-040`.
- `gate-05-content-authoring-base`: shader assets are cooked through shaderc; the cook walks `assets/shaders/` per `FD-037`, resolves `#include` per `FD-038`, fans out per backend per `FD-039`, expands variants per `FD-040`, validates bind layout per `FD-041`, and writes `CookedShader-v0` per `FD-042`. Failures produce diagnostics with file/line.
- `gate-06-iteration-workflow/01-code-architecture.md`: hot reload uses the reverse-dependency index from `CookedShader-v0.include_hashes` (per `FD-038`) and swaps pipelines at frame boundary per `FD-042`.
- `gate-19-release-pipeline`: CI must install LunarG SDK or build shaderc/glslang from source; document C++ build dependency. Per-backend cook fan-out (per `FD-039`) determines which platform's CI also needs `hassle-rs` / DXC.

**Not decided.** Whether HLSL is accepted as a secondary source format; explicitly **no** in v0.

## FD-005: Mobile budget reporting timing

**Decision.** Starting **Gate 5**, every gate's `04-performance-report.md` must report **both** desktop baseline numbers **and** mobile simulator numbers for the gate's required fixture. Real mobile device numbers remain mandatory only in Gate 19.

**Rationale.** Catches mobile-only regressions (allocator pressure, shader limits, slow paths under MoltenVK feature subset, NativeAOT cold start) before they reach Gate 19 packaging.

**Downstream impact.**

- `performance-budgets.md`: per-gate budget table gains a mobile-simulator column starting Gate 5; mobile-simulator hardware class becomes mandatory from Gate 5 instead of Gate 7.
- Every `04-performance-report.md` template from Gate 5 onward must dual-report.

**Not decided.** Exact mobile simulator hardware profile; Gate 5 owner picks one and records it in `performance-budgets.md`.

## FD-006: Cooked asset binary format

**Decision.** Cooked assets use **bincode** (serde-based) with a fixed binary header.

Binary header layout:

```text
CookedAssetHeader {
  magic: [u8; 8] = b"ENGCOOK\0",
  header_version: u16,        // header struct version, NOT asset schema version
  asset_kind: u16,            // enum AssetKind (mesh, texture, scene, prefab, audio, navmesh, shader, pipeline, cooked_shader, ...)
  schema_version: { major: u16, minor: u16, patch: u16 },
  content_hash: [u8; 32],     // sha256 of the bincode payload that follows
  uncompressed_size: u64,
  compressed_size: u64,       // == uncompressed_size if compression == none
  compression: u8,            // 0 = none, 1 = zstd
  reserved: [u8; 7]
}
// followed by bincode payload of size `compressed_size`
```

**Byte order.** All multi-byte fields (header + bincode payload) are **little-endian**. This is the bincode default and matches every supported target architecture (`x86_64`, `aarch64`); no runtime byte-swap is needed. Loaders that ever encounter a big-endian target must convert at read time and emit a `Diagnostic`.

**Rationale.** bincode is fast, allocation-friendly, and serde-native; explicit header decouples asset versioning from bincode format changes and supports streaming validation.

**Downstream impact.**

- `data-schema-contracts.md`: add a "Cooked Asset Binary Format" section that pins this header.
- `gate-05-content-authoring-base/01-code-architecture.md`: cook pipeline writes this header; loader validates it before invoking bincode.
- `gate-08-hot-update-package`: payload integrity check verifies header magic and content hash before activation.

**Not decided.** Choice of compression for which asset kind; Gate 5 owns defaults.

## FD-007: Skinned render input shape

**Decision.** There is **no standalone** `SkinnedRenderInput-v0` contract. Skinned drawables are a field of `RendererInput-v0`:

```text
RenderFrameInput {
  ...existing fields...,
  skinned_items: [SkinnedItem]
}
```

This change bumps `RendererInput-v0` to a `minor` version (additive field).

**Rationale.** Skinned items share frame ownership, sort policy, and validation flow with normal drawables; keeping them in one contract avoids two parallel submission paths.

**Downstream impact.**

- `data-schema-contracts.md`: remove the standalone `## SkinnedRenderInput-v0` section; document `SkinnedItem` under `## RendererInput-v0`.
- `compatibility-error-handling.md`: remove the `SkinnedRenderInput-v0` matrix row.
- `gate-09-subsystem-extension-contracts/*`: replace all references to the standalone contract with "RendererInput-v0 skinned items".
- `gate-10-gameplay-subsystems-foundation/*`: animation extraction writes into `RenderFrameInput.skinned_items`.
- Top-level `README.md` contract quick reference: remove `SkinnedRenderInput-v0`.

**Not decided.** Whether morph targets get a separate field or extend `SkinnedItem`; deferred to Gate 11.

## FD-008: IO and async runtime model

**Decision.** The engine is **pure synchronous Rust** plus a self-managed `std::thread` IO pool. No `tokio`, `async-std`, `smol`, or async runtime appears in any workspace `Cargo.toml`.

**Rationale.** Game engines have a frame-scheduled main loop; async runtimes add startup cost, allocator pressure, and an extra mental model. Blocking IO on a dedicated pool is simpler, more deterministic, and easier to profile.

**Downstream impact.**

- Root `Cargo.toml` (or a future `deny.toml`) must forbid async runtime dependencies in deps and dev-deps; CI grep enforces this.
- `gate-06-iteration-workflow`: file watcher (FD-019) integrates via std channels; no `tokio::sync::mpsc`.
- `gate-08-hot-update-package`: HTTP downloader uses a blocking client (recommended: `ureq`); concurrency comes from the IO pool, not async tasks.
- `gate-16-audio-system`: audio mixer schedules its own thread (FD-002) and does not depend on any runtime.

**Resolved by FD-033.** Cross-thread channel choice is now frozen: every production crate uses `crossbeam-channel`. The audio command queue remains a lock-free SPSC (per FD-002) because `crossbeam-channel` allocates on send.

## FD-009: Schema migration mechanism

**Decision.** Migrations between `<Contract>-v0.x` versions use **serde-level defaults**: `#[serde(default)]`, `#[serde(rename = "...")]`, and `Option<T>` for newly added optional fields. Custom `Deserialize` impls are allowed for cases where a default cannot be expressed declaratively. There is **no formal `Migrator` trait or migration framework** in v0.

When a `major` bump becomes necessary, the migration mechanism will be re-frozen at that point (likely a per-contract `Migrator` trait).

**Rationale.** v0 contracts are evolving fast and most changes will be additive; a heavyweight framework is premature. Serde defaults cover the common case without runtime cost.

**Downstream impact.**

- Every `-v0` contract documented in `data-schema-contracts.md` must follow additive-only rules during v0.
- `gate-04-ecs-scene-runtime/03-best-practices.md`: codify "new optional fields require `#[serde(default)]`; renaming a field is a v1 bump".
- An offline `engine-migrate` CLI is **not** part of any v0 gate; it may be introduced when the first v1 ships.

**Not decided.** When to introduce the formal `Migrator` framework; tied to the first `major` bump (probably ECSScene or Prefab).

## FD-010: Cargo feature flag taxonomy

**Decision.** All workspace Cargo features use a **three-segment, kebab-case** naming scheme:

| Prefix | Meaning | Examples |
|---|---|---|
| `backend-*` | Selects a renderer or RHI backend implementation. | `backend-vulkan` |
| `subsystem-*` | Enables a gameplay subsystem implementation. | `subsystem-physics-rapier`, `subsystem-audio-cpal` |
| `tooling-*` | Enables editor, profiler, or dev-only crates. | `tooling-editor`, `tooling-hot-reload` |
| `target-*` | Selects a platform profile (CI matrix selector). | `target-mobile`, `target-desktop` |

Rules:

- No bare names (`vulkan`, `editor`) and no underscores.
- A crate may not silently enable another crate's feature; cross-crate enabling goes through the workspace meta-crate.
- Default features are limited to `backend-vulkan` and the desktop target combination; everything else opt-in.

**Rationale.** Predictable CI matrix expansion; clear ownership when a feature breaks the build.

**Downstream impact.**

- `gate-01-workspace-rhi/03-best-practices.md`: codify naming scheme.
- `gate-11-gameplay-subsystems-expansion`: physics/animation alt backends use `subsystem-*` features.
- `gate-19-release-pipeline`: CI matrix is generated by `target-*` cross `backend-*`.

**Not decided.** Whether `default = ["backend-vulkan"]` lives at the workspace meta-crate or each crate.

## FD-011: Editor vs runtime crate split

**Decision.** The editor lives in a **separate crate** (`engine-editor`) gated by the `tooling-editor` feature. The runtime crates (`engine-core`, `engine-renderer`, `engine-scene`, etc.) **must not** compile any editor code in default/runtime/mobile builds.

**Rationale.** Smallest IPA/APK size, no editor IPC machinery in shipped runtime, no "editor mode" runtime conditionals scattered through gameplay code.

**Downstream impact.**

- `gate-05-content-authoring-base/01-code-architecture.md`: editor scaffolding goes into `engine-editor`, not `engine-core`.
- `gate-17-production-editor-tools`: all production editor surfaces extend `engine-editor`.
- `gate-19-release-pipeline`: release builds explicitly drop `tooling-editor`; CI lints for any leftover `#[cfg(feature = "tooling-editor")]` in runtime crates.

**Not decided.** Whether the editor uses egui, an iced fork, or a custom toolkit; deferred to Gate 5.

## FD-012: Determinism policy

**Decision.** From **Gate 4** onward, the engine bans `std::collections::HashMap` and `HashSet` in any code path that participates in scene state, ECS iteration order, physics step, animation eval, scripting tick, or asset cook ordering. Acceptable alternatives:

- `IndexMap` / `IndexSet` (insertion-ordered).
- `rustc_hash::FxHashMap` / `FxHashSet` plus an explicit sort step where iteration order is observable.
- `BTreeMap` / `BTreeSet` when order semantics are wanted.

CI runs at least one deterministic replay test per gameplay-related gate (Gate 4, Gate 10, Gate 12, Gate 13, Gate 18) that re-runs a fixture twice and byte-compares the produced scene/event log.

**Rationale.** Replay, deterministic networking (future), reproducible profiling, debuggable physics; pervasive `std::HashMap` makes all of these fragile.

**Downstream impact.**

- `gate-01-workspace-rhi/03-best-practices.md`: add the HashMap ban as a workspace-wide lint (clippy custom or grep CI rule).
- `gate-04-ecs-scene-runtime/02-validation-acceptance.md`: deterministic replay test is a Gate 4 exit requirement.
- `gate-10/12/13/18`: each gate adds a determinism integration test to its `07-test-plan.md`.

**Enforcement (frozen at Gate 1).**

- A `deny.toml` at the workspace root pins `cargo-deny` to ban async runtimes (`tokio`, `async-std`, `smol`) and to enforce license allow-list (per FD-025).
- CI runs the following ripgrep check after every workspace build and fails if it returns any hit outside `crates/*/tests/` and `crates/*/benches/`:

  ```bash
  rg --type rust 'std::collections::Hash(Map|Set)' crates/ \
    --glob '!crates/*/tests/**' --glob '!crates/*/benches/**'
  ```

- The same CI step runs an `unsafe`-block grep that respects FD-035 (the unsafe-code policy in `design/README.md`): any `unsafe` keyword outside `render-vulkan`, `engine-audio`, or `engine-script` fails the build.

**Not decided.** Whether to enable Rust's `-Z randomize-layout` in CI; deferred until first determinism bug.

## FD-013: Platform layer scope

**Decision.** Gate 1 ships **winit-only** as the windowing/input layer. A `PlatformAdapter` trait is introduced in **Gate 7** to abstract mobile-specific lifecycle events (suspend/resume, IME, safe-area insets, low-memory warnings, touch event normalization).

Initial `PlatformAdapter` implementations:

| Platform | Implementation |
|---|---|
| Desktop (Windows/macOS/Linux) | winit adapter |
| Android | winit adapter + GameActivity hooks (added in Gate 7) |
| iOS | UIKit bridge (added in Gate 7) |

**Rationale.** Lets Gate 1-6 stay on the simplest stack; defers the mobile-only complexity to where it is actually needed.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: document winit-only scope and the deferred `PlatformAdapter`.
- `gate-07-mobile-hot-update-contracts/01-code-architecture.md`: introduces `PlatformAdapter` trait and its mobile implementations.
- `gate-18-gameplay-framework-platform`: input action mapping consumes `PlatformAdapter` events.

**Not decided.** Whether desktop also gets a touch/pen path; deferred until first product need.

## FD-014: Logging and tracing

**Decision.** Use **`tracing` + `tracing-subscriber`** for all engine-internal logs, spans, and structured events. The `Diagnostic` envelope (see `compatibility-error-handling.md`) is the user/asset/contract error surface; `tracing` is the developer/profiler observability surface. The two are complementary, not redundant.

**Rationale.** `tracing` supports structured fields, spans, and multiple subscribers (console, file, JSON, perfetto); `log` lacks structured fields.

**Downstream impact.**

- All gates: log calls use `tracing::{info, warn, error, debug, trace}` and `#[tracing::instrument]` on hot paths.
- `gate-19-release-pipeline`: release builds attach a JSON file subscriber; editor/dev attaches a console subscriber with colors.
- `gate-06-iteration-workflow`: hot-reload logs go through tracing spans so they can be filtered in the editor.

**Not decided.** Whether to include `tracing-tracy` for the in-engine profiler; deferred to Gate 17.

## FD-015: Vulkan GPU memory allocator

**Decision.** Use the **`gpu-allocator`** crate (Embark) for all Vulkan device memory allocations.

**Rationale.** Pure Rust, no C++ FFI build cost, actively maintained, MIT/Apache-2.0 license (matches FD-025), good defaults for game workloads.

**Downstream impact.**

- `gate-02-vulkan-renderer/01-code-architecture.md`: every `BufferDescriptor`/`TextureDescriptor` allocation goes through `gpu_allocator::vulkan::Allocator`.
- `gate-19-release-pipeline`: Bill-of-materials includes `gpu-allocator` license.

**Not decided.** Per-pool allocation strategy tuning; left to Gate 2 implementation.

## FD-016: Physics backend

**Decision.** First and currently only physics backend is **Rapier 3D**. The `subsystem-physics-rapier` feature gates it; `Physics/Animation-v0` is the contract Rapier must conform to.

**Rationale.** Pure Rust, mobile-friendly, deterministic mode available, integrates cleanly with FD-012.

**Downstream impact.**

- `gate-10-gameplay-subsystems-foundation/01-code-architecture.md`: physics module uses Rapier; backend handles never leak (per `Physics/Animation-v0` rules).
- `gate-11-gameplay-subsystems-expansion`: alt physics backend (e.g. Jolt) only appears if a product need is logged; default stays Rapier.

**Not decided.** Whether Rapier's parallel solver is enabled by default; benchmark in Gate 10.

## FD-017: Audio backend

**Decision.** Audio uses **`cpal`** for device output and **`symphonia`** for asset decode. The `subsystem-audio-cpal` feature gates it.

**Rationale.** Pure Rust, cross-platform (WASAPI, CoreAudio, ALSA, AAudio), no licensing fees, license fits FD-025.

**Downstream impact.**

- `gate-16-audio-system/01-code-architecture.md`: device callback is the cpal stream callback; mixer runs in the cpal thread (FD-002 audio thread).
- `gate-16-audio-system/03-best-practices.md`: codify "no allocations or asset IO inside the cpal callback".

**Not decided.** DSP/effects library; deferred until first product need.

## FD-018: ECS implementation

**Decision.** Implement a **hand-written sparse-set ECS** inside `engine-scene` (the canonical crate per FD-029; earlier drafts of this document called it `engine-ecs`). No external ECS crate (no `hecs`, no `bevy_ecs`, no `flecs`).

**Rationale.** Full control over serialization (must match `ECSScene-v0`), scheduler, determinism (FD-012), and editor introspection. Bevy_ecs would push toward Bevy's schedule and plugin model; hecs lacks the editor introspection layer; flecs is C++ FFI.

**Downstream impact.**

- `gate-04-ecs-scene-runtime/01-code-architecture.md`: documents the sparse-set layout, archetype-free design, system scheduler shape.
- `gate-04-ecs-scene-runtime/03-best-practices.md`: component design patterns are specific to this ECS.
- `gate-09-subsystem-extension-contracts`: `SubsystemExtension-v0` component registration plugs into this ECS, not a third-party one.

**Not decided.** Whether to add SIMD-optimized iteration helpers in v0; deferred to Gate 10/11 profiling.

## FD-019: File watcher

**Decision.** Use the **`notify`** crate for source asset and script file watching (Gate 6 hot reload).

**Rationale.** De-facto standard, cross-platform, license fits FD-025.

**Downstream impact.**

- `gate-06-iteration-workflow/01-code-architecture.md`: file watcher runs on the IO pool (FD-008), dispatches events into a main-thread queue.

**Not decided.** Debounce window default; Gate 6 picks one.

## FD-020: Networking scope

**Decision.** Networking and multiplayer are **out of scope** for Gates 1-19. The engine has no built-in transport, replication, or matchmaking. If networking is added later, it will be a new gate group (Gate 20+) and a new `-v0` contract set.

**Rationale.** Prevents scope creep; networking choices interact with determinism, scripting, and packaging in ways that should not be half-baked.

**Downstream impact.**

- Top-level `README.md`: lists networking under explicit non-goals.
- `gate-18-gameplay-framework-platform/05-feature-requirements.md`: confirms no transport API in v0 framework.

**Not decided.** Whether a future networking gate uses QUIC, raw UDP, or WebRTC.

## FD-021: CI/CD provider

**Decision.** Primary CI/CD is **GitHub Actions**. Self-hosted runners are allowed for mobile device farms but the workflow YAML lives in `.github/workflows/`.

**Rationale.** Largest mobile-runner support, free for OSS-tier usage, ubiquitous developer familiarity.

**Downstream impact.**

- `gate-01-workspace-rhi`: introduces a basic `ci.yml` that builds and tests the workspace.
- `gate-19-release-pipeline`: release workflows, signing, and artifact publishing live as GitHub Actions reusable workflows.

**Not decided.** Caching strategy (sccache vs cargo-build-cache); Gate 19 owns.

## FD-022: Image and texture importer

**Decision.** Asset cook uses:

| Format | Crate |
|---|---|
| PNG, JPG, BMP, TGA, etc. | `image` |
| KTX2 containers | `ktx2` |
| Basis Universal supercompressed textures | `basis-universal` FFI bindings |

**Rationale.** `image` is the standard Rust image crate; KTX2 and Basis cover the mobile GPU texture story.

**Downstream impact.**

- `gate-05-content-authoring-base/01-code-architecture.md`: texture cook fanout chooses the right decoder based on extension.
- `gate-19-release-pipeline`: `basis-universal` adds a C++ build dep similar to FD-004 shaderc; document in CI requirements.

**Not decided.** Whether to also support DDS; deferred until a content request.

## FD-023: 3D model import format

**Decision.** The primary 3D model import format is **glTF 2.0** via the **`gltf`** crate. No FBX, no Collada, no USD in v0.

**Rationale.** glTF is the open, supported-by-every-DCC interchange format; FBX requires Autodesk SDK; USD is large and overkill for v0.

**Downstream impact.**

- `gate-05-content-authoring-base/01-code-architecture.md`: model cook is a glTF importer that emits `engine.renderable` + `engine.animation.*` components.
- Authoring docs (any future) point users at glTF export from their DCC.

**Not decided.** Whether to support glTF KHR extensions selectively (e.g. mesh_quantization, draco) in v0.

## FD-024: Rust edition and MSRV

**Decision.** Workspace uses **Rust Edition 2021**. MSRV is **latest stable minus 2** (e.g. if latest stable is 1.82, MSRV is 1.80). CI tests both MSRV and latest stable.

**Rationale.** Edition 2024 still ramping; Edition 2021 is stable and well-supported. MSRV gives room for users on slightly older toolchains.

**Downstream impact.**

- Every `Cargo.toml`: `edition = "2021"`, `rust-version = "1.80"` (or the current value as of Gate 1 start).
- CI matrix runs both `stable` and the MSRV-pinned toolchain.

**Not decided.** When to move to Edition 2024; revisit at start of any year following its stabilization.

## FD-025: Source license

**Decision.** The engine source is dual-licensed **MIT OR Apache-2.0** (the cargo ecosystem default). Every crate's `Cargo.toml` carries `license = "MIT OR Apache-2.0"`.

**Rationale.** Maximum compatibility with the Rust crate ecosystem; users can pick the license that fits their project.

**Downstream impact.**

- Root `README.md` and per-crate `Cargo.toml` carry the license string.
- `gate-19-release-pipeline`: third-party license collection (cargo-about or similar) emits a `NOTICES.txt` in every shipped artifact.

**Enforcement (frozen at Gate 1).**

- Workspace pins `cargo-about` as the license-collection tool. The release job runs `cargo about generate about.hbs > NOTICES.txt` and fails if the file is empty or missing.
- The same `deny.toml` cited in FD-012 carries a `[licenses]` allow-list of `["MIT", "Apache-2.0", "MIT OR Apache-2.0", "Apache-2.0 WITH LLVM-exception", "Unicode-DFS-2016", "BSD-2-Clause", "BSD-3-Clause", "Zlib", "CC0-1.0"]`. Any other license forces an explicit `[[exceptions]]` entry with a justification comment.
- CI greps every workspace `Cargo.toml` for `license = "MIT OR Apache-2.0"`; missing or different value fails the build.

**Not decided.** Whether assets and shaders also use this license or a separate (e.g. CC0) asset license; deferred to first content release.

## FD-026: Shading model and color pipeline

**Decision.** The only v0 shading model is **PBR Metallic-Roughness** (GGX specular + Lambert diffuse, energy-conserving). The working color space is **linear sRGB**; the HDR offscreen target is `R16G16B16A16_SFLOAT`; tone-mapping is **ACES (Narkowicz fitted)** in v0; exposure is physical (EV100 from camera aperture/shutter/ISO). Light intensity uses **lux** for directional and **lumens** for point/spot; emissive material is in **nits**. Authoritative reference: [lighting-system.md](lighting-system.md).

**Rationale.** A single shading model removes a huge class of "which shader path do we test on which platform" questions. Physical units + linear pipeline + ACES is the only choice that scales from mobile to PBR DCC export (Blender, Substance, Maya) without per-asset hacks.

**Downstream impact.**

- `gate-03-scene-rendering-contract`: HDR target + ACES tone-mapping land here; `MaterialBinding` carries `color_space` per-texture.
- `gate-04-ecs-scene-runtime`: `engine.light.intensity` units are documented per light kind; the editor inspector shows the unit suffix.
- `gate-17-production-editor-tools`: tone-mapping debug view, exposure inspector, and white-balance controls live in the editor.
- `gate-19-release-pipeline`: mobile preset locks tone-mapping to ACES and may bake exposure compensation per-platform.

**Not decided.** Whether to allow per-camera tone-mapping override (filmic vs ACES) for cinematics; deferred to Gate 17.

## FD-027: Shading pipeline (forward / forward+)

**Decision.** The engine ships **forward shading only** in v0. Gate 3 implements minimum forward (1 directional + up to 4 point/spot per draw). Gate 10 or Gate 11 adds **Forward+ / clustered forward** behind the `subsystem-lighting-cluster` feature, with a hard cap of 256 lights per view and 16 lights per cluster. **Deferred shading is explicitly out of scope for v0.**

**Rationale.** Forward+ scales from low-end mobile (where it degrades cleanly back to minimum forward) to PC, and avoids the bandwidth cost of a deferred G-buffer on tile-based mobile GPUs. Mixing forward and deferred at runtime is a constant source of cross-platform bugs; we pick one.

**Downstream impact.**

- `gate-03-scene-rendering-contract`: `RendererInput-v0` carries `lights: [LightItem]` consumed by a forward pass; renderer does not allocate a G-buffer.
- `gate-10-gameplay-subsystems-foundation` / `gate-11-gameplay-subsystems-expansion`: one of them owns the Forward+ cluster build; ownership recorded in the gate's `01-code-architecture.md`.
- `lighting-system.md`: documents the per-step evolution and per-step gate ownership.

**Not decided.** Whether Forward+ lands in Gate 10 or Gate 11; gate owners record the split.

## FD-028: Shadow algorithm

**Decision.** Shadows evolve in lockstep with the shading pipeline:

- **Gate 3:** one 2048×2048 R32_SFLOAT directional shadow map, fixed orthographic projection, 1×1 PCF. Point/spot shadow requests are downgraded to `Off` with a one-time diagnostic.
- **Gate 10/11:** **CSM** (3 or 4 cascades, PSSM split, 3×3 PCF, slope-scaled depth bias) for directional, plus cube-map shadows for point and perspective shadow map for spot (cap: 4 shadow-casting point/spot per view). Behind `subsystem-lighting-csm`.
- Soft shadow techniques (PCSS, contact-hardening, ray-traced) are **out of scope for v0**.

**Rationale.** A single directional shadow map is enough to validate the entire shadow pipeline (depth pass, bias, sampler comparison, pass dependencies). CSM and per-light cube/perspective maps are well-understood algorithms that we know our target hardware can run. Soft-shadow research is not a v0 concern.

**Downstream impact.**

- `gate-03-scene-rendering-contract`: shadow pass is part of the minimum render graph; `RendererInput-v0` documents the downgrade behavior.
- `gate-10/11`: the owning gate enables CSM + cube-map + perspective shadow paths.
- `lighting-system.md`: per-step ownership and feature-flag mapping.

**Not decided.** CSM cascade count and split lambda — tracked under `OFQ-007`.

## FD-029: Workspace crate layout

**Decision.** The workspace ships **exactly 20 crates**, named by the table below. Engine-layer crates use the `engine-*` prefix; RHI-stack crates use the `render-*` prefix; `platform` and `sandbox` are intentionally unprefixed because they are the OS boundary and the validation executable. All crates live under `crates/<crate-name>/`.

| Crate | Role | Introduced |
|---|---|---|
| `platform` | Window, input, raw window handle, `PlatformAdapter`. | Gate 1 (winit-only); Gate 7 adds `PlatformAdapter`. |
| `render-core` | RHI traits, descriptors, opaque handles. No backend code. Owns `RHI-v0`. | Gate 1 |
| `render-vulkan` | Vulkan backend implementing `render-core` traits. | Gate 1 (compile stub); Gate 2 (real impl). |
| `render-opengl` | OpenGL backend stub. | Gate 1 (compile stub). |
| `render-dx12` | DirectX 12 backend stub. | Gate 1 (compile stub). |
| `engine-core` | Engine facade: lifecycle, world bootstrap, root scheduler. May re-export `render-core` types (see Gate 1 Q3). | Gate 1 |
| `engine-serialize` | Deterministic serialization helpers (RON / JSON / bincode), schema-version helpers. | Gate 1 (placeholder); Gate 4 (real impl). |
| `engine-renderer` | High-level renderer that consumes `RendererInput-v0`, owns the render graph, lighting passes, tone-mapping. Depends on `render-core` only; never on a specific backend. | Gate 1 (placeholder); Gate 3 (real impl). |
| `engine-scene` | Hand-written sparse-set ECS, scene/prefab persistence, extraction pipeline. (Replaces all earlier mentions of `engine-ecs`; per FD-018.) | Gate 1 (placeholder); Gate 4 (real impl). |
| `engine-asset` | Asset registry, importers, cookers, hot-reload runtime. (Replaces earlier unprefixed `asset`.) | Gate 1 (placeholder); Gate 5 (real impl); Gate 6 (watcher). |
| `engine-script` | .NET hosting wrapper (CoreCLR + NativeAOT, per FD-001); owns `ScriptAPI-v0`. | Gate 1 (placeholder); Gate 5 (real impl). |
| `engine-editor` | Editor scaffolding, behind `tooling-editor` feature (FD-011). | Gate 1 (placeholder); Gate 5 (real impl); Gate 17 (production). |
| `engine-hot-update` | Package download, verify, install, rollback. (Replaces earlier unprefixed `hot-update`.) | Gate 1 (placeholder); Gate 7/8 (real impl). |
| `engine-physics` | Engine-facing physics API, Rapier backend (FD-016). | Gate 1 (placeholder); Gate 10 (real impl); Gate 11 (expansion). |
| `engine-animation` | Skeleton, AnimationPlayer, evaluator. | Gate 1 (placeholder); Gate 10 (real impl); Gate 11 (expansion). |
| `engine-audio` | cpal-backed mixer (FD-017), spatialization. | Gate 1 (placeholder); Gate 16 (real impl). |
| `engine-ui` | UI runtime, `UiBatch` extraction. | Gate 1 (placeholder); Gate 15 (real impl). |
| `engine-nav` | Navmesh runtime, pathfinding (per OFQ-001 library choice). | Gate 1 (placeholder); Gate 13 (real impl). |
| `engine-character` | Character controller (movement, locomotion). | Gate 1 (placeholder); Gate 12 (real impl). |
| `sandbox` | Validation executable; per-gate fixtures live as subcommands or features. | Gate 1 |

**Two-layer renderer split.**

- `render-core` exposes the **`RHI-v0`** contract: traits, descriptors, handles, capabilities, errors. No backend code, no shaders, no render graph.
- `engine-renderer` exposes the **`RendererInput-v0`** consumer: render graph, passes (shadow / opaque PBR / tone-map / present), lighting (forward in Gate 3, later Forward+ / CSM / IBL behind `subsystem-lighting-*` features), debug-draw consumption.
- Gameplay-side crates (`engine-scene`, `engine-ui`, `engine-physics`, `engine-script`, etc.) depend on `engine-renderer` for high-level draw entry, **not** on `render-core` or `render-*` backend crates.
- A future production renderer crate (`engine-renderer-pro`) may be added behind a feature; the public contract surface stays in `render-core` + `engine-renderer`.

**Naming rule.**

- Engine-layer crates: `engine-` prefix.
- RHI-stack crates: `render-` prefix.
- `platform` and `sandbox` are intentionally unprefixed.
- Backend crates added later for a single subsystem (e.g. a Jolt physics backend) live in a sibling crate named after the backend (`physics-jolt`), not inside `engine-physics`.

**Gate 1 obligation.** Gate 1 creates the full workspace shell with **all 20 crates** declared in the root `Cargo.toml`, even if most are empty `pub fn placeholder() {}` stubs. Later gates may not add or rename workspace members ad hoc; renames go through the contract change workflow at the end of `compatibility-error-handling.md`.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: replaces the legacy placeholder list (`engine-ecs`, `asset`, `hot-update`) with the FD-029 list and references this FD.
- `gate-01-workspace-rhi/05-feature-requirements.md` / `07-test-plan.md` / `README.md`: feature requirements and gate-exit grep checklists list all 20 crates by canonical name.
- All later gates that referenced `engine-ecs` use `engine-scene`; references to `asset` use `engine-asset`; references to `hot-update` use `engine-hot-update`.
- `lighting-system.md`'s `engine-renderer::lighting::units` is now resolved to a real crate.

**Not decided.** Whether to split `engine-renderer` further into `engine-renderer-graph` + `engine-renderer-pipelines` once the lighting expansion lands; that is a Gate 10/11 refactor question, not a v0 contract.

## FD-030: Math library

**Decision.** The engine uses **`glam`** as the single linear-algebra library. Public-facing math types are `glam::{Vec2, Vec3, Vec3A, Vec4, Quat, Mat3, Mat4, Affine3A}`. No `nalgebra`, `cgmath`, `euclid`, or hand-rolled `[f32; N]` matrices in public APIs.

When a third-party crate insists on a different math library (notably **Rapier** uses `nalgebra` internally), the engine never leaks the foreign types: `engine-physics` exposes only `glam` types on its public surface and converts at the boundary via `Vec3::from_array` / `Mat4::from_cols_array` helpers (no `mint` dependency).

**Rationale.** `glam` is SIMD-friendly (`#[repr(simd)]` `Vec3A`), small, supports `no_std`, integrates with `bytemuck` for GPU uploads, and is the de-facto Rust game-engine math library (Bevy, Macroquad, Fyrox). It uses **column-major** storage for matrices — the same convention as GLSL / SPIR-V / HLSL — which avoids a transpose at upload time.

**Downstream impact.**

- Workspace root `Cargo.toml` pins a single `glam` version that every crate inherits via `glam.workspace = true`.
- All schema types `Vec2` / `Vec3` / `Vec4` / `Quat` / `Mat3` / `Mat4` in [data-schema-contracts.md](data-schema-contracts.md) resolve to `glam` types; serialization is little-endian `f32` arrays.
- `engine-physics` converts to/from `nalgebra` at the Rapier seam; no `nalgebra` types appear in any other crate.
- Shaders consume matrices as column-major (no transpose at upload).

**Not decided.** Whether to enable `glam`'s `serde` feature globally or hand-roll bincode serialization with `[f32; N]` arrays for stability; deferred to Gate 4 when scene serialization lands.

## FD-031: Coordinate system, units, NDC

**Decision.** Engine-wide spatial and unit conventions are frozen as:

| Aspect | Convention |
|---|---|
| World handedness | **Right-handed**. |
| Up axis | **+Y**. |
| Forward axis (camera) | **-Z** (camera looks down -Z; +Z is behind the camera). |
| Texture UV origin | **Top-left** (`(0, 0) = upper-left`, `(1, 1) = lower-right`). |
| Matrix order | **Column-major** (matches `glam`, GLSL, SPIR-V). |
| Vector / matrix multiplication | `matrix * vector` — matrix on the **left**, vector on the right; matrices represent world-from-local. |
| NDC X / Y range | `[-1, 1]`. |
| NDC depth range | **`[0, 1]` (Vulkan-style, reverse-Z friendly)**. OpenGL / DX12 backends convert at projection-matrix construction, not in the shader. |
| Y-flip for Vulkan | Renderer applies a viewport Y-flip (`viewport.height < 0`) so the same projection matrix works on every backend. |
| Linear distance | **1.0 = 1 meter**. Authored content (glTF, FBX) is imported assuming meters; non-meter content is scaled at import. |
| Time | seconds (`f32` for per-frame delta, `f64` for absolute timeline). |
| Angle | **radians**, everywhere in code and serialization. Editor UI may show degrees but converts on edit. |

**Rationale.** Matches glTF 2.0 (FD-023) exactly, which removes the import conversion. Matches `glam`'s defaults. Vulkan `[0, 1]` depth gives the best precision distribution for reverse-Z and is trivially adapted for GL / DX12 backends. A single agreed convention removes an entire class of "is this in the right space" bugs.

**Downstream impact.**

- `data-schema-contracts.md`: pins all `Vec3` / `Quat` / `Mat4` fields to this convention via a "Math And Coordinate Conventions" subsection.
- `gate-02-vulkan-renderer/01-code-architecture.md`: projection matrix construction goes through `engine-renderer::math::perspective_rh_z01()` (reverse-Z) and uses a Vulkan viewport Y-flip.
- `gate-03-scene-rendering-contract`: shadow-map projection uses reverse-Z; shaders sample with `>=` depth test.
- `gate-04-ecs-scene-runtime`: `engine.transform` is in this space; +Y up, -Z forward documented in the architecture.
- `gate-05-content-authoring-base`: glTF importer asserts meters; warns and uniformly scales otherwise.
- `gate-10-gameplay-subsystems-foundation`: Rapier physics world is configured with gravity = `(0.0, -9.81, 0.0)`.

**Not decided.** Whether to expose a "Z-up import" mode for legacy Blender content; deferred to Gate 5 when import settings UI lands (currently the importer always rotates Z-up source content to +Y up).

## FD-032: Error handling crate split

**Decision.** Two libraries used in two distinct contexts:

| Library | Used in | Pattern |
|---|---|---|
| `thiserror` | All **library** crates (everything except `sandbox`) for typed, named error enums (`RhiError`, `CookError`, `ScriptError`, `PackageError`, etc.). | `#[derive(thiserror::Error, Debug)]` with one variant per failure mode; each variant carries `#[source]` cause **or** a `code()` accessor for the `Diagnostic.code` mapping. |
| `anyhow` | `sandbox` and binaries only (CLI / release tooling). | `anyhow::Result<T>` for top-level program-level error reporting; never in public library APIs. |

`thiserror` error enums are the source of truth for the `Diagnostic.code` mapping in [compatibility-error-handling.md](compatibility-error-handling.md): each crate keeps a one-to-one mapping table in its `errors.rs`.

**Banned.** `Box<dyn Error>` in public signatures, `eyre`, `color-eyre`, ad-hoc `String` errors. `panic!` is reserved for programmer-invariant violations only (per the existing rule in `compatibility-error-handling.md`).

**Rationale.** `thiserror` keeps library errors strongly typed without runtime overhead; `anyhow` is fine in binaries where the goal is "print and exit". The split keeps the `Diagnostic.code` mapping mechanical.

**Downstream impact.**

- Every library crate's `Cargo.toml` carries `thiserror = { workspace = true }`.
- `sandbox`'s `Cargo.toml` carries `anyhow = { workspace = true }`; no library crate does.
- `engine-core` does **not** introduce a single wrapper error that re-enums every other crate's error; each crate's enum stands alone and maps to diagnostics individually.
- `compatibility-error-handling.md`: each contract row references the owning crate's error enum by name.

**Not decided.** Specific MSRV-compatible version pin for `thiserror` (currently 1.x); workspace root `Cargo.toml` picks one at Gate 1.

## FD-033: Cross-thread channel crate

**Decision.** The engine uses **`crossbeam-channel`** as the only cross-thread channel implementation in production code. `std::sync::mpsc` is **banned** in production code (allowed only in throwaway examples and one-off tests).

Allowed channel constructors:

- `crossbeam_channel::unbounded()` — events with naturally bounded burst sizes (window events, diagnostics queue).
- `crossbeam_channel::bounded(capacity)` — back-pressured queues (renderer extraction snapshot, IO pool job queue, file watcher dispatch buffer). Bounded channels must document their capacity and the behavior when full (block / drop oldest / drop newest).
- `crossbeam_channel::select!` — threads that wait on more than one channel.

The audio command queue is a lock-free SPSC (per FD-002), **not** a `crossbeam-channel`, because `crossbeam-channel` allocates on send and the audio thread must not allocate.

**Rationale.** `crossbeam-channel` is faster than `std::sync::mpsc`, supports `select!`, supports bounded channels with sane back-pressure, and is already a transitive dependency of common workspace crates. The earlier "first crate to need it picks" policy (FD-008) is replaced by this FD.

**Downstream impact.**

- `gate-04-ecs-scene-runtime`: extraction snapshot uses a `bounded(2)` `crossbeam-channel` (capacity = 2 for double-buffer) between main and render threads.
- `gate-06-iteration-workflow`: file watcher dispatches into an unbounded `crossbeam-channel`; consumer drains at frame boundary.
- `gate-08-hot-update-package`: IO pool job queue is a bounded `crossbeam-channel`; capacity sized at `max_inflight_downloads`.
- `gate-16-audio-system`: audio command queue remains the lock-free SPSC; the `crossbeam-channel` ban for the audio thread is explicit.
- FD-008's "Not decided" clause is removed.

**Not decided.** Whether to use `crossbeam-channel::after()` for timed wake-ups in the IO pool or `std::thread::park_timeout`; deferred to Gate 5 / 8 IO pool implementation.

## FD-034: Camera component minimum field set

**Decision.** The `engine.camera` ECS component (frozen at Gate 4, consumed by Gate 3 extraction into `RenderView`) carries **exactly** the following fields in v0. No gate may silently extend the on-disk shape; additions require a contract change per the workflow in [compatibility-error-handling.md](compatibility-error-handling.md).

| Field | Type | Notes |
|---|---|---|
| `projection` | enum `Perspective \| Orthographic` | matrix construction goes through `engine-renderer::math::perspective_rh_z01()` / `ortho_rh_z01()` per `FD-031`. |
| `near` | `f32` | meters; `near > 0.0`. |
| `far` | `f32` | meters; `far > near`. |
| `fov_y_or_size` | `f32` | radians (vertical FOV) for `Perspective`; orthographic half-height in meters for `Orthographic`. |
| `viewport_rect` | `Rect` (`{ min: Vec2, max: Vec2 }`, normalized `[0, 1]` of the render target) | default `{ min: (0, 0), max: (1, 1) }`. Used for split-screen, picture-in-picture, and overlay inset; values clamped to `[0, 1]` with `min < max`. |
| `render_layer_mask` | `u32` (bitmask of `RenderLayerId`) | default `0xFFFF_FFFF` (renders every layer). A `RenderableItem` is drawn into this camera only if `(item.render_layer_bit & camera.render_layer_mask) != 0`. The mapping from the human-readable `render_layer: String` (on `engine.renderable`) to a bit index is owned by Gate 4 and is one of the registered enums in [data-schema-contracts.md](data-schema-contracts.md). |
| `clear_flags` | enum `ClearFlags = ColorAndDepth \| DepthOnly \| Nothing \| Skybox` | default `ColorAndDepth`. `Skybox` requires `SceneSettings.environment_map` (Gate 10/11). |
| `clear_color` | `LinearRgb` | applied only when `clear_flags == ColorAndDepth`. |
| `priority` | `i32` | base camera draw order (lower draws first). `FD-035` defines stack composition between Base and Overlay cameras. |
| `render_target` | `Option<AssetId>` | `None` in v0 — renders to the swapchain. Non-`None` is reserved by `OFQ-011` (Gate 17 owns the `RenderTarget` asset type) and producers must not submit it. |
| `msaa_samples` | `u8` (`1`, `2`, `4`, or `8`) | default `1`. Backends downgrade unsupported sample counts with a one-time diagnostic; the frame is not aborted. |
| `hdr_output` | `bool` | default `false`. `true` requests an HDR offscreen target; only consumed when the platform exposes an HDR swapchain (deferred capability — currently always tone-mapped to LDR per `FD-026`). |
| `exposure` | `{ aperture: f32, shutter_speed: f32, iso: f32, ev_compensation: f32 }` | physical exposure per `FD-026`. |

**Rationale.** The user-question audit during Gate 1 review surfaced that the original `engine.camera` had only `projection` / `near` / `far` / `fov` / `priority` / `clear_color` / `exposure`. That set could not express split-screen viewports, per-camera layer culling, overlay cameras that preserve the base camera's color/depth, or any future RTT path. Pinning the full v0 set now (even when several fields default to a no-op) prevents the `RenderView.render_layers` orphan field problem (Gate 3 contract had a layer field that no Gate 4 component produced) and removes the AI-implementation cliff at Gate 4 freeze.

**Banned.** Fields not in the table above. In particular: `cull_mode` overrides, `oblique_clip_plane`, per-camera `tone_mapping` override (deferred to `OFQ-011`), and any virtual-camera / follow / look-at / shake fields (those belong to `OFQ-012` and are **never** part of `engine.camera`).

**Downstream impact.**

- [data-schema-contracts.md](data-schema-contracts.md) `ECSScene-v0` core-component table: `engine.camera` row matches the table above; `Rect`, `ClearFlags`, and `RenderLayerId` are added to Common Field Types.
- [data-schema-contracts.md](data-schema-contracts.md) `RendererInput-v0` `RenderView`: gains the corresponding fields populated by Gate 4 extraction (see `FD-035`).
- `gate-03-scene-rendering-contract/01-code-architecture.md`: Renderer Input Model documents that `viewport_rect`, `clear_flags`, `render_layer_mask`, and `msaa_samples` are read from `RenderView` and applied by the render graph.
- `gate-04-ecs-scene-runtime/01-code-architecture.md`: Core Components `Camera` entry matches this table; extraction validates `near > 0`, `far > near`, `viewport_rect` bounds, and `msaa_samples ∈ {1, 2, 4, 8}`.

**Not decided.** Whether `render_layer_mask` widens to `u64` post-v0; tracked under `OFQ-011` if a content team requests more than 32 render layers.

## FD-035: Camera stack / multi-view composition

**Decision.** `RendererInput-v0.views` is a typed multi-view list with explicit composition rules. Each `RenderView` carries:

| Field | Type | Notes |
|---|---|---|
| `compose` | enum `Base { clear: ClearFlags, clear_color: LinearRgb } \| Overlay { base_view_id: u32, blend_mode: BlendMode }` | `BlendMode = Replace \| AlphaBlend \| Additive`. Default produced by Gate 4 extraction is `Base { clear: ColorAndDepth, clear_color }` (copied from `engine.camera`). |
| `stack_order` | `i32` | tie-breaker among views with the same `compose` kind; lower values draw first. |

**Composition order is deterministic and frozen as:**

1. All `Base` views are drawn first, sorted by `(engine.camera.priority, RenderView.stack_order, view_id)` ascending; each `Base` view clears its render target according to its `ClearFlags`.
2. All `Overlay` views are drawn after their referenced `base_view_id` finishes, sorted by `(stack_order, view_id)` ascending; overlays are composited on top of their base view's color attachment with the declared `blend_mode` and **never clear** the color or depth buffer.
3. An `Overlay` whose `base_view_id` does not exist in the same frame is dropped with a `RV0007 OverlayBaseMissing` diagnostic; the frame is not aborted.
4. Final swapchain composition reads the last-rendered base+overlay group in `priority` order; no additional implicit copies.

**Rationale.** v0 already shipped `RenderFrameInput.views: [RenderView]` as a structural multi-view list but defined no composition semantics, so AI implementers had no way to build split-screen, picture-in-picture, UI overlay, minimap, or rear-view-mirror features without inventing private conventions. Pinning the `Base` / `Overlay` distinction, the deterministic order, and the missing-base diagnostic resolves the contract gap without introducing a virtual-camera / Cinemachine-style director (those remain in `OFQ-012`).

**Downstream impact.**

- [data-schema-contracts.md](data-schema-contracts.md) `RendererInput-v0` `RenderView` required fields: gains `compose` and `stack_order`. New diagnostic code `RV0007 OverlayBaseMissing` listed in [compatibility-error-handling.md](compatibility-error-handling.md) error code tables.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: render graph documents the per-view sub-graph and the overlay composite pass; Open Question on whether renderer input owns culling is **also** resolved (see `FD-036`).
- `gate-04-ecs-scene-runtime/01-code-architecture.md`: extraction lists every enabled `engine.camera` (not only `SceneSettings.active_camera`); `active_camera` becomes the default `Base` view when the scene has no other base cameras, and overlay cameras are linked to a base by an optional `base_camera: Option<PersistentId>` field on `engine.camera` (added under `FD-034` future addition workflow only — **not in v0**; in v0 there is exactly one base camera = `active_camera` and the rest are silently ignored unless their `priority > active_camera.priority`, in which case they are treated as base candidates and a `RV0008 MultipleBaseCameras` warning fires).
- `gate-15-runtime-ui/01-code-architecture.md`: UI canvas in `space: overlay` mode submits an `Overlay { blend_mode: AlphaBlend }` view that references the highest-priority `Base` view.

**Banned.** Implicit camera ordering by entity-creation order, GPU-side composition via shader patches, and any "clear depth without clearing color" trick outside the `clear_flags` enum.

**Not decided.** Whether to allow `BlendMode::Custom { src: BlendFactor, dst: BlendFactor, op: BlendOp }` for advanced compositing; deferred until first product need (likely Gate 17 with post-process volumes via `OFQ-011`).

## FD-036: Frustum culling ownership

**Decision.** Frustum culling is **producer-owned** in v0. Gate 4 (extraction) emits already-culled `RenderableItem` / `LightItem` / `SkinnedItem` lists per `RenderView`, computed against the view's `view_matrix * projection_matrix` and the item's `bounds: AxisAlignedBox` (axis-aligned bounds vs frustum using the conservative SAT test — six plane half-space checks plus the eight-corner inclusion test). The renderer does **not** re-cull and does **not** silently drop visible items.

`RenderView` gains an optional `frustum: Option<[Vec4; 6]>` field (the six plane equations in world space, `(nx, ny, nz, d)` outward-pointing); when present, the renderer **may** run a debug-build consistency check that no submitted item lies fully outside any plane and emits a `RV0009 CulledItemSubmitted` diagnostic if it does. The check is disabled in release builds.

`RendererInput-v0.stats` (frame-stats payload) gains `visible_drawables` / `culled_drawables` counters populated by Gate 4 extraction and surfaced through `tracing` spans (`frame.cull`).

**Rationale.** Resolves Gate 3 Open Question 1 ("Whether renderer input owns culling or consumes already-culled instances"). Producer-owned culling matches the existing extraction model (`FD-002`: extraction runs on the main thread; renderer runs on a separate thread with no ECS access), keeps the renderer backend-agnostic, and removes the temptation for backends to do their own scene-graph queries. The optional `frustum` field makes the cull contract auditable without making it expensive.

**Algorithm pin.** Items use their world-space `AxisAlignedBox` (Gate 4 transforms local `engine.bounds` by `engine.transform.world`). For skinned meshes, Gate 10 expands bounds by the skeleton's pre-computed maximum extent (per `FD-007`). Bounding-sphere or OBB tests are explicitly **not** v0; an item whose AABB is overly conservative is correct but possibly drawn unnecessarily — that is acceptable for v0.

**Downstream impact.**

- [data-schema-contracts.md](data-schema-contracts.md) `RendererInput-v0` `RenderView`: gains `frustum: Option<[Vec4; 6]>`. Adds the `stats: FrameStats { visible_drawables, culled_drawables, visible_lights, culled_lights, draw_calls, ... }` block to the frame-input outputs.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: Open Question 1 is removed; Renderer Input Model documents "`renderables` and `lights` arrive already culled per view".
- `gate-04-ecs-scene-runtime/01-code-architecture.md`: extraction pseudocode documents the per-view AABB-vs-frustum loop; the test plan asserts `visible + culled == total_renderables`.
- `gate-07-test-plan` (every gate from Gate 3 onward): camera-move tests assert `visible_drawables` and `culled_drawables` change as expected.

**Banned.** Renderer-side scene queries, GPU occlusion queries (deferred to a future OFQ), and hierarchical / BVH / octree culling structures in v0 (the linear AABB-vs-frustum loop is cheap enough for the v0 scene sizes — Gate 3 budget is < 5 ms `frame.cull`).

**Not decided.** Whether to add `subsystem-occlusion` feature flag and a future Forward+ cluster build cull (already tracked under `OFQ-009`).

## FD-037: Shader source file layout and stage convention

**Decision.** Authored shader source lives at a fixed workspace location with a fixed naming convention. Cook discovery is mechanical — the cooker does not need a per-file manifest.

| Aspect | Convention |
|---|---|
| Source root | `assets/shaders/`. The crate that owns runtime shaders (`engine-renderer` for engine built-ins) re-exports its shaders into `assets/shaders/engine/` via `build.rs` or a symlink; user-content shaders live under `assets/shaders/user/`. |
| Common-code root | `assets/shaders/common/` (one level under source root). Files here may be `#include`d but are never compiled directly. |
| Stage extension | `.vert.glsl` (vertex), `.frag.glsl` (fragment), `.comp.glsl` (compute, future per `OFQ-013`). One stage per file. Cooker rejects mixed-stage files. |
| Entry point | Always `main`. Anything else is rejected by the cooker with `SH0001 NonMainEntryPoint`. The `ShaderModuleDescriptor.entry_points` field carries `"main"` for the corresponding stage. |
| File encoding | UTF-8 without BOM; LF line endings normalized at cook time (cook hash is computed over normalized bytes so cross-OS check-ins produce identical hashes). |
| Naming | `<material_or_pipeline_name>.<stage>.glsl` (e.g. `pbr_opaque.vert.glsl` + `pbr_opaque.frag.glsl`). A `Pipeline` asset references this name without extension; the cooker globs the matching `*.glsl` files. |
| `#version` directive | First non-comment line must be `#version 450 core` (Vulkan SPIR-V target). Cooker rejects other versions with `SH0002 UnsupportedShaderVersion`. |
| Allowed stages in v0 | **Vertex + Fragment only**. Geometry / tessellation are **banned** (matches MoltenVK feature subset per `FD-003`). Compute is reserved for `OFQ-013`. |

**Rationale.** A fixed convention removes per-file metadata and lets the cooker walk the source tree deterministically. The naming pairs stages automatically (`*.vert.glsl` + `*.frag.glsl`) so a `Pipeline-v0` asset just declares the base name. Entry-point pinning to `main` removes a class of cross-backend mismatches (DXIL / MSL renaming).

**Downstream impact.**

- `gate-01-workspace-rhi/03-best-practices.md`: the "Global Shader Strategy" table's authoring language / stages row is satisfied by this FD.
- `gate-02-vulkan-renderer/01-code-architecture.md`: the Vulkan backend creates `vk::ShaderModule` from SPIR-V produced by the cook step; entry-point is always `"main"`.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: the Material Descriptor Model references shaders by `Pipeline` `AssetId`; the pipeline asset carries the base shader name plus variant keys (see `FD-040`).
- `gate-05-content-authoring-base/01-code-architecture.md`: shader cook walks `assets/shaders/**/*.{vert,frag}.glsl`, compiles each via `shaderc`, and writes `CookedShader-v0` artifacts (per `FD-042`).

**Banned.** Single-file multi-stage shaders, `main_vs`/`main_ps`-style HLSL entry-point names, shader source outside `assets/shaders/`, and shader files committed without LF normalization.

**Not decided.** Whether to support `.glsl.in` template files for engine-internal codegen of common headers; deferred until first need.

## FD-038: Shader include and preprocessor

**Decision.** Shader `#include` is **enabled** via the `GL_GOOGLE_include_directive` extension that `shaderc` supports. The cooker is the single resolver:

| Aspect | Rule |
|---|---|
| Syntax | `#include "path/to/file.glsl"` (quoted form) or `#include <engine/path.glsl>` (angle form). Both pass through the same resolver. |
| Search path | Quoted form: relative to the including file first, then `assets/shaders/common/`. Angle form: only `assets/shaders/common/`. |
| Cycle detection | Cooker maintains a per-compile include-stack; a re-entry produces `SH0003 IncludeCycle` with the full cycle path and aborts that pipeline (other pipelines continue). |
| Maximum depth | 16. Beyond that produces `SH0004 IncludeDepthExceeded`. |
| Dependency recording | Every successful compile records the **full list of resolved include paths plus their content hashes** into `CookedShader-v0.include_hashes` (see `FD-042`). This is what Gate 6 hot-reload watches. |
| Macro definition order | The cooker injects (1) variant `#define`s (per `FD-040`), (2) bind-layout constants (per `FD-041`), (3) engine-wide defines (`ENGINE_REVERSE_Z`, `ENGINE_VULKAN_NDC`, `ENGINE_MAX_LIGHTS_PER_DRAW=5`), then (4) the user source. The order is frozen so hash inputs are deterministic. |
| Banned preprocessor features | `#extension` directives other than `GL_GOOGLE_include_directive` and SPIR-V extensions the backend explicitly enables; `__FILE__` / `__LINE__` macros (replaced by `shaderc` source markers in diagnostics). |

When a header file under `assets/shaders/common/` changes, Gate 6 hot-reload re-cooks every `CookedShader-v0` artifact whose `include_hashes` references the changed file. The dependency direction is **header -> consumer**, computed by inverting `include_hashes` at cook time.

**Rationale.** Without `#include`, the engine would either duplicate `pbr.glsl` / `shadow.glsl` / `skinning.glsl` across every material or invent a private template language. The `shaderc` extension is the standard solution. Recording include hashes (not paths only) makes the hot-reload dependency graph exact — a touched file with unchanged content does **not** trigger rebuilds.

**Downstream impact.**

- `gate-05-content-authoring-base/01-code-architecture.md`: shader cook step builds and persists the include dependency graph.
- `gate-06-iteration-workflow/01-code-architecture.md`: file watcher routes `assets/shaders/common/**` change events through the reverse-dependency index and re-cooks the affected pipelines.
- `compatibility-error-handling.md`: shader diagnostics carry `source: "<file>:<line>"` per `shaderc`'s source-marker output; include-cycle diagnostics list the full path.

**Not decided.** Whether to support `#pragma once` (currently the recursion check makes it unnecessary); deferred unless an authoring pattern demands it.

## FD-039: Backend shader translation pipeline

**Decision.** SPIR-V (Vulkan 1.2 environment) is the **authoritative runtime IR**. Every backend either consumes SPIR-V directly or runs a cook-time translator. Translation is **never** at load time.

| Backend | Consumes | Translator | When |
|---|---|---|---|
| `render-vulkan` (desktop) | SPIR-V | none | direct |
| `render-vulkan` via MoltenVK (iOS) | SPIR-V | MoltenVK runtime (SPIR-V → MSL inside MoltenVK) | runtime, owned by MoltenVK |
| `render-opengl` | GLSL 450 core | **`naga`** (SPIR-V → GLSL) | cook time |
| `render-dx12` | DXIL | **`naga`** (SPIR-V → HLSL) + **DXC** (HLSL → DXIL) via `hassle-rs` FFI | cook time |

**`ShaderFormat` enum disposition** (clarifies `RHI-v0`):

| Variant | Status in v0 |
|---|---|
| `SpirV` | **Active.** Always emitted by cook; required by `render-vulkan`. |
| `Glsl` | **Active.** Emitted by cook only when `backend-opengl` feature is enabled; consumed by `render-opengl`. |
| `Hlsl` | **Reserved.** Cook never emits raw HLSL; the DX12 backend's HLSL is a `naga` intermediate, not surfaced through `ShaderFormat`. Producers must not submit. |
| `Dxil` | **Active.** Emitted by cook only when `backend-dx12` feature is enabled; consumed by `render-dx12`. |
| `Wgsl` | **Reserved (placeholder).** Not produced by cook in v0. Carrying it through `ShaderFormat` is for future WebGPU; producers must not submit. |
| `MslSource` | **Reserved.** MoltenVK does its own SPIR-V → MSL at runtime; the engine never produces MSL source. Producers must not submit. |

**Per-platform cook fanout.** The cooker emits one `CookedShader-v0` artifact per `(pipeline_name, variant_key, platform)` triple. The `CookedShader-v0` payload carries the SPIR-V blob unconditionally plus optional GLSL / DXIL blobs when the corresponding backend feature is enabled. The platform set is derived from `PlatformProfile` per the build's enabled `backend-*` features.

**Rationale.** `naga` is the de-facto Rust shader translator (Bevy, wgpu use it); it has a SPIR-V frontend and GLSL / HLSL backends. Using `naga` avoids the SPIRV-Cross C++ dependency. DXC is the only practical DXIL compiler; `hassle-rs` is the established Rust binding. MoltenVK already owns SPIR-V → MSL inside the iOS Vulkan runtime, so the engine does not translate to MSL itself.

**Banned.** Runtime shader compilation in v0 release builds (only editor/dev builds may invoke `shaderc` at runtime, behind the `tooling-shader-runtime` feature); the SPIRV-Cross C++ library; per-backend hand-authored shader copies.

**Downstream impact.**

- `gate-01-workspace-rhi/01-code-architecture.md`: `ShaderFormat` doc-comments reflect Active vs Reserved.
- `gate-02-vulkan-renderer/01-code-architecture.md`: backend consumes SPIR-V from `CookedShader-v0`; MoltenVK note added to iOS section.
- `gate-05-content-authoring-base/01-code-architecture.md`: shader cook fanout enumerates `naga` translations conditional on enabled `backend-*` features; CI fails the cook if `naga` reports an unimplemented translation for a shader the user wrote.
- `gate-19-release-pipeline`: package step bundles only the artifacts for the platforms targeted by the release.

**Not decided.** Whether to add a `tooling-shader-cross-validate` feature that diffs the Vulkan vs OpenGL render outputs of the same shader; deferred to Gate 17.

## FD-040: Shader variant / permutation model

**Decision.** Material assets declare a **static variant key set**; the cooker emits one `CookedShader-v0` per reachable combination. There is **no runtime shader compilation in release builds**.

| Aspect | Rule |
|---|---|
| Key declaration | A `Material` asset's manifest lists `variant_keys: [VariantKey]` where `VariantKey = { name: String, kind: Bool \| Enum { values: [String] } }`. Each `Bool` key contributes 1 bit; each `Enum { values: [v1..vN] }` key contributes `ceil(log2(N))` bits. |
| Maximum keys | **64 bits total** across all keys for a single pipeline. Cooker rejects with `SH0005 VariantKeysExceeded` if the total exceeds 64. |
| Reachability pruning | The material manifest may declare `exclude: [VariantMask]` to mark combinations the cooker should skip (e.g. `SKINNED + STATIC_BATCH` cannot co-exist). Default is "compile every combination". |
| Engine-reserved keys | `SKINNED` (bool, set by `engine-renderer` when the draw call binds a bone palette), `INSTANCED` (bool), `SHADOW_PASS` (bool, set during shadow-map rendering), `MAX_LIGHTS_<N>` (enum, set by Gate 3/10/11 lighting subset). User materials must not collide with engine-reserved names; cooker rejects with `SH0006 ReservedVariantKey`. |
| Variant key bit-packing | The cooker assigns bit ranges in declaration order; the resulting `variant_key: u64` is the lookup key inside `CookedShader-v0` and is recorded in `MaterialBinding`. The bit assignment is part of the cooked artifact (so the loader does not need to re-derive it). |
| Dev-time runtime compilation | Editor / sandbox builds may compile unknown variants on demand through the `tooling-shader-runtime` feature (gated behind FD-039 ban). The newly compiled variant is added to the in-memory cache only; the on-disk artifact set is not modified. |
| Default key | Every pipeline always cooks `variant_key = 0` (all bools false, all enums first value) so the engine has a guaranteed fallback when a requested combination is missing. |

**Rationale.** Static, bit-packed variant keys give deterministic cook output, removable shader stutter at runtime, and a trivial lookup contract (`(pipeline_id, variant_key)` → `CookedShader-v0`). The 64-bit cap is large enough for realistic PBR + lighting + skinning + instancing combinations and small enough to fit in a single `u64` register.

**Downstream impact.**

- [data-schema-contracts.md](data-schema-contracts.md) `MaterialBinding`: gains `variant_key: u64`.
- `gate-05-content-authoring-base/01-code-architecture.md`: shader cook iterates the material's variant cross-product, calls `shaderc` per combination, and writes one `CookedShader-v0` per `(pipeline, variant_key, platform)`.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: extraction resolves the runtime variant by OR-ing engine-reserved bits (`SKINNED`, `INSTANCED`, etc.) into the material's authored key.

**Banned.** Open-ended `#ifdef` chains in source not tied to a declared `VariantKey`; runtime string-keyed shader lookups; release-build `shaderc` invocation.

**Not decided.** Whether to support "dynamic branching" alternative for keys with very low cost (e.g. `USE_NORMAL_MAP` as a `subgroupUniform` if-branch); deferred to Gate 11 when material complexity grows.

## FD-041: Descriptor set / bind layout convention

**Decision.** Vulkan / SPIR-V descriptor sets and bindings follow a frozen four-set layout. Every shader, every cook step, every backend, and every gameplay system reads from the same map.

| `set =` | Owner | Frequency | Contents |
|---|---|---|---|
| `0` | `engine-renderer` (per-frame globals) | once per frame | `binding=0`: `FrameUniforms` UBO (`view`, `proj`, `view_proj`, `inv_view_proj`, `camera_position`, `time`, `frame_index`). `binding=1`: `LightSSBO` (array of `LightItem` per `FD-026`). `binding=2`: shadow-map sampler array (per `FD-028`). `binding=3..15`: reserved for engine-global samplers/images. |
| `1` | material | once per material binding | `binding=0`: `MaterialUBO` (`ParamBlock.bytes`, layout determined by reflection). `binding=1..15`: material textures + samplers in declaration order (matches `TextureSlot.binding`). |
| `2` | per-draw / per-instance | once per draw call | `binding=0`: `InstanceUBO` (`world_transform`, `world_inverse_transpose`, `instance_index`). `binding=1`: optional `SkinnedBonePalette` SSBO (per `FD-007`; bound only when `SKINNED` variant key is set). `binding=2..7`: reserved for per-draw extensions. |
| `3` | `engine.subsystem.*` extensions (`FD-029`) | varies | Reserved for Gate 9 `SubsystemExtension-v0` plugins. v0 engine code does not bind set 3. |

**Push constants.** Limited to **128 bytes** (Vulkan guaranteed minimum). Reserved layout in v0:

```c
layout(push_constant) uniform PushConstants {
    uint  draw_id;          // 4 bytes, gives shader access to indirect-draw metadata
    uint  debug_flags;      // 4 bytes, per-draw debug toggles
    uint  variant_runtime;  // 4 bytes, runtime portion of variant key (e.g. light count)
    uint  _pad;             // 4 bytes
    // remaining 112 bytes reserved for future engine use
};
```

User materials and gameplay shaders must **not** add push constants; everything material-specific goes through `set=1` UBO.

**Sampler convention.** Engine ships a fixed set of named samplers in `set=0`: `s_linear_clamp`, `s_linear_wrap`, `s_nearest_clamp`, `s_nearest_wrap`, `s_shadow_compare`. Materials reference these by name; the cook step rewrites references to the actual binding slots. Custom samplers require a per-material declaration and a slot in `set=1`.

**`ParamBlock.layout_hash` derivation.** Computed as `sha256(set_index || binding_index || std140_layout_bytes)` over the material's `set=1, binding=0` UBO as reported by `spirv-reflect`. The renderer rejects with `RV0010 ParamBlockLayoutMismatch` when the runtime-supplied `ParamBlock.bytes` carries a mismatched `layout_hash`.

**Rationale.** Without a frozen bind-layout map, every shader author would invent their own set / binding numbers and every backend would need a per-shader translation table. The four-set split mirrors the standard Vulkan tutorial recommendation (globals / material / instance / extensions) and gives every gate the same vocabulary. The 128-byte push-constant cap matches the Vulkan baseline so MoltenVK and DX12 backends do not need special-case handling.

**Downstream impact.**

- `gate-02-vulkan-renderer/01-code-architecture.md`: `PipelineLayout` creation uses these four set layouts; the Vulkan backend pre-creates the descriptor set layouts at device init.
- `gate-03-scene-rendering-contract/01-code-architecture.md`: `PipelineDescriptor.bind_layouts` is populated from the convention above; the field becomes a *validation* shape, not a free-form authoring surface.
- [data-schema-contracts.md](data-schema-contracts.md) `PipelineDescriptor` and `ParamBlock` rules cite this FD for `layout_hash` derivation and binding numbering.

**Banned.** Set numbers >= 4 in v0 shaders; per-shader custom global sampler declarations; push constants outside the reserved 16-byte engine prefix.

**Not decided.** Whether to widen the push-constant prefix when 16 bytes proves insufficient; deferred to first concrete need.

## FD-042: CookedShader-v0 and PSO cache

**Decision.** Cooked shaders are a first-class `-v0` contract with their own asset kind and binary payload. Backends additionally maintain a persistent **PSO cache** for `vkPipelineCache` / `ID3D12PipelineLibrary` payloads.

**`AssetType` enum gains two entries:** `Pipeline`, `CookedShader`. `Pipeline` is the authoring-side asset (declares shader base name + variant keys + render state); `CookedShader` is the per-`(pipeline, variant_key, platform)` cooked artifact. The `MaterialBinding.pipeline: AssetId` references a `Pipeline` asset; the renderer resolves to the matching `CookedShader` at PSO build time.

**`CookedShader-v0` payload schema** (full schema lives in [data-schema-contracts.md](data-schema-contracts.md)):

- `contract_version: ContractVersion = "CookedShader-v0.1.0"`
- `pipeline_id: AssetId` — references the `Pipeline` asset.
- `variant_key: u64` — bit-packed per `FD-040`.
- `target_platform: PlatformProfile` — one of the enabled `backend-*` targets.
- `stages: { vertex: ShaderStageBlob, fragment: ShaderStageBlob, compute: Option<ShaderStageBlob> }` where `ShaderStageBlob = { spirv: Vec<u8>, glsl: Option<String>, dxil: Option<Vec<u8>>, entry_point: String, source_hash: Hash }`.
- `reflected_layout: PipelineLayoutInfo` — the `set=0..3` descriptor layouts derived by `spirv-reflect`, plus `ParamBlock.layout_hash` (per `FD-041`).
- `include_hashes: [{ path: String, hash: Hash }]` — the recursive set of `#include`d files (per `FD-038`); Gate 6 watches these.
- `cook_inputs_hash: Hash` — `sha256(source_hash || include_hashes || variant_key || engine_defines)`. The hot-reload re-cook check compares this to the live source set to decide whether a recompile is needed.

The payload is wrapped by the standard `CookedAssetHeader` (`asset_kind = CookedShader`, payload via bincode) per `FD-006`.

**PSO cache** lives at `<user_cache_dir>/<engine_name>/pso_cache/<backend>-<adapter_hash>.bin` (e.g. `~/.cache/engine/pso_cache/vulkan-<sha256(BackendCapabilities)>.bin`).

| Aspect | Rule |
|---|---|
| Population | Backend reads the file at device init and seeds `vk::PipelineCache` / `ID3D12PipelineLibrary`. Writes back at clean shutdown and on a 60-second debounce timer during long sessions. |
| Invalidation | The filename embeds `sha256(BackendCapabilities)` so any change in adapter / driver version / supported features produces a different file and old caches are simply not loaded. Cooked shader changes do not invalidate the cache (driver re-verifies SPIR-V hash internally and discards stale entries). |
| Failure handling | Read failures are logged and ignored; the backend starts with an empty cache. Write failures emit a one-time diagnostic; subsequent runs simply re-warm. |
| Per-user privacy | Cache files contain only PSO bytes; no user content. They are safe to delete at any time. |
| Mobile platforms | Android writes under the app's internal cache dir; iOS writes under `NSCachesDirectory`. iOS / MoltenVK uses `vkPipelineCache` semantics (delegated through MoltenVK). |

**Rationale.** Pinning the cooked-shader schema makes the cook ↔ runtime contract verifiable; the `cook_inputs_hash` is the single source of truth for hot-reload re-cook decisions. Persisting the PSO cache eliminates the "shader compilation stutter" on second-and-later runs without adding a separate startup pre-compile pass.

**Downstream impact.**

- [data-schema-contracts.md](data-schema-contracts.md): `AssetType` enum gains `Pipeline` and `CookedShader`; new `CookedShader-v0` section is added; `MaterialBinding.variant_key: u64` field is added (per `FD-040`).
- `gate-05-content-authoring-base/01-code-architecture.md`: cook step writes one `CookedShader-v0` per `(pipeline, variant_key, platform)` and updates the `AssetRegistry-v0` `CookedArtifact` list.
- `gate-06-iteration-workflow/01-code-architecture.md`: hot-reload watches `assets/shaders/**` and `assets/materials/**` source; on change it (1) recooks the affected `CookedShader-v0` artifacts, (2) swaps the live `vk::ShaderModule` / `Pipeline` at frame boundary, (3) keeps the previous PSO live until the swap completes.
- `gate-02-vulkan-renderer/01-code-architecture.md`: device init loads `<user_cache_dir>/.../vulkan-*.bin` into `vk::PipelineCache`; clean shutdown writes back.

**Banned.** Shipping the PSO cache as a release artifact (it is per-machine), checking it into version control, or relying on it for correctness (the engine must work with an empty cache).

**Not decided.** Whether to share PSO cache files across the editor and the player on the same machine; deferred to Gate 6 implementation.

---

## Open Foundation Questions

These are explicitly **not yet decided**. A gate that needs the answer must escalate before freezing.

| ID | Topic | Owner gate(s) | Notes |
|---|---|---|---|
| OFQ-001 | Navmesh library | Gate 13 | Choice between Recast/Detour FFI and a Rust implementation. |
| OFQ-002 | Mobile simulator hardware profile | Gate 5 | First gate to dual-report (FD-005) records the exact profile here. |
| OFQ-003 | Editor UI toolkit | Gate 5 | egui vs iced vs custom; deferred to first editor implementation. |
| OFQ-004 | NativeAOT trim profile per platform | Gate 7, Gate 19 | Per-platform script subset trim rules. |
| OFQ-005 | DSP/effects library | Gate 16 | Deferred until first product need. |
| OFQ-006 | Networking transport | Future Gate 20+ | Marked out of scope by FD-020. |
| OFQ-007 | CSM cascade count and split scheme | Gate 10 or Gate 11 | Choose 3 vs 4 cascades and the PSSM split lambda; pin per-platform defaults. |
| OFQ-008 | Light probe / SH grid baking pipeline | Post-v0 (Gate 11 may promote) | Whether to add diffuse SH probes / volumetric probe grids before Gate 19. |
| OFQ-009 | GPU cluster build for Forward+ | Post-v0 | Whether to move cluster light assignment from CPU to a compute pass. |
| OFQ-010 | Real-time global illumination | Post-v0 | SSGI / Lumen-style / voxel GI; needs a future RFC. |
| OFQ-011 | Render targets and post-process volumes | Gate 17 (production editor tools) | Per-camera `RenderTarget` asset type, `PostProcessVolume` component (bloom / depth-of-field / vignette / color grading / TAA), and per-camera tone-mapping override (see `FD-026` deferred clause). Until this OFQ is resolved, `engine.camera.render_target` is `None` only and post-processing is a single global ACES tone-map (per `FD-026`). |
| OFQ-012 | Virtual camera / Cinemachine-style track system | Gate 18 gameplay framework, or new dedicated gate | High-level gameplay camera director: virtual cameras with priority blending, dolly / spline tracks, follow / look-at constraints, camera shake / impulse channels, timeline integration. **Strictly a consumer of `engine.camera`** — does not change Gate 3 / Gate 4 `-v0` contracts. Tracking ID for the user request raised during Gate 1 design freeze. |
| OFQ-013 | Compute shader contract | Gate 10 or Gate 11 (Forward+ cluster build) | Add `ComputePipelineDescriptor` to `RHI-v0`, `.comp.glsl` activation per `FD-037`, dispatch / barrier rules, and `ComputeShader` variant of `ShaderStageBlob` in `CookedShader-v0`. Strictly additive; v0 does not ship compute. |
| OFQ-014 | Mesh / ray tracing / geometry shader scope | Post-v0 | Whether to add mesh-shader and RT pipelines to a future `RHI-v1`. **Geometry shaders are banned in all versions** (matches MoltenVK feature subset per `FD-003` and the explicit `FD-037` v0 ban). |

When an OFQ is resolved, promote it to an `FD-###` entry above, remove it from this table, and add it to the decision index.
