# Gate 5 Validation And Acceptance

## Gate Exit Principle

Gate 5 is accepted only when assets, minimal editor workflow, and C# scripting can all consume the frozen ECS scene model without changing it.

## Verification Goals

- Prove the engine can cook and load assets through `AssetRegistry-v0`.
- Prove the minimal editor can inspect, edit, save, and reload ECS scenes.
- Prove C# scripts can load, run lifecycle callbacks, serialize fields, and fail safely.

## Required Results

- Source asset manifest, cook pipeline, dependency graph, cooked manifest, registry API, cache/versioning, and validation commands exist.
- Minimal editor can load an ECS scene, select entities, edit core fields, create/delete entities, save, and reload.
- C# script components attach to entities, execute lifecycle callbacks, and serialize supported fields.

## Acceptance Checklist

- [ ] `AssetRegistry-v0` is documented and frozen.
- [ ] `ScriptAPI-v0` is documented and frozen.
- [ ] Cook-only command produces cooked assets for the validation scene.
- [ ] Validate-assets command catches missing/broken references.
- [ ] Editor save/load preserves edits to transforms, cameras, lights, names, and renderable references.
- [ ] Script exceptions are reported and disable the failing script instance without crashing the engine.
- [ ] C# API does not expose renderer backend internals.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- Cook-only validation command
- Validate-assets command
- Script component scene round-trip tests

## Manual Validation

- Open sandbox/editor mode, edit a scene, save it, reload it, and confirm changes persist.
- Attach a sample C# script that updates a transform and logs lifecycle callbacks.
- Trigger a deliberate script exception and confirm safe recovery.
- Corrupt an asset reference and confirm diagnostics identify the dependency chain.

## Blocking Issues

- Asset IDs or cooked manifest format are still unstable.
- Editor directly edits backend-specific data.
- C# scripts can access Vulkan/OpenGL/DX12 objects.
- Script exceptions crash the process.
- Cooked scene cannot render through the registry path.

## Required Evidence

- Cook/validate command outputs.
- Editor save/load test result.
- Sample C# script run log.
- Script exception diagnostic example.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 6: yes/no

