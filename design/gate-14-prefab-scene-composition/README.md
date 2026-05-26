# Gate 14: Prefab And Scene Composition System

## Purpose

Create reusable entity templates and composition patterns for gameplay content. This gate turns repeated scene setups into prefabs, variants, archetypes, and pooled runtime entities.

## Entry Sync Point

- Gate 13 navigation/AI and Gate 12 character patterns are stable enough to become reusable templates.
- Scene serialization and editor basics are stable.

## Parallel Workstreams

1. Prefab Asset Format
   - Owns prefab schema, nested entity structure, component lists, asset references, versioning, and validation.
   - Uses asset registry extension APIs; does not casually change core scene schema.
2. Prefab Instances And Overrides
   - Adds instance tracking, property overrides, component enable/disable overrides, and nested prefab references.
   - Defines override conflict behavior and missing-source handling.
3. Editor Prefab Workflow
   - Adds create prefab, instantiate prefab, inspect overrides, apply/revert changes, and broken reference diagnostics.
   - Keeps advanced visual prefab graph tools deferred.
4. Gameplay Archetypes
   - Adds curated templates such as player, enemy, pickup, door trigger, static prop, camera rig, and light setup.
   - Archetypes are data-driven and can include C# script components.
5. Object Pooling And Lifecycle
   - Adds pooling manager for projectiles, effects, temporary enemies, and reusable runtime entities.
   - C# APIs for spawn/despawn/pool return.

## Exit Condition

- Prefab instances save/load with overrides intact.
- Editor can create, instantiate, apply, and revert prefab changes.
- Gameplay test scene uses prefabs/archetypes instead of hand-built repeated entities.
- Object pooling reduces runtime churn for repeated spawn/despawn flows.

## Non-Goals

- Procedural world generation, full visual prefab graph editor, networked prefab replication, and automatic code generation.

## Parallel Safety Notes

- Prefab schema changes must be versioned.
- Prefab work should not break base scene schema.
- Pooling must use public ECS lifecycle APIs.
