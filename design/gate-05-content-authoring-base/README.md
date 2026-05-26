# Gate 5: Content Authoring Base

## Purpose

Start the authoring layer after ECS scenes are stable. This gate can run asset pipeline, minimal editor, and C# scripting foundation in parallel because they consume the same frozen ECS scene model.

## Entry Sync Point

- `ECSScene-v0` is frozen.

## Parallel Workstreams

1. Asset Pipeline And Cooking
   - Owns `crates/engine-asset` (per `FD-029`).
   - Implements source manifest, cook rules, dependency graph, cooked manifest, runtime registry, cache/versioning, and validation commands.
2. Minimal Scene Editor
   - Owns `crates/engine-editor`.
   - Implements editor mode, hierarchy, selection, inspector, entity create/delete, save/load, and small undo/redo.
   - Uses placeholder or pre-cooked assets until the registry is ready.
3. C# Scripting Foundation
   - Owns `crates/engine-script` and `scripts/csharp`.
   - Implements C# hosting facade, script component model, lifecycle callbacks, small engine API, serializable fields, and safe error handling.

## Contracts To Freeze

- `AssetRegistry-v0`
- `ScriptAPI-v0`
- Minimal editor interaction rules

## Exit Condition

- `AssetRegistry-v0` is frozen.
- `ScriptAPI-v0` is frozen.
- Minimal editor can inspect/edit/save ECS scenes.
- C# scripts load, run lifecycle callbacks, serialize fields, and fail safely.

## Parallel Safety Notes

- Asset, editor, and script sessions own separate crates.
- Editor and scripting may use asset references only through the registry API once frozen.
- Script API must not expose renderer backend internals.
