# Gate 5 Feature Requirements And Execution Boundaries

## Gate Objective

Build the first authoring base: asset cooking and registry, minimal editor, and C# scripting foundation, all consuming `ECSScene-v0` without changing it.

## Required Features

### G5-F01 Asset Manifest And Cook Pipeline

Required behavior:
- Define source asset manifest entries.
- Implement cook rule dispatch by asset type.
- Write deterministic cooked artifacts at agreed scope.
- Store cooked manifest metadata.

Minimum output:
- Validation scene assets cook successfully.

### G5-F02 Dependency Graph And Validation

Required behavior:
- Track asset dependencies and reverse dependencies.
- Validate missing files, broken references, unsupported formats, and stale cooked outputs.

Minimum output:
- Diagnostics identify the broken dependency chain.

### G5-F03 Runtime Asset Registry

Required behavior:
- Resolve asset references to load states and runtime handles.
- Provide stable asset IDs or canonical paths.
- Support unloaded/loading/ready/failed states at agreed minimum scope.

Minimum output:
- ECS scene renders using registry-loaded assets.

### G5-F04 Minimal Editor

Required behavior:
- Implement hierarchy, selection, inspector, entity create/delete, save/load, dirty state, and small undo/redo.
- Edit core components through ECS/serialization APIs.

Minimum output:
- User can edit the validation scene and persist changes.

### G5-F05 C# Scripting Foundation

Required behavior:
- Host C# runtime behind `engine-script` facade.
- Load script assembly.
- Discover script component types.
- Attach script components to ECS entities.
- Run `OnCreate`, `OnStart`, `OnUpdate`, `OnDestroy` or agreed lifecycle subset.
- Serialize supported fields.
- Report script exceptions safely.

Minimum output:
- Sample script updates an entity and survives save/load.

## Target Effects

- Content, editor, and scripts can work against stable ECS scene data.
- Assets become registry-driven.
- C# is established as the strong-typed scripting layer.

## Explicit Non-Goals

- No hot reload.
- No production editor.
- No mobile/AOT runtime.
- No script debugger or arbitrary reflection.
- No physics, animation, UI, audio, or prefab system.

## AI Execution Rules

- Asset registry is the single source of truth for content references.
- Editor must not edit backend resources directly.
- C# API remains narrow and backend-independent.
- Script exceptions must not crash the engine.

## Completion Signal

Gate 5 is complete when asset cook/registry works, editor scene editing works, C# script components run and serialize, and `AssetRegistry-v0` plus `ScriptAPI-v0` are frozen.
