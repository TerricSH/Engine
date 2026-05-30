# Gate 13: Navigation And AI Foundation

## Purpose

Add navigation, pathfinding, and basic AI agents after the character controller is stable. Agents should drive the controller rather than bypassing physics or transform rules.

## Entry Sync Point

- Gate 12 character controller is stable.
- Scene composition and entity templates are mature enough to define agents.

## Parallel Workstreams

1. Navigation Mesh Assets
   - Owns `crates/engine-nav` and navmesh cookers.
   - Converts level geometry into navmesh assets with walkable areas and metadata.
   - Adds editor/debug visualization for navmesh surfaces.
2. Pathfinding And Queries
   - Adds path queries such as `FindPath`, `IsWalkableTo`, `GetNearestPoint`, and `GetRandomPoint`.
   - Supports simple path smoothing and path corridor following.
3. AI Agent Component
   - Adds navigation agent state, desired target, path following, speed, stopping distance, and movement authority.
   - Integrates with the Gate 12 character controller rather than direct transform teleporting.
4. Behavior Runtime Foundation
   - Adds a minimal behavior tree or finite state machine asset runtime for patrol/chase/idle scenarios.
   - Keeps visual behavior authoring deferred.
5. C# And Editor Support
   - C# APIs for setting target positions, querying paths, and binding behavior assets.
   - Editor visualization for paths, agent radius, and navmesh diagnostics.

## Exit Condition

- Navmesh cooks and loads through the asset registry.
- Agents can patrol and chase using pathfinding and character controller movement.
- Behavior test scene runs with multiple agents without modifying character/physics internals.
- Agent components save/load and expose minimal editor/C# controls.

## Non-Goals

- Crowd simulation, dynamic navmesh updates, cooperative pathfinding, traffic lanes, and visual behavior tree editor.

## Parallel Safety Notes

- Navigation agents never own physics transforms directly.
- Behavior runtime uses public ECS/script APIs.
- Navmesh assets register through asset extension surfaces.
