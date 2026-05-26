# Gate 4 Validation And Acceptance

## Gate Exit Principle

Gate 4 is accepted only when the static renderer scene can be represented, saved, loaded, and rendered through `ECSScene-v0`.

## Verification Goals

- Prove the engine has an ECS-backed runtime scene model that can feed the Gate 3 renderer input.
- Prove scene save/load is stable enough for assets, editor, and C# scripting to build on.
- Prove the core component schema is frozen and versioned.

## Required Results

- `ECSScene-v0` is documented and implemented.
- Core components exist: `Name`, `Transform`, `Renderable`, `Camera`, `Light`, `Bounds`.
- Scene files include schema version and enough validation metadata.
- ECS-to-renderer extraction produces the same visual result as the pre-ECS static scene.

## Acceptance Checklist

- [ ] ECS supports entity IDs, component storage, queries, add/remove, and deterministic basic system order.
- [ ] Scene serialization round-trips all core components.
- [ ] Invalid scenes fail validation with clear diagnostics.
- [ ] Missing active camera is detected.
- [ ] Renderer extraction reads ECS and emits `RendererInput-v0`.
- [ ] Old temporary scene submission path is removed or marked as test-only.
- [ ] `deterministic_replay` integration test passes: load a fixture scene, tick `N` frames, and verify the resulting world state and extracted `RendererInput-v0` (including any `skinned_items`) are byte-identical across two runs on the same machine and across at least two host platforms in CI (per `FD-012`).

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- Scene round-trip tests for core components
- Renderer extraction tests comparing expected renderer input
- `deterministic_replay` integration test (cross-platform CI matrix per `FD-012`)

## Manual Validation

- Load the ECS scene in sandbox and compare with the pre-ECS visual baseline.
- Save the scene, reload it, and verify entity/component counts match.
- Move camera and confirm culling/draw statistics still behave correctly.

## Blocking Issues

- Scene format has no schema version.
- Renderer extraction requires backend-specific code.
- Core components are still changing in ways that would break editor/assets/scripts.
- Save/load loses entity identity, transform data, material references, lights, or camera settings.
- `deterministic_replay` produces divergent output across runs or platforms; any use of `std::collections::HashMap`/`HashSet` in iteration paths (per `FD-012`).

## Required Evidence

- Scene round-trip test output.
- Screenshot before and after ECS migration.
- Scene schema review note.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 5: yes/no

