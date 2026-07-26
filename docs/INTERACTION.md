# Character and interaction foundation

Playable and AI characters should send movement intent to a scene-authored
`engine.character_controller` instead of writing their Transform:

```csharp
Character.Move(new Vector3(move.X, 0.0f, move.Y), 5.0f);
if (Input.WasPressed("jump"))
    Character.Jump();
```

`Character.Move`, `Jump`, and `Control` may also target an `Entity` or
persistent entity ID. Commands are validated, queued at the script frame
boundary, and consumed by the controller on its next simulation update.
Directions must be horizontal with length no greater than one. An optional
speed must be positive and no greater than 100 m/s.

## Interactable component

Add `engine.interactable` to any collider that should participate in the
project's use convention. Its scene fields are:

- `enabled` (bool, default `true`);
- `prompt` (UTF-8 text, at most 256 bytes, default `Use`);
- `action` (1–64 ASCII letters, digits, `_` or `-`, default `use`);
- `max_distance` (0.1–100 metres, default 3);
- `grabbable` (bool, default `false`).

`Interactable` is available in the editor's Gameplay component category and
creates the validated defaults above. Add a collider to the same entity so a
physics probe can hit it.

The component is available through the generic C# `Components` API. Changes
take effect on the next probe. The action string is data, not an engine rule:
project C# decides what `open`, `talk`, `pickup`, or another action does.

## Deferred use probe

Interaction probes share the bounded physics-query contract. A query issued
in one update is answered in the next frame-local script snapshot:

```csharp
private PhysicsQuery? pendingUse;

public void OnUpdate(float deltaTime)
{
    if (pendingUse is { } query)
    {
        if (Interaction.TryGetTarget(query, out var target))
        {
            UI.ShowPrompt(target.Prompt); // project-owned presentation
            if (Input.WasPressed("use"))
                HandleAction(target.Action, target.Entity);
        }
        pendingUse = null;
    }

    if (pendingUse is null)
        pendingUse = Interaction.Probe(cameraPosition, cameraForward, 4.0f);
}
```

`Probe` excludes the owning entity by default. A hit exposes interaction
metadata only when the component is enabled and the hit distance does not
exceed its authored `max_distance`.

## Grab convention

`Interaction.Grab` accepts a persistent joint ID, a kinematic rigid-body grab
anchor, and a returned `InteractionTarget`. It rejects targets not marked
`grabbable`, then creates the same persistent fixed joint used by
`Physics.Grab`. Move the anchor with the camera/hand and call
`Interaction.ReleaseGrab` with the same joint ID to release it. Break-force
and break-torque thresholds are optional; joint-break events use the normal
physics event channel.

The engine deliberately does not define weapon stats, inventory changes,
door logic, dialogue, or puzzle outcomes. Those remain project C# rules built
on the stable target, physics, damage, UI, and persistence APIs.
