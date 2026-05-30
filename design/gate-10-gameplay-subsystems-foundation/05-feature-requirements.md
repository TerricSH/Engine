# Gate 10 Feature Requirements And Execution Boundaries

## Gate Objective

Implement first usable physics and animation foundations through Gate 9 extension surfaces while keeping both subsystems isolated and backend-independent above their adapter layers.

## Required Features

### G10-F01 Physics Components

Required behavior:
- Implement `RigidBody`, `Collider`, `PhysicsMaterial` references, collision layers/masks, and body type data.

Minimum output:
- Components save/load and appear in editor metadata.

### G10-F02 Physics World And Fixed Step

Required behavior:
- Implement physics world resource.
- Run simulation on fixed timestep.
- Queue mutations to simulation boundaries.

Minimum output:
- Rigid bodies fall under gravity in validation scene.

### G10-F03 Physics Backend Adapter

Required behavior:
- Implement first backend adapter, recommended Rapier 3D.
- Hide backend body/collider handles from ECS and serialization.

Minimum output:
- Backend can create bodies/colliders from ECS data.

### G10-F04 Physics Queries And Events

Required behavior:
- Implement raycast, overlap, sweep, collision enter/stay/exit, and trigger events.
- Expose query snapshots and event stream after physics step.

Minimum output:
- Query and event tests pass.

### G10-F05 Animation Assets And Components

Required behavior:
- Implement skeleton asset, animation clip asset, `Skeleton`, `AnimationPlayer`, and optional `SkinnedMeshBinding`.

Minimum output:
- Skeleton/clip assets load through asset registry.

### G10-F06 Animation Evaluation

Required behavior:
- Implement single-clip playback, looping, speed, pause/resume, current time, local pose interpolation, hierarchy solve, and bone palette extraction.

Minimum output:
- Animated skeletal mesh renders through `RendererInput-v0.skinned_items` (per `FD-007`).

### G10-F07 Editor And C# Exposure

Required behavior:
- Add basic editor fields and C# APIs for physics and animation.
- Add debug draw for colliders and skeletons.

Minimum output:
- C# can receive collision callbacks and trigger animation playback.

## Target Effects

- Physics and animation become usable independent engine subsystems.
- Both can save/load, render debug data, and expose safe C# APIs.

## Explicit Non-Goals

- No joints, character controller, vehicles, ragdoll, cloth, fluids, networking determinism.
- No blending, animation state machines, IK, retargeting, morph targets, root motion, animation timeline editor.
- No custom physics solver.

## AI Execution Rules

- Physics and animation own separate crates.
- Backend handles are never serialized.
- C# must not mutate physics mid-step.
- Animation feeds renderer through `RendererInput-v0.skinned_items` (per `FD-007`).
- Debug draw uses Gate 9 surface.

## Completion Signal

Gate 10 is complete when physics and animation validation scenes run, save/load works, editor fields exist, C# APIs work, and both subsystems remain isolated.
