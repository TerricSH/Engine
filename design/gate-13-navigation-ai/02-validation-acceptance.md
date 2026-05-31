# Gate 13 Validation And Acceptance

## Gate Exit Principle

Gate 13 is accepted only when AI agents can navigate using cooked navmesh data and drive the Gate 12 character controller rather than writing transforms directly.

## Required Results

- Navmesh cooker and navmesh asset type exist.
- Navigation runtime can load navmesh assets through the asset registry.
- Path queries are implemented.
- AI agent component can follow paths through the character controller.
- Minimal behavior runtime exists for patrol/chase/idle scenarios.

## Acceptance Criteria

- [ ] Navmesh cooks from a validation level.
- [ ] `FindPath`, `IsWalkableTo`, `GetNearestPoint`, and `GetRandomPoint` or agreed equivalents work.
- [ ] AI agent follows a path by issuing controller movement requests.
- [ ] Behavior test scene runs patrol and chase behaviors.
- [ ] Agent components save/load with scene.
- [ ] Editor can visualize navmesh, paths, and agent radius.
- [ ] C# can set target position and query current path.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- `cargo test --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- Navmesh cook/load tests.
- Pathfinding query tests.
- Agent scene serialization tests.

## Manual Validation

- Run navigation test scene with multiple agents.
- Verify agents do not teleport or bypass controller movement.
- Visualize navmesh and paths in editor/debug view.
- Run patrol/chase behavior test.

## Blocking Conditions

- Agents write transforms directly for movement.
- Navmesh assets bypass asset registry.
- Behavior runtime requires visual editor features not planned for this gate.
- Agent state cannot save/load.

## Required Evidence

- Navmesh cook output.
- Pathfinding test logs.
- Agent behavior scene video or log.
- C# API sample output.

## Exit Decision

> **Note**: Exit decision fields should be filled by the gate reviewer after
> validation is complete.  The following implementation status is provided
> for reference:
>
> - P0 items: 9/9 resolved (asset pipeline, query API, path smoothing,
>   behavior asset registration).
> - P1 items: agent target recalc, radius/height validation, debug draw
>   wiring — all addressed.
> - Performance measurements: pending (see [`04-performance-report.md`](04-performance-report.md)).

- Gate owner: *TBD — assign before gate exit*
- Date: *TBD*
- Approved to proceed to Gate 14: *pending*

