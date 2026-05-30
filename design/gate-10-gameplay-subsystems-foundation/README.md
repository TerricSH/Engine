# Gate 10: Gameplay Subsystems Foundation

## Purpose

Land the first foundation versions of large gameplay-facing subsystems after extension surfaces are stable. Physics and animation can run in parallel because they own separate crates and use shared extension APIs.

## Entry Sync Point

- `SubsystemExtension-v0` is frozen.

## Parallel Workstreams

1. Physics Foundation
   - Owns `crates/engine-physics` and one initial backend crate.
   - Recommended first backend: Rapier 3D for Rust-native landing speed, with backend traits kept open for Jolt later.
   - Adds `RigidBody`, `Collider`, `PhysicsMaterial`, collision layers/masks, fixed timestep, gravity, basic forces, raycast, overlap, sweep, and collision events.
   - Editor support: collider wireframes and simple inspector fields.
   - C# support: collision callbacks and safe query APIs.
2. Animation Foundation
   - Owns `crates/engine-animation` and animation cooker additions through the asset extension surface.
   - Adds `Skeleton`, `AnimationClip`, `AnimationPlayer`, single-clip playback, linear keyframe interpolation, looping, playback speed, and bone palette extraction.
   - Renderer integration uses `RendererInput-v0.skinned_items` (per `FD-007`).
   - Editor support: assign animation asset, play/pause/preview, simple time scrub if editor timing is ready.
   - C# support: play/stop/is-playing/current-time APIs.
3. Optional UI Foundation
   - Can run in parallel only if UI uses separate crates and does not compete with editor UI internals.
4. Optional Audio Foundation
   - Can run in parallel if asset/audio crates are isolated.

## Exit Condition

- Physics test scene simulates rigid bodies, queries, and collision events.
- Animation test scene plays a skeletal clip and feeds skinned mesh data through renderer input.
- Both systems save/load through scene serialization, expose minimal editor fields, and provide safe C# APIs.
- Physics and animation branches do not modify each other's crates or backend internals.

## Non-Goals

- Physics: joints, character controller, vehicles, ragdoll, soft bodies, cloth, fluids, networking determinism.
- Animation: blending, state machines, IK, retargeting, morph targets, animation timeline editor, root motion, ragdolls.

## Parallel Safety Notes

- Physics owns physics crates.
- Animation owns animation crates.
- Shared fields require a Gate 9 extension update before subsystem branches continue.
