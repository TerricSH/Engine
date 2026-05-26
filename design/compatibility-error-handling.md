# Compatibility And Error Handling

This document defines how gates freeze contracts, evolve schemas, reject incompatible data, and recover from failures. It is intentionally cross-cutting so implementation sessions do not invent different rules for scenes, assets, packages, scripts, UI, audio, and release artifacts.

## Freeze Semantics

| Freeze type | Meaning | Allowed compatible change | Incompatible change |
|---|---|---|---|
| API freeze | Public Rust/C#/tooling signatures are stable for downstream gates. | Add a new method behind a feature flag or add an overload that does not alter existing behavior. | Rename/remove method, change return/error type, alter ownership or threading contract. |
| Schema freeze | Serialized logical fields are stable. | Add optional field with deterministic default; add enum value only if older consumers reject it cleanly. | Rename/remove required field, change field meaning, serialize runtime-only handles, alter identity rules. |
| Behavior freeze | Observable behavior required by tests is stable. | Narrow bug fix that preserves documented outcomes. | Change validation timing, fallback behavior, or mutation/rollback guarantees. |
| Platform policy freeze | Platform-specific constraints are stable. | Add stricter validation that rejects unsafe packages earlier. | Permit a payload or runtime behavior previously forbidden by policy, especially iOS executable updates. |

Every frozen contract must have an owner gate, a version, a validation command, and rejection behavior.

## Compatibility Matrix

| Contract | Owner | Primary consumers | Compatibility check | Rejection behavior |
|---|---|---|---|---|
| `RHI-v0` | Gate 1 | Gate 2 renderer backends | Backend reports required features and limits before device creation | Device creation returns `UnsupportedFeature` or `UnsupportedLimit`; no partial device |
| `RendererInput-v0` | Gate 3 | ECS extraction, UI, debug draw, animation runtime (skinned items, v0.2), renderer | Contract version and required asset/material fields before frame submission; skinned-item palette/skeleton size match | Frame is not submitted; skinned item with palette/skeleton mismatch is dropped with diagnostic while rest of frame still submits |
| `ECSScene-v0` | Gate 4 | Editor, scripts, prefabs, gameplay systems | Schema version, component registry, IDs, references, active camera | Load fails before runtime world mutation |
| `AssetRegistry-v0` | Gate 5 | Renderer, scenes, scripts, hot update, packages | Registry schema, asset IDs, hashes, dependencies, platform profile | Snapshot swap is aborted; old registry remains active |
| `CookedShader-v0` | Gate 5 (cook) / Gate 2 (consumer) | All render backends (`render-vulkan`, `render-opengl`, `render-dx12`) | Contract version, `target_platform` matches runtime platform, `entry_point == "main"`, four-set `reflected_layout` shape, presence of `glsl` / `dxil` blobs matches enabled `backend-*` features, `cook_inputs_hash` equality | Shader load is rejected with `SH0001 NonMainEntryPoint` / `SH0007 PlatformMismatch`; renderer falls back to `variant_key = 0` with `RV0011 ShaderVariantMissing` if the requested variant is absent; PSO cache mismatch is silently ignored (cache is per-machine) |
| `ScriptAPI-v0` | Gate 5 | C# scripts, mobile subset, gameplay APIs | Required API version and platform capability flags at assembly/component load | Script component is faulted or disabled; world continues |
| `MobileHotUpdate-v0` | Gate 7 | Gate 8 installer, Gate 19 release | Engine/script/content version ranges, hashes, signatures, platform payload policy | Package is rejected before activation |
| `PackageInstallState-v0` | Gate 8 | Asset registry, logic runtime, release QA | State transition table and previous-known-good pointer | Resume, reject, or roll back; active package is not overwritten |
| `SubsystemExtension-v0` | Gate 9 | Physics, animation, UI, audio, nav, editor | Descriptor versions, unique IDs, component migrations | Registration fails for that subsystem; engine reports blocking diagnostic |
| `Physics/Animation-v0` | Gate 10 | Character, AI, editor, scripts | Component schema, backend-handle exclusion, fixed-step/event timing | Scene load or subsystem init fails before simulation |
| `CharacterController-v0` | Gate 12 | AI, gameplay scripts | Public movement API and transform authority rules | Controller command is rejected; transform is not overwritten |
| `NavAI-v0` | Gate 13 | Gameplay framework, behavior runtime | Navmesh asset schema, agent/controller binding | Agent enters diagnostic stopped state; no direct transform teleport |
| `Prefab-v0` | Gate 14 | Editor, scenes, hot update, gameplay | Prefab schema, source version, override target paths, cycle detection | Runtime rejects unresolved overrides; editor may preserve them for repair |
| `RuntimeUI-v0` | Gate 15 | Renderer, scripts, gameplay | Canvas/node/widget schema and input routing contract | Invalid tree is not activated; previous UI tree remains |
| `Audio-v0` | Gate 16 | Gameplay, scripts, editor | Asset format, source/listener component schema, mixer group IDs | Missing/invalid source is silent with diagnostic; mixer stays alive |
| `ReleaseMetadata-v0` | Gate 19 | Packaging, QA, rollback, diagnostics | Artifact hashes, signatures, QA and performance report links | Artifact cannot be promoted to release candidate |

