# Gate 9 Test Plan

## Test Strategy

Gate 9 tests prove that future subsystems can plug into ECS, assets, editor, C#, debug draw, and skinned rendering without central file edits.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G9-F01 Component Extension Registry | Register dummy component | Unit | Component serializes/deserializes and appears in metadata |
| G9-F02 Asset Type Extension Registry | Register dummy asset cooker/loader | Unit | Dummy asset cooks, validates, loads |
| G9-F03 Editor Plugin Surface | Register dummy editor panel | Integration | Panel appears through plugin host |
| G9-F04 Script API Extension Surface | Register dummy C# binding | Integration | Binding is discoverable and callable in test harness |
| G9-F05 Debug Draw Surface | Submit dummy line/shape | Integration | Debug draw reaches renderer input without Vulkan calls |
| G9-F06 RendererInput-v0 skinned items | Dummy skinned producer writes SkinnedItem into RendererInput-v0 and palette/skeleton mismatch test | Unit + Integration | Skinned items reach renderer; bad item dropped with diagnostic per `FD-007` |

## Gate Integration Tests

1. Dummy subsystem full registration
   - One crate registers component, asset, editor, script, debug draw.
   - No core parser/editor/script files need subsystem-specific edits.
2. Deterministic registration test
   - Register extensions in defined order.
   - Confirm stable IDs and dependencies.
3. Debug draw render smoke test
   - Draw dummy debug primitive through renderer path.

## Required Evidence

- Dummy subsystem test logs.
- Extension registry state dump.
- Debug draw screenshot/log.

## Failure Criteria

- Adding a subsystem requires editing central enums.
- Debug draw imports backend internals.
- Editor or script extension bypasses registry contracts.

## Test Fixtures

- `dummy_subsystem` test crate or module.
- Dummy component: `DummyHealth { value: i32 }`.
- Dummy asset type: `DummyAsset` with fake source extension.
- Dummy editor panel and dummy C# binding.
- Dummy debug primitive provider.

## Executable Integration Cases

### IT-G9-01 Dummy Subsystem End-To-End Registration

Steps:
1. Register dummy subsystem descriptor.
2. Register component, asset type, editor panel, script binding, and debug draw provider.
3. Create a scene with dummy component.
4. Serialize and deserialize it.

Expected:
- No core enum/parser edits are required.
- Component round-trips through extension hooks.
- Editor/script/debug registrations are discoverable.

Evidence:
- Registry dump.
- Scene round-trip output.

### IT-G9-02 Registration Ordering And Dependency Errors

Steps:
1. Register dummy subsystem with missing dependency.
2. Register with duplicate component/asset ID.
3. Register in correct dependency order.

Expected:
- Missing dependency and duplicate ID fail clearly.
- Correct order succeeds deterministically.

Evidence:
- Extension validation report.

### IT-G9-03 Debug Draw Backend Independence

Steps:
1. Submit dummy debug line/box.
2. Inspect renderer input debug queue.
3. Search dummy subsystem for backend imports.

Expected:
- Debug primitive reaches renderer input.
- No Vulkan/OpenGL/DX12 imports in dummy subsystem.

Evidence:
- Debug draw capture/log.
