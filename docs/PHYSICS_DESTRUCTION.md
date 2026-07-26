# Destructible props and damage

The gameplay feature set registers the scene component
`engine.physics.destructible`. It provides engine-level health accumulation and
one-shot prefab replacement while leaving weapon rules, damage values, teams,
armor, and scoring in project C#.

## Scene component

The component uses schema `0.1.0` and these fields:

| Field | Default | Meaning |
|---|---:|---|
| `enabled` | `true` | Ignore damage while disabled. |
| `max_health` | `100` | Positive authored health ceiling. |
| `health` | `100` | Current health, persisted in scenes and checkpoints. |
| `minimum_damage` | `0` | Ignore individual hits below this raw amount. |
| `damage_scale` | `1` | Multiplier applied after the minimum-damage test. |
| `replacement_prefab` | none | Optional cooked prefab spawned when health reaches zero. |
| `destroy_on_break` | `true` | Remove the original entity after replacement succeeds. |
| `inherit_velocity` | `true` | Copy the original rigid body's linear and angular velocity to rigid replacement pieces. |
| `fracture_impulse_scale` | `1` | Scale and distribute the hit impulse across rigid replacement pieces. |
| `broken` | `false` | Persisted one-shot runtime state; a broken object rejects later hits. |

Numeric fields are sanitized by scene deserialization and validated again by
the native damage path. `replacement_prefab` participates in normal asset
dependency validation. The editor can author the registered component through
the Gameplay component category using a validated default field map. Joint and
ragdoll components remain intentionally hidden from generic “Add Component”
until their required multi-entity constraint/fitting tools exist.

## C# API

```csharp
// A raycast or weapon script decides the amount and damage kind.
Damage.Apply(
    hit.Entity,
    25.0f,
    DamageKind.Impact,
    hit.Point,
    forward * 12.0f);

foreach (var damage in Damage.Events)
{
    if (damage.Broke)
        Console.WriteLine(
            $"{damage.TargetEntityId} spawned " +
            $"{damage.SpawnedEntityIds.Count} fracture entities");
}
```

`Damage.Apply` also accepts a persistent entity ID. Amounts and vectors are
validated in managed code and again by the native command bridge. A command
drain accepts at most 256 requests; an amount is positive and no greater than
1,000,000, and position/impulse components use the physics mutation bound.

Accepted hits subtract scaled health at the script frame boundary. Results are
delivered through `Damage.Events` in the next script snapshot to the target
and to the issuing script entity when they differ. Each event exposes raw and
applied damage, remaining health, damage kind, hit point, impulse, break state,
and spawned persistent IDs.

## Break transaction

When health reaches zero:

1. the component is marked broken exactly once;
2. the optional replacement prefab is instantiated at the original
   Transform translation;
3. every rigid replacement piece inherits the old rigid body's velocity when
   enabled and receives its share of the scaled hit impulse;
4. the original entity is removed only after replacement succeeds.

If the replacement asset is missing or its prefab graph cannot be
instantiated, the original entity remains present and an actionable script
diagnostic is emitted. Its broken state remains visible for debugging and
checkpoint consistency.

This is replacement-based destruction, not runtime mesh cutting. Automatic
collision-impulse-to-damage rules, material-specific resistance, geometric
fracture generation, and fracture authoring previews remain future systems.
