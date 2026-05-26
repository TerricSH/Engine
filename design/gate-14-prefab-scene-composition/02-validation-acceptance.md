# Gate 14 Validation And Acceptance

## Gate Exit Principle

Gate 14 is accepted only when prefabs can represent reusable gameplay composition without breaking base scene serialization.

## Required Results

- Prefab asset schema exists and is versioned.
- Prefab instances and overrides work.
- Editor can create, instantiate, apply, revert, and inspect prefabs.
- Gameplay archetypes exist for common patterns.
- Object pooling and lifecycle manager work for repeated spawn/despawn flows.

## Acceptance Criteria

- [ ] Prefab instance saves and loads with override data intact.
- [ ] Nested prefab references resolve correctly.
- [ ] Missing prefab source is reported clearly.
- [ ] Editor can apply and revert overrides.
- [ ] Gameplay test scene uses archetypes instead of hand-built repeated entities.
- [ ] Object pool reduces allocations or entity churn in a repeated spawn/despawn test.
- [ ] Prefab schema extends scene composition without breaking `ECSScene-v0`.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp`
- Prefab save/load round-trip tests.
- Override diff/apply/revert tests.
- Pool lifecycle tests.

## Manual Validation

- Create a prefab in editor, instantiate it, edit overrides, save, reload, and verify state.
- Build a small gameplay scene from archetypes.
- Exercise object pooling with repeated projectile/effect spawns.

## Blocking Conditions

- Prefab schema changes base scene schema in a breaking way.
- Override data is lost during save/load.
- Pooling bypasses ECS lifecycle rules.
- Editor cannot detect broken prefab references.

## Required Evidence

- Prefab schema example.
- Editor prefab workflow notes or capture.
- Pooling test output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 15: yes/no

