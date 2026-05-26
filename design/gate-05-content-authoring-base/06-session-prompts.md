# Gate 5 Session Prompts

Gate 5 can run multiple sessions in parallel after `ECSScene-v0` is frozen. Sessions must consume ECS scene data without changing it.

## Session 5A: Asset Pipeline Owner

Goal: Implement asset manifest, cook pipeline, dependency graph, registry, and validation.

Owns:
- `crates/engine-asset`
- asset docs/tests

Must not edit:
- editor internals
- C# host internals
- renderer backend internals

Expected output:
- `AssetRegistry-v0`
- cook-only and validate-assets workflows
- validation scene assets load through registry

Validation:
- asset cook/load tests
- broken dependency diagnostics

## Session 5B: Minimal Editor Owner

Goal: Implement ECS scene authoring basics.

Owns:
- `crates/engine-editor`

Must not edit:
- ECS schema
- asset registry schema
- renderer backend internals

Expected output:
- hierarchy, selection, inspector, create/delete, save/load, small undo/redo

Validation:
- edit scene, save, reload, compare results

## Session 5C: C# Scripting Owner

Goal: Implement strong-typed script component foundation.

Owns:
- `crates/engine-script`
- `scripts/csharp`

Must not edit:
- renderer backend internals
- asset manifest schema unless coordinated

Expected output:
- C# runtime host facade
- assembly load/type discovery
- script component lifecycle
- serialized fields
- safe exception diagnostics

Validation:
- sample script runs lifecycle callbacks
- script field round-trip
- exception handling test
