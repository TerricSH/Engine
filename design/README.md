# Engine Design Documents

This folder is the authoritative design source for the engine. It is organized into 19 sequential gates plus three cross-cutting documents that every gate must respect.

Each gate freezes a small, well-defined contract before downstream gates build on it. The cross-cutting documents define how those contracts are versioned, validated, recovered, and measured so that gates stay independent.

## Cross-Cutting Documents

| Document | Scope |
|---|---|
| [compatibility-error-handling.md](compatibility-error-handling.md) | Freeze semantics, the canonical compatibility matrix, diagnostics envelope, failure-handling rules, cross-gate integration tests, and the contract change workflow. |
| [data-schema-contracts.md](data-schema-contracts.md) | Field-level logical schemas for every `<Name>-v0` contract referenced in the matrix. |
| [performance-budgets.md](performance-budgets.md) | Measurement protocol, baseline hardware classes, and per-gate p50/p95/max budgets for CPU, GPU, memory, and operation latency. |
| [foundation-decisions.md](foundation-decisions.md) | Cross-cutting engineering decisions (`FD-###`) covering toolchain, threading, hosting, backends, feature flags, determinism, licensing, and out-of-scope topics. |
| [lighting-system.md](lighting-system.md) | Shading model, color pipeline, light units, shading pipeline (forward/forward+), shadow algorithm, environment lighting, mobile/desktop subset, per-gate ownership. |

These five documents are the source of truth. If a gate document and a cross-cutting document disagree, the cross-cutting document wins and the gate document must be updated through the contract change workflow.

## Gate Index

Gates are executed in order. A gate may not start until the previous gate exits cleanly and its frozen contracts are recorded.

| # | Gate | Owns / Freezes |
|---|---|---|
| 1 | [Workspace And RHI Foundation](gate-01-workspace-rhi/README.md) | Workspace skeleton, `RHI-v0` |
| 2 | [Vulkan Renderer](gate-02-vulkan-renderer/README.md) | First working Vulkan backend implementing `RHI-v0` |
| 3 | [Scene Rendering Contract](gate-03-scene-rendering-contract/README.md) | `RendererInput-v0` |
| 4 | [ECS Scene Runtime](gate-04-ecs-scene-runtime/README.md) | `ECSScene-v0` |
| 5 | [Content Authoring Base](gate-05-content-authoring-base/README.md) | `AssetRegistry-v0`, `ScriptAPI-v0`, editor base |
| 6 | [Iteration Workflow](gate-06-iteration-workflow/README.md) | Hot reload, scripting/editor iteration loops |
| 7 | [Mobile Hot Update Contracts](gate-07-mobile-hot-update-contracts/README.md) | Mobile runtime profiles, `MobileHotUpdate-v0` |
| 8 | [Hot Update Package](gate-08-hot-update-package/README.md) | `PackageInstallState-v0`, install/rollback runtime |
| 9 | [Subsystem Extension Contracts](gate-09-subsystem-extension-contracts/README.md) | `SubsystemExtension-v0` |
| 10 | [Gameplay Subsystems Foundation](gate-10-gameplay-subsystems-foundation/README.md) | `Physics/Animation-v0` |
| 11 | [Gameplay Subsystems Expansion](gate-11-gameplay-subsystems-expansion/README.md) | Expanded physics/animation features behind frozen contracts |
| 12 | [Character Controller](gate-12-character-controller/README.md) | `CharacterController-v0` |
| 13 | [Navigation And AI](gate-13-navigation-ai/README.md) | `NavAI-v0` |
| 14 | [Prefab Scene Composition](gate-14-prefab-scene-composition/README.md) | `Prefab-v0` |
| 15 | [Runtime UI](gate-15-runtime-ui/README.md) | `RuntimeUI-v0` |
| 16 | [Audio System](gate-16-audio-system/README.md) | `Audio-v0` |
| 17 | [Production Editor Tools](gate-17-production-editor-tools/README.md) | Production-grade editor surfaces |
| 18 | [Gameplay Framework And Platform](gate-18-gameplay-framework-platform/README.md) | Game state, input actions, event bus, platform capabilities |
| 19 | [Release Pipeline](gate-19-release-pipeline/README.md) | `ReleaseMetadata-v0`, CI/CD, packaging, QA, diagnostics |

## Per-Gate File Layout

Every gate folder uses the same eight-file layout:

