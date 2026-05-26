# Gate 10 Validation And Acceptance

## Gate Exit Principle

Gate 10 is accepted only when physics and animation foundations are usable together in the engine without modifying each other's crates or renderer backend internals.

## Required Results

- Physics foundation supports `RigidBody`, `Collider`, `PhysicsMaterial`, layers/masks, fixed timestep, gravity, forces, raycast, overlap, sweep, collision events, and trigger events.
- Animation foundation supports skeleton assets, animation clips, `AnimationPlayer`, single-clip playback, interpolation, looping, speed, pause/resume, current time, and bone palette extraction.
- Physics and animation components save/load through scene serialization.
- Minimal editor fields and C# APIs exist for both systems.
- Debug draw uses Gate 9 debug draw surface.

## Acceptance Criteria

- [ ] Physics test scene shows rigid bodies falling, colliding, and settling.
- [ ] Raycast, overlap, and sweep queries return expected entities.
- [ ] Collision enter/stay/exit and trigger events fire in expected order.
- [ ] Animation test scene plays a skeletal clip.
- [ ] Bone palette data reaches `RendererInput-v0.skinned_items` (per `FD-007`).
- [ ] C# can receive collision callbacks and trigger animation playback.
- [ ] Editor can view/edit basic physics and animation fields.
- [ ] Physics and animation use separate validation scenes and do not edit each other's crates.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,editor,scripting-csharp`
- `cargo test --workspace --features backend-vulkan,editor,scripting-csharp`
- Physics query and event tests.
- Animation asset load and playback tests.
- Scene round-trip tests for physics/animation components.

## Manual Validation

- Run physics test scene and observe stable body behavior.
- Run animation test scene and observe correct skeletal playback.
- Enable debug draw for colliders and skeletons.
- Trigger C# collision and animation APIs in sample scripts.

## Blocking Conditions

- Physics debug draw calls Vulkan directly.
- Animation modifies renderer backend internals instead of using skinned input.
- Physics or animation components cannot save/load.
- C# callbacks can crash the engine.

## Required Evidence

- Physics and animation test scene outputs.
- C# callback logs.
- Debug draw screenshot.
- Test command outputs.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 11: yes/no

