# Gate 4 Session Prompts

Gate 4 establishes `ECSScene-v0`. Do not start asset/editor/script integration until this gate freezes.

## Session 4A: ECS Core Owner

Goal: Implement minimal ECS world and core components.

Owns:
- `crates/engine-scene`
- Core component definitions if placed there

Must not edit:
- Renderer backend crates
- Asset/editor/script crates

Expected output:
- Entity IDs, component storage, query APIs, add/remove behavior
- `Name`, `Transform`, `Renderable`, `Camera`, `Light`, `Bounds`

Validation:
- ECS unit tests
- Stale handle or equivalent safety tests

## Session 4B: Scene Serialization Owner

Goal: Define and implement `ECSScene-v0` persistence.

Owns:
- `crates/engine-serialize`
- Scene schema docs/tests

Must not edit:
- Asset registry schema
- Future prefab schema

Expected output:
- Versioned scene schema
- Core component round-trip
- Validation diagnostics

Validation:
- Scene save/load tests
- Invalid scene diagnostics tests

## Session 4C: Renderer Extraction Owner

Goal: Translate ECS scenes into `RendererInput-v0`.

Owns:
- extraction modules in `engine-core`

Must not edit:
- `render-vulkan` internals

Expected output:
- ECS scene visually matches Gate 3 static scene
- Extraction contains no backend-specific code

Validation:
- ECS scene render test
- Renderer input comparison test