## Versioning And Migration Policy

1. Contract versions use semantic versioning even when the document name says `v0`.
2. `major` changes are incompatible and require a migration or explicit downstream gate update.
3. `minor` changes may add optional fields with deterministic defaults and validation.
4. `patch` changes may clarify validation or fix bugs without changing serialized meaning.
5. Migrations are pure transformations from an older logical schema to a newer one. They must not require renderer, physics, audio, script, or platform runtime state.
6. Migration diagnostics must include source version, target version, field path, and whether data was preserved, defaulted, or rejected.
7. Unknown fields are preserved only in editor repair mode. Runtime load, package activation, and release QA reject unknown required fields.

## Diagnostics Envelope

Every validation, load, reload, package, and release error should be representable as:

```text
Diagnostic {
  code: String,
  severity: info | warning | error | fatal,
  system: String,
  contract: Option<String>,
  version: Option<String>,
  message: String,
  path: Option<String>,
  entity: Option<PersistentId>,
  asset: Option<AssetId>,
  package_id: Option<String>,
  recoverable: bool,
  suggested_action: Option<String>
}
```

Rules:

- Diagnostics are emitted before mutation when validation can run ahead of time.
- `fatal` diagnostics block activation, scene mutation, or device creation.
- User/content errors must identify an asset, entity, package, field path, or command.
- Programmer invariants may panic in tests, but release/editor paths should convert expected validation failures into diagnostics.

## Diagnostic Code Registry

Every `Diagnostic.code` string used anywhere in the engine MUST be registered in the tables below. New codes are added by the owning gate's architecture document and copied here in the same PR. The registry exists to prevent code collisions, enable global searchability, and document each code's stable meaning across the workspace.

### Code Notation

Two notation styles coexist and **must not** be mixed within one subsystem:

1. **Prefix + four-digit suffix** (`SH0001`, `RV0010`, `SC0014`) — used for cook-time, asset, scene, and renderer-validation diagnostics. The four-digit suffix allows deliberate gaps that group related codes (e.g. `SH0001-0009` are cook-time shader errors; `SH0010+` is reserved for runtime shader-loader errors). The code body (`NonMainEntryPoint`, etc.) is a stable PascalCase identifier referenced from inline citations.
2. **Dotted lower-case** (`rhi.invalid_handle`, `rhi.backend`) — used for `RhiError` enum variants mapped 1:1 from `thiserror` per `FD-032`. Codes derive mechanically as `<subsystem>.<snake_case_variant>`.

### Reserved Prefix Namespace

| Prefix | Owning gate(s) | Scope |
|---|---|---|
| `SH` | Gate 5 (cook) / Gate 2 (loader) | Shader cook and runtime shader-loader diagnostics (`FD-004`, `FD-037`–`FD-040`) |
| `RV` | Gate 3 (renderer) / Gate 4 (extraction) | Renderer input validation, view composition, variant lookup, descriptor binding (`FD-035`, `FD-036`, `FD-040`, `FD-041`) |
| `SC` | Gate 4 (scene) | Scene extraction and component-level validation (`FD-034`) |
| `AS` | Gate 5 (asset) | Reserved — generic asset cook / `AssetRegistry-v0` diagnostics not covered by `SH` |
| `HU` | Gate 7 (contracts) / Gate 8 (installer) | Reserved — `MobileHotUpdate-v0` validation, `PackageInstallState-v0` transitions |
| `PR` | Gate 14 | Reserved — `Prefab-v0` resolution, override paths, cycle detection |
| `UI` | Gate 15 | Reserved — `RuntimeUI-v0` tree validation, input routing |
| `AU` | Gate 16 | Reserved — `Audio-v0` source/listener/mixer diagnostics |
| `NV` | Gate 13 | Reserved — `NavAI-v0` navmesh load and agent state |
| `RP` | Gate 19 | Reserved — `ReleaseMetadata-v0` packaging, signing, QA |
| `rhi.*` | Gate 1 | `RhiError` variant mapping per `FD-032` (see [gate-01-workspace-rhi/01-code-architecture.md](gate-01-workspace-rhi/01-code-architecture.md) § Error Model) |

