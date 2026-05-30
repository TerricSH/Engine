# Gate 4: ECS Scene Runtime

## Purpose

Create the runtime scene model that later systems will share. This gate freezes core components, scene serialization basics, and renderer extraction.

## Entry Sync Point

- `RendererInput-v0` is frozen.

## Parallel Workstreams

1. ECS Core
   - Owns `crates/engine-scene` (per `FD-029`).
   - Implements entity IDs, component storage, queries, add/remove behavior, and basic system order.
2. Scene Serialization
   - Owns `crates/engine-serialize` or serialization modules.
   - Defines schema version, entity IDs, component format, asset references, active camera, and validation rules.
3. Renderer Extraction
   - Owns ECS-to-renderer extraction in `engine-core`.
   - Converts ECS components to `RendererInput-v0` without backend-specific code.

## Contracts To Freeze

- `ECSScene-v0`
- Core components: `Name`, `Transform`, `Renderable`, `Camera`, `Light`, `Bounds`
- Scene schema/versioning rules
- Renderer extraction contract

## Exit Condition

- ECS-loaded scene renders the same as pre-ECS scene.
- Scene saves and reloads without data loss.
- Core component schema is frozen for later gates.

## Parallel Safety Notes

- Asset, editor, and script sessions can read drafts but should not merge deep integration before this gate closes.
- Any future component extension should use a plugin-style extension surface rather than editing central schemas directly.
