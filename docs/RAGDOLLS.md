# Ragdolls

The `gameplay` + `runtime-subsystems` feature set supports scene-authored
skeletal ragdolls. A ragdoll keeps the skinned mesh on its owner entity and
generates a deterministic internal rigid-body/joint graph from
`engine.ragdoll`.

## Authoring contract

`RagdollComponent` contains:

- a bounded list of body definitions keyed by skeleton bone name;
- ball, capsule, or box collider geometry plus local offset/rotation, mass,
  and damping;
- a bounded list of fixed, revolute, or spherical constraints between those
  bodies, with anchors, axis, optional limits, and break thresholds;
- the current `Animated`, `Simulated`, or `Recovering` ownership mode;
- recovery timing and deterministic generated body/joint IDs.

Body names must be unique and must exist in the entity's loaded
`SkeletonComponent` asset. Constraints must connect two authored bodies and a
child body can have only one parent constraint. The registered component has
editor metadata and explicit scene fields; invalid dimensions, rotations,
limits, timing, or missing bones are rejected before graph generation.

Generated IDs use:

```text
<owner>.__ragdoll.body.<index>
<owner>.__ragdoll.joint.<index>
```

Generated entities carry the internal `engine.ragdoll_part` ownership marker.
The frame reconciler removes orphaned parts, refuses ID conflicts, repairs
authored joint changes incrementally, and never exposes physics backend
handles.

## Ownership and pose flow

In `Animated` mode, generated bodies are kinematic and follow the previous
evaluated bone pose. `Activate` switches them to dynamic bodies without
rebuilding their constraints and optionally distributes an impulse across
them. After physics, body transforms are converted back to owner-local bone
transforms; this external pose is applied after animation layers and IK and
therefore owns the final GPU skinning palette.

`Recover` switches bodies to kinematic ownership and fades the last physical
pose into the currently evaluated animation over a bounded duration.
`SnapToAnimation` is the zero-duration form. Once the blend finishes, the
external pose override is removed.

```csharp
// Death or knockdown.
Ragdoll.Activate(npc, shotDirection * 18.0f);

// Later, blend back to the project's get-up animation.
Ragdoll.Recover(npc, 0.45f);

foreach (var change in Ragdoll.Events)
    Console.WriteLine($"{change.EntityId}: {change.BodyEntityIds.Count} bodies");
```

The native equivalent is
`GameLoop::set_ragdoll_active(entityId, active, recoveryDuration, impulse)`.
Managed commands validate persistent IDs, recovery duration (0–30 seconds),
finite bounded impulses, target existence, component availability, and a
per-frame request budget.

## Persistence

The owner component, generated part ownership, rigid bodies, persistent
joints, current mode, recovery progress, and a not-yet-applied activation
impulse are scene/checkpoint data. Existing save-game rigid-body snapshots
preserve each generated body's pose, velocity, and sleep state by persistent
ID. Restore rebuilds backend bodies and joints without serializing Rapier
handles.

## Remaining production work

The current system expects explicit body and constraint definitions. Automatic
capsule fitting from a humanoid skeleton, adjacent-body collision exclusion,
joint-drive physical animation, editor gizmos/limit previews, and
root/locomotion alignment for authored get-up clips remain future work.
