# Gate 4 Feature Requirements And Execution Boundaries

## Gate Objective

Create the ECS-backed runtime scene model, scene serialization, validation, and renderer extraction path needed by content, editor, scripting, and later gameplay systems.

## Required Features

### G4-F01 Minimal ECS World

Required behavior:
- Implement entity IDs with stale-handle protection or an equivalent safe identity model.
- Implement component storage for core components.
- Implement query APIs needed by renderer extraction.
- Implement entity create/delete and component add/remove.

Minimum output:
- ECS can represent the Gate 3 static scene.

Do not overbuild:
- No full parallel scheduler.
- No gameplay component library beyond core components.

### G4-F02 Core Components

Required behavior:
- Implement `Name`, `Transform`, `Renderable`, `Camera`, `Light`, and `Bounds`.
- Define validation and serialization shape for each.

Minimum output:
- Core components can be saved, loaded, validated, and extracted for rendering.

### G4-F03 ECSScene-v0 Serialization

Required behavior:
- Define scene schema with version fields.
- Serialize entity records, component records, active camera, and asset references.
- Validate scene before applying to runtime.

Minimum output:
- Scene round-trip preserves core data.
- Invalid scenes produce diagnostics.

### G4-F04 Renderer Extraction

Required behavior:
- Query ECS world and build `RendererInput-v0`.
- Keep extraction backend-independent.
- Preserve render statistics behavior from Gate 3.

Minimum output:
- ECS scene visually matches the pre-ECS static scene.

## Target Effects

- Engine has a persistent runtime scene model.
- Later asset/editor/script systems have stable data to consume.
- Renderer remains isolated from ECS internals.

## Explicit Non-Goals

- No asset registry.
- No editor UI.
- No C# scripting.
- No physics, animation, UI, audio, prefab, or gameplay framework.
- No advanced scene hierarchy beyond what is explicitly needed.

## AI Execution Rules

- Do not serialize runtime handles or backend objects.
- Do not let renderer backend query ECS directly.
- Keep scene schema versioned.
- Validate before mutating runtime world.

## Completion Signal

Gate 4 is complete when ECS scene save/load works, renderer extraction works, and the ECS-rendered validation scene matches the previous static renderer scene.
