# Gate 10 Test Plan

## Test Strategy

Gate 10 tests prove physics and animation foundations work independently and together, while using Gate 9 extension surfaces.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G10-F01 Physics Components | Serialize rigid body/collider/material refs | Unit | Round-trip preserves data |
| G10-F02 Physics World And Fixed Step | Gravity simulation | Integration | Dynamic body falls deterministically enough for tolerance |
| G10-F03 Physics Backend Adapter | Create backend bodies from ECS | Integration | Backend handles remain internal |
| G10-F04 Physics Queries And Events | Raycast/overlap/sweep | Unit/Integration | Correct entities returned |
| G10-F04 Physics Queries And Events | Collision/trigger event order | Integration | Enter/stay/exit events emitted correctly |
| G10-F05 Animation Assets And Components | Load skeleton/clip assets | Unit | Compatibility validation works |
| G10-F06 Animation Evaluation | Single clip playback | Integration | Bone palette updates over time |
| G10-F07 Editor And C# Exposure | C# collision + animation sample | Integration | Callbacks and playback work safely |

## Gate Integration Tests

1. Physics validation scene
   - Static ground, falling bodies, colliders, queries.
2. Animation validation scene
   - Skinned mesh plays one clip and renders.
3. Combined scene
   - Physics and animation run in the same ECS world without editing each other's crates.
4. Serialization scene
   - Save/load physics and animation components.

## Required Evidence

- Physics scene log/video.
- Animation scene capture.
- C# callback logs.
- Scene round-trip test output.

## Failure Criteria

- Backend handles serialize into scene data.
- C# mutates physics mid-step.
- Animation writes directly to backend buffers.
- Physics and animation require changes in each other's crates.

## Test Fixtures

- `scenes/gate10_physics.scene`: static ground, falling boxes/spheres, trigger volume, raycast target.
- `scenes/gate10_animation.scene`: skinned mesh, skeleton asset, single animation clip.
- `scenes/gate10_combined.scene`: physics objects and animated character in same ECS world.
- `scripts/csharp/Gate10PhysicsAnimationSample`.
- `assets/gate10/skeleton`, `assets/gate10/walk_clip`, physics material fixtures.

## Executable Integration Cases

### IT-G10-01 Physics World Step

Steps:
1. Load physics scene.
2. Run fixed timestep for 120 simulation steps.
3. Query final transforms and event log.

Expected:
- Dynamic bodies fall and settle within tolerance.
- Trigger/collision event order is enter -> stay -> exit where applicable.
- Raycast/overlap/sweep results match expected target entities.

Evidence:
- Physics state JSON.
- Event log.

### IT-G10-02 Animation Playback

Steps:
1. Load animation scene.
2. Play one clip for a fixed number of frames.
3. Capture bone palette snapshots at frame 0, middle, and end.
4. Render skinned mesh.

Expected:
- Bone palette changes over time.
- Skeleton/clip compatibility passes.
- Skinned mesh renders through `RendererInput-v0.skinned_items` (per `FD-007`).

Evidence:
- Bone palette snapshot file.
- Render capture.

### IT-G10-03 Combined Runtime Isolation

Steps:
1. Load combined scene.
2. Run physics and animation together.
3. Save and reload scene.
4. Trigger C# collision callback and animation playback command.

Expected:
- Physics and animation run without editing each other's crates.
- Backend handles are absent from serialized scene.
- C# callbacks execute after safe simulation points.

Evidence:
- Combined scene log.
- Serialized scene inspection report.
