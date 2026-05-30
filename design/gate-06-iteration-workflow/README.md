# Gate 6: Iteration Workflow

## Purpose

Improve development speed after assets and scripts have stable contracts. This gate adds hot reload, script/editor integration, and runtime diagnostics without turning the editor into a production tool yet.

## Entry Sync Point

- `AssetRegistry-v0` is frozen.
- `ScriptAPI-v0` is available for editor integration.

## Parallel Workstreams

1. Hot Reload
   - Owns reload modules in `crates/engine-asset` and Vulkan resource replacement integration.
   - Supports texture, material, shader, and scene reload at frame boundaries.
2. Editor Asset And Script Integration
   - Owns editor asset assignment UI, script component inspector, C# build command integration, and diagnostics.
3. Registry-Driven Sandbox Diagnostics
   - Owns sandbox diagnostics for loaded assets, reload status, script errors, draw calls, culling, and scene validation.

## Contracts To Preserve

- `AssetRegistry-v0`
- `ScriptAPI-v0`
- Frame-boundary resource replacement rules

## Exit Condition

- Texture/material/shader/scene hot reload works in the running sandbox.
- Editor can assign assets and script components safely.
- Failed reloads preserve last valid assets.

## Parallel Safety Notes

- Hot reload must not change asset IDs or cooked manifest semantics.
- Script editor integration consumes script metadata; it does not change runtime hosting internals without coordination.
- Reload failures must be reported without corrupting ECS or renderer state.
