# Gate 13 Feature Requirements And Execution Boundaries

## Gate Objective

Add navigation and AI agents that use cooked navmesh data and drive the Gate 12 character controller instead of bypassing movement rules.

## Required Features

### G13-F01 Navmesh Asset And Cooker

Required behavior:
- Implement navmesh asset type and cooker from level geometry and nav settings.
- Store agent radius/height and cook settings in metadata.

Minimum output:
- Validation level cooks to navmesh asset.

### G13-F02 Runtime Navigation Queries

Required behavior:
- Load navmesh through asset registry.
- Implement find path, nearest point, random point, and walkability queries at agreed scope.

Minimum output:
- Query tests pass on validation navmesh.

### G13-F03 Path Following

Required behavior:
- Implement path corridor/waypoint following and simple smoothing.

Minimum output:
- Agent can follow a path to target.

### G13-F04 AI Agent Component

Required behavior:
- Implement agent state: navmesh reference, radius/height, target, current path, status, stopping distance, speed, controller reference.

Minimum output:
- Agent saves/loads and drives character controller movement.

### G13-F05 Minimal Behavior Runtime

Required behavior:
- Implement either a small FSM or behavior tree runtime for patrol/chase/idle.
- Include deterministic tick semantics and blackboard/local memory rules.

Minimum output:
- Patrol/chase validation scene runs.

### G13-F06 Editor And C# Diagnostics

Required behavior:
- Add navmesh/path/agent debug draw.
- Expose C# APIs to set targets and query current path/state.

Minimum output:
- Editor visualizes paths and C# can command an agent.

## Target Effects

- AI agents navigate using cooked data.
- Agent movement respects character controller, physics, and animation rules.
- Behavior runtime is narrow and testable.

## Explicit Non-Goals

- No crowd simulation.
- No dynamic navmesh updates.
- No traffic lanes or cooperative planning.
- No visual behavior tree editor.
- No direct AI transform teleport movement.

## AI Execution Rules

- Agent dimensions must match controller dimensions.
- Navigation data is cooked, not generated every frame.
- AI movement goes through controller API.
- Behavior runtime must not become an unbounded scripting language.

## Completion Signal

Gate 13 is complete when navmesh cooks/loads, queries work, agents follow paths through controller, behavior test scene runs, and editor/C# diagnostics are usable.