| File | Purpose |
|---|---|
| `README.md` | One-page overview: purpose, entry sync point, parallel workstreams, contracts to freeze, exit condition. |
| `01-code-architecture.md` | Whole-system diagram at gate exit, frozen contracts, architectural notes, open questions, detailed module design. |
| `02-validation-acceptance.md` | Gate exit principle, required results, acceptance checklist, blocking conditions, exit decision. |
| `03-best-practices.md` | Patterns to follow and anti-patterns to avoid for the gate's scope. |
| `04-performance-report.md` | Concrete numbers measured against [performance-budgets.md](performance-budgets.md). |
| `05-feature-requirements.md` | Numbered required features, target effects, explicit non-goals, AI execution rules, completion signal. |
| `06-session-prompts.md` | Ready-to-use prompts for the implementation sessions that own this gate. |
| `07-test-plan.md` | Test matrix mapping each feature to unit/integration/manual coverage. |

## How To Use These Documents

1. Read [compatibility-error-handling.md](compatibility-error-handling.md) and [data-schema-contracts.md](data-schema-contracts.md) before any session that introduces or consumes a `-v0` contract.
2. Read [performance-budgets.md](performance-budgets.md) before starting a gate's `04-performance-report.md`.
3. Inside a gate, follow file order: README → 01 → 05 → 03 → 02 → 07 → 04 → 06.
4. When a gate's frozen contract changes, follow the contract change workflow at the end of [compatibility-error-handling.md](compatibility-error-handling.md).

## Contract Quick Reference

The canonical `-v0` contract names live in the compatibility matrix and the schema doc. Use those exact names in gate documents and code. The current set is:

`RHI-v0`, `RendererInput-v0`, `ECSScene-v0`, `AssetRegistry-v0`, `ScriptAPI-v0`, `MobileHotUpdate-v0`, `PackageInstallState-v0`, `SubsystemExtension-v0`, `Physics/Animation-v0`, `CharacterController-v0`, `NavAI-v0`, `Prefab-v0`, `RuntimeUI-v0`, `Audio-v0`, `ReleaseMetadata-v0`.

Gate documents must not invent new `-v0` names without first adding them to the matrix and the schema doc.

## Engineering Governance

Cross-cutting engineering choices (language toolchain, threading topology, .NET hosting, backend libraries, feature-flag scheme, determinism rules, license, **workspace crate layout (FD-029), math library (FD-030), coordinate system / NDC / units (FD-031), error handling crate split (FD-032), cross-thread channel crate (FD-033)**) are frozen in [foundation-decisions.md](foundation-decisions.md) as `FD-###` entries. Before introducing a new dependency, threading pattern, platform abstraction, math type, error type, or channel, check that an `FD` does not already decide it. New cross-cutting decisions must be added there through the same contract change workflow.

Every `Cargo.toml` in the workspace must carry:

- `edition = "2021"` and a `rust-version` matching the workspace MSRV (per `FD-024`).
- `license = "MIT OR Apache-2.0"` (per `FD-025`).
- Features that follow the `backend-*` / `subsystem-*` / `tooling-*` / `target-*` taxonomy (per `FD-010`); the editor lives behind `tooling-editor` (per `FD-011`).
- The crate's name must appear in the `FD-029` crate table; no ad hoc workspace members.
- Library crates use `thiserror` for typed error enums (per `FD-032`); only `sandbox` and other binaries use `anyhow`.

Release artifacts must include a `NOTICES.txt` generated from the dependency graph (per `FD-025`); CI fails the release job if it is missing.

### Unsafe Code Policy

- `render-vulkan`, `engine-audio` (cpal callback), and `engine-script` (.NET hosting FFI) are the **only** crates allowed to contain `unsafe` blocks in non-test code. All other engine crates carry `#![forbid(unsafe_code)]` at the crate root.
- Every `unsafe` block in the three permitted crates must be preceded by a `// SAFETY:` comment that names the invariant being upheld (lifetime, alignment, thread context, FFI contract).
- `cargo geiger` (or equivalent) runs in CI; any new `unsafe` block in a forbidden crate fails the build.

## Non-Goals (v0)

The following are explicitly out of scope for Gates 1-19 and have no `-v0` contract:

- **Networking and multiplayer** (transport, replication, matchmaking). See `FD-020`. May be introduced in a future Gate 20+ group.
- **Visual scripting** beyond what `ScriptAPI-v0` exposes for C#.
- **Procedural content generation runtime** beyond what `AssetRegistry-v0` allows for cooked data.
- **Server / dedicated headless mode** outside of cook and CI fixtures.

Attempts to add any of the above must start as a new RFC against [foundation-decisions.md](foundation-decisions.md), not as a hidden gate scope expansion.