### Active Codes

#### Shader (`SH`)

| Code | Severity | Emitter | Meaning |
|---|---|---|---|
| `SH0001` `NonMainEntryPoint` | error | Gate 5 cook / Gate 2 loader | Shader source declares an entry point other than `main` (`FD-037`). |
| `SH0002` `UnsupportedShaderVersion` | error | Gate 5 cook | First non-comment line is not `#version 450 core` (`FD-004`). |
| `SH0003` `IncludeCycle` | error | Gate 5 cook | Recursive `#include` chain detected (`FD-038`). |
| `SH0004` `IncludeDepthExceeded` | error | Gate 5 cook | Include depth exceeds 16 (`FD-038`). |
| `SH0005` `VariantKeysExceeded` | error | Gate 5 cook | Material declares `VariantKey` set whose total bit width exceeds 64 (`FD-040`). |
| `SH0006` `ReservedVariantKey` | error | Gate 5 cook | User material declares a variant key colliding with an engine-reserved name (`SKINNED`, `INSTANCED`, `SHADOW_PASS`, `MAX_LIGHTS_<N>`; `FD-040`). |
| `SH0007` `PlatformMismatch` | error | Gate 2 loader | `CookedShader-v0.target_platform` does not match runtime platform (`FD-037`). |

`SH0008`–`SH0009` reserved for additional cook-time errors. `SH0010+` reserved for runtime shader-loader errors.

#### Renderer (`RV`)

| Code | Severity | Emitter | Meaning |
|---|---|---|---|
| `RV0007` `OverlayBaseMissing` | warning | Gate 3 renderer | An `Overlay` `RenderView`'s `base_view_id` is absent in the current frame; the overlay is dropped and the frame continues (`FD-035`). |
| `RV0008` `MultipleBaseCameras` | warning | Gate 4 extraction | Two enabled cameras at the same `priority` are authored as `Base`; the lower `view_id` wins (`FD-035`). |
| `RV0009` `CulledItemSubmitted` | warning (debug-only) | Gate 3 renderer | Debug-build consistency check: a submitted item lies fully outside one of the view's frustum planes (`FD-036`). |
| `RV0010` `ParamBlockLayoutMismatch` | error | Gate 3 renderer | Runtime-supplied `ParamBlock.layout_hash` does not match the pipeline's expected layout (`FD-041`). |
| `RV0011` `ShaderVariantMissing` | warning (one-shot per pipeline) | Gate 3 renderer | Requested `(pipeline_id, variant_key)` has no cooked variant; renderer falls back to `variant_key = 0` (`FD-040`). |

`RV0001`–`RV0006` retired during contract drafting and not reused; `RV0012+` available for future Gate 3 / Gate 4 allocations.

#### Scene (`SC`)

| Code | Severity | Emitter | Meaning |
|---|---|---|---|
| `SC0014` `RenderTargetNotInV0` | error | Gate 4 extraction | `engine.camera.render_target` is non-`None`, which is reserved for post-v0 (`OFQ-011`); the camera is treated as if it rendered to the swapchain (`FD-034`). |

`SC0001`–`SC0013` retired during contract drafting and not reused; `SC0015+` available for future Gate 4 allocations.

#### RHI (`rhi.*`)

Mechanically mirrored from the `RhiError` enum per `FD-032`. The authoritative variant → code → severity mapping lives in [gate-01-workspace-rhi/01-code-architecture.md](gate-01-workspace-rhi/01-code-architecture.md) § Error Model; this section restates it for cross-subsystem readers.

