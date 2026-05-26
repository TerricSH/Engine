# Gate 14 Test Plan

## Test Strategy

Gate 14 tests prove prefabs, overrides, archetypes, and pooling work across serialization, editor, scripts, and ECS lifecycle.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G14-F01 Prefab Asset Schema | Parse/validate prefab asset | Unit | Valid prefab accepted, invalid rejected |
| G14-F02 Prefab Instantiation | Instantiate prefab into ECS | Integration | Entities/components created with source link |
| G14-F03 Override System | Apply/revert property override | Unit/Integration | Override persists and can revert |
| G14-F04 Nested Prefabs And Variants | Nested and cyclic reference test | Unit | Valid nested prefab works; cycle rejected |
| G14-F05 Editor Prefab Workflow | Create/instantiate/apply/revert in editor | Manual/Integration | Full authoring loop works |
| G14-F06 Gameplay Archetypes | Build validation scene from archetypes | Integration | Archetypes produce expected entities |
| G14-F07 Object Pooling | Spawn/despawn pool stress | Integration | Entities reset and reuse safely |

## Gate Integration Tests

1. Prefab authoring loop scene.
2. Prefab save/load with overrides.
3. Nested prefab validation.
4. Object pooling with scripts/physics/audio state reset if present.

## Failure Criteria

- Override data is lost.
- Pooling bypasses ECS lifecycle.
- Prefab schema breaks base scene schema.

## Test Fixtures

- `prefabs/gate14/base_enemy.prefab`: base entity with renderable, collider, script.
- `prefabs/gate14/fast_enemy.prefab`: variant overriding speed/material/script fields.
- `scenes/gate14_prefab_authoring.scene`: editor workflow scene.
- `scenes/gate14_pooling.scene`: projectile or effect pooling scene.

## Executable Integration Cases

### IT-G14-01 Prefab Instance Override Round Trip

Steps:
1. Load base prefab.
2. Instantiate it into a scene.
3. Override one transform, one material, and one script field.
4. Save and reload the scene.
5. Inspect override diff.

Expected:
- Source prefab link is preserved.
- Overrides persist and are visible.
- Revert restores base values.

Evidence:
- Prefab diff report.
- Scene round-trip file.

### IT-G14-02 Nested Variant Validation

Steps:
1. Load variant prefab.
2. Resolve base prefab chain.
3. Load a negative fixture with cyclic prefab reference.

Expected:
- Valid variant resolves.
- Cyclic reference is rejected with clear diagnostics.

Evidence:
- Prefab validation log.

### IT-G14-03 Pooling Lifecycle

Steps:
1. Spawn pooled projectiles repeatedly.
2. Return them to pool.
3. Spawn again.
4. Inspect script/physics/animation/audio reset state where applicable.

Expected:
- Reused entities do not retain stale state.
- ECS lifecycle hooks run in documented order.

Evidence:
- Pool lifecycle trace.
