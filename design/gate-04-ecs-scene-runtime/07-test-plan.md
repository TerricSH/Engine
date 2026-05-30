# Gate 4 Test Plan

## Test Strategy

Gate 4 tests prove that `ECSScene-v0` can represent, validate, save, load, and render the Gate 3 scene through ECS extraction.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G4-F01 Minimal ECS World | Create/delete entities and components | Unit | IDs are valid; stale handles are rejected or guarded |
| G4-F01 Minimal ECS World | Query core components | Unit | Query API returns correct entity/component sets |
| G4-F02 Core Components | Serialize each core component | Unit | Round-trip preserves values |
| G4-F03 ECSScene-v0 Serialization | Full scene round-trip | Integration | Entity count, components, active camera preserved |
| G4-F03 ECSScene-v0 Serialization | Invalid scene validation | Unit | Missing camera/broken data fails before runtime mutation |
| G4-F04 Renderer Extraction | Extract ECS into renderer input | Integration | `RendererInput-v0` matches expected scene data |

## Gate Integration Tests

1. ECS rendering parity test
   - Render Gate 3 static scene.
   - Convert to ECS scene.
   - Render ECS scene and compare visible outcome/statistics.
2. Scene persistence test
   - Save ECS scene.
   - Reload it.
   - Render again and compare entity/component counts and output.
3. Invalid scene test
   - Remove active camera.
   - Break a renderable reference.
   - Confirm diagnostics and no partial world mutation.

## Required Commands

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan`
- `cargo test --workspace --features backend-vulkan`
- `cargo run -p sandbox --features backend-vulkan -- ecs-scene`

## Required Evidence

- Scene round-trip test output.
- ECS extraction test output.
- Screenshot/log showing visual parity with Gate 3 scene.

## Failure Criteria

- Scene save/load loses core component data.
- Renderer extraction imports backend crates.
- Invalid scene mutates runtime world before validation completes.

## Test Fixtures

- `scene_gate04_valid.ron`: one camera, at least two renderables, one light, bounds data, names, and transforms.
- `scene_gate04_missing_camera.ron`: invalid scene with no active camera.
- `scene_gate04_broken_renderable.ron`: invalid renderable reference.
- Expected extraction snapshot for the valid scene.

## Executable Integration Cases

### IT-G4-01 Scene Round Trip

Steps:
1. Load `scene_gate04_valid.ron`.
2. Save it to a temporary output path.
3. Reload saved scene.
4. Compare entity count, component count, active camera, and component values.

Expected:
- Round-trip is lossless for core components.
- Schema version is preserved.

Evidence:
- Round-trip diff report.

### IT-G4-02 Validation Before Mutation

Steps:
1. Attempt to load missing-camera fixture.
2. Attempt to load broken-renderable fixture.
3. Verify runtime world before and after attempt.

Expected:
- Invalid scenes fail with diagnostics.
- Existing runtime world remains unchanged.

Evidence:
- Validation error log.
- World state before/after snapshot.

### IT-G4-03 ECS Renderer Extraction

Steps:
1. Load valid ECS scene.
2. Extract renderer input.
3. Compare output to expected extraction snapshot.
4. Render in sandbox.

Expected:
- Extracted renderer input matches expected data.
- Rendered scene visually matches Gate 3 baseline.

Evidence:
- Extraction JSON.
- Screenshot or render stats comparison.
