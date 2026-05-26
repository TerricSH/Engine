# Gate 5 Test Plan

## Test Strategy

Gate 5 tests prove that asset registry, minimal editor, and C# scripting all work against the frozen ECS scene without changing it.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G5-F01 Asset Manifest And Cook Pipeline | Cook validation scene assets | Integration | Cooked artifacts and manifest entries produced |
| G5-F02 Dependency Graph And Validation | Remove referenced texture/material | Unit/Integration | Validator reports dependency chain |
| G5-F03 Runtime Asset Registry | Load scene assets through registry | Integration | Scene renders using registry-loaded handles |
| G5-F04 Minimal Editor | Edit transform/name/light/camera | Manual/Integration | Edits persist after save/load |
| G5-F04 Minimal Editor | Undo/redo basic edit | Unit/Manual | Scene state returns to expected values |
| G5-F05 C# Scripting Foundation | Run sample script lifecycle | Integration | `OnStart`/`OnUpdate` run and log |
| G5-F05 C# Scripting Foundation | Script exception test | Integration | Exception is reported; engine keeps running |
| G5-F05 C# Scripting Foundation | Script field round-trip | Unit | Field values persist through scene save/load |

## Gate Integration Tests

1. Cooked ECS scene test
   - Cook assets.
   - Load ECS scene through registry.
   - Render it.
2. Editor authoring loop
   - Open scene in editor mode.
   - Edit several core components.
   - Save and reload.
3. Script integration test
   - Attach sample C# component.
   - Run lifecycle callbacks.
   - Save/reload script fields.
4. Broken asset and script error test
   - Break asset reference.
   - Throw script exception.
   - Confirm diagnostics.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo run -p sandbox --features backend-vulkan,editor,scripting-csharp -- editor-scene`

## Required Evidence

- Cook/validate command output.
- Editor save/load notes.
- C# sample script logs.
- Error diagnostic examples.

## Failure Criteria

- Editor bypasses ECS/serialization APIs.
- C# exposes backend internals.
- Registry cannot load validation scene.
- Script exception crashes engine process.

## Test Fixtures

- `assets/source/gate05/`: one mesh, one texture, one material, one shader, one ECS scene.
- `assets/source/gate05_broken/`: scene with missing texture and invalid material reference.
- `scripts/csharp/Gate05Sample`: sample component with serialized fields and lifecycle logging.
- Expected cooked manifest and dependency graph snapshot.

## Executable Integration Cases

### IT-G5-01 Cook And Registry Load

Steps:
1. Run cook-only workflow for Gate 5 assets.
2. Run validate-assets.
3. Load ECS scene through registry.
4. Render scene.

Expected:
- Cooked artifacts exist.
- Dependency graph includes scene -> material -> texture/shader relationships.
- Scene renders with registry-loaded assets.

Evidence:
- Cook log.
- Cooked manifest.
- Dependency graph dump.
- Screenshot.

### IT-G5-02 Editor Authoring Loop

Steps:
1. Open validation scene in editor mode.
2. Rename one entity.
3. Move one transform.
4. Change one light value.
5. Save scene.
6. Reload scene.

Expected:
- Edited values persist.
- Dirty flag clears after save.
- Undo/redo restores transform edit during session.

Evidence:
- Editor operation log.
- Scene diff before/after.

### IT-G5-03 C# Script Lifecycle

Steps:
1. Build sample C# project.
2. Attach script component to entity.
3. Run sandbox for several frames.
4. Save and reload script field values.

Expected:
- Lifecycle callbacks execute in documented order.
- Serialized fields persist.
- Intentional exception is reported and engine continues.

Evidence:
- Script log.
- Exception diagnostic log.
- Scene round-trip diff.