| Code | Severity | Meaning |
|---|---|---|
| `rhi.unsupported_backend` | fatal | Backend feature not built into the binary |
| `rhi.unsupported_feature` | fatal | Required Vulkan / DX feature absent on device |
| `rhi.unsupported_limit` | fatal | Required Vulkan / DX limit not met |
| `rhi.invalid_descriptor` | error | Caller supplied an invalid `*Descriptor` |
| `rhi.invalid_handle` | error | Handle is already destroyed or belongs to another device |
| `rhi.device_lost` | fatal | Backend reports device lost |
| `rhi.surface_lost` | error (recoverable) | Backend reports surface lost; renderer may recreate |
| `rhi.out_of_memory` | fatal | Backend allocator returned OOM |
| `rhi.allocation_failed` | error | Specific allocation failed without exhausting the device |
| `rhi.validation_failed` | error | Vulkan / DX validation layer reported a violation |
| `rhi.incompatible_bind_layout` | error | Pipeline descriptor violates the four-set convention (`FD-041`) |
| `rhi.backend` | error | Backend-specific error surfaced as an opaque string |

### Allocation Rules

1. Adding a new code requires editing this registry in the same PR as the architecture / contract change that introduces it.
2. Within a prefix, allocate the next free four-digit suffix. **Do not reuse** suffixes of removed or retired codes — leave them noted in the section header above.
3. Codes are public; renaming a code (either the suffix or the PascalCase identifier) is a contract break and follows the contract change workflow at the bottom of this document.
4. When in doubt about which prefix to use, the prefix follows the **emitter**, not the contract. A renderer-side validation of a scene contract belongs under `RV`, not `SC`.
5. Reserved prefixes (`AS`, `HU`, `PR`, `UI`, `AU`, `NV`, `RP`) may be activated as soon as their owning gate emits its first code; no separate reservation step is needed. Conversely, owning gates may rename their reserved prefix only before the first active code is allocated under it.

## Failure Handling Rules

| Area | Failure rule |
|---|---|
| Scene load | Validate into a temporary scene model first. Do not mutate the active runtime world until all required references, components, and active camera rules pass. |
| Renderer frame | If extraction creates invalid `RendererInput-v0`, skip submission for that frame and keep the previous valid swapchain/device state. Device loss follows backend recovery rules. |
| Asset cook | Cook into a temporary output and registry snapshot. On failure, keep previous cooked artifacts and registry active. |
| Hot reload | Apply to a staging resource, validate, then swap. Failed reload restores the previous live resource and emits diagnostics. |
| Script exception | Mark the script instance faulted for the frame or until reset policy runs. Do not abort the whole scene unless the callback is part of gate validation. |
| Physics step | Queue scene mutations to fixed-step boundaries. Reject mid-step C# mutations with diagnostics instead of applying them immediately. |
| Animation eval | Invalid clip/skeleton binding disables that animation player and reports the asset/component path. Other animation players continue. |
| Package install | Download and verify in staging. Activation is a pointer switch. Boot failure rolls back to previous known-good package. |
| Prefab source missing | Runtime rejects the instance. Editor repair mode keeps instance data and unresolved override records for user repair. |
| UI tree update | Validate a new tree off to the side. On failure, keep the previous active tree and report path to invalid node/widget. |
| Audio decode/source | A failing source produces silence and diagnostics; mixer callback must not block, panic, or access unloaded assets synchronously. |
| Release packaging | Any missing hash, signature, QA report, performance report, or symbol mapping blocks release candidate promotion. |

## Cross-Gate Integration Tests

Each gate that consumes a prior frozen contract must add at least one integration test that proves compatibility rather than only testing local code.

Required recurring tests:

1. Load the newest `ECSScene-v0` fixture, run asset validation, extract `RendererInput-v0`, and render or snapshot it.
2. Cook assets, build `AssetRegistry-v0`, load a scene that references those assets, and reject one broken dependency.
3. Load a scene with script components, run one successful callback and one failing callback, and verify world integrity.
4. Validate a `MobileHotUpdate-v0` manifest against desktop, Android, and iOS profiles; confirm iOS rejects executable payloads.
5. Stage, activate, fail boot validation, and roll back a package without corrupting the active registry.
6. Instantiate a prefab with physics, animation, script, UI, and audio components; save/load the scene and verify override paths still resolve.
7. Run a gameplay loop scene through Gate 18 and collect Gate 19 diagnostics/performance reports.

## Contract Change Workflow

1. Propose the change in the owning gate's architecture or requirements document.
2. Update `data-schema-contracts.md` if serialized fields, identity rules, or public contract fields change.
3. Update this document if compatibility, migration, or recovery behavior changes.
4. Add or update validation fixtures and integration tests.
5. Update the affected gate's `02-validation-acceptance.md` and `07-test-plan.md`.
6. Downstream gates may proceed only after the changed contract has a version and rejection behavior.
