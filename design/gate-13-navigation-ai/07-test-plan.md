# Gate 13 Test Plan

## Test Strategy

Gate 13 tests prove navmesh assets, path queries, AI agents, and behavior runtime can drive character controllers safely.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G13-F01 Navmesh Asset And Cooker | Cook validation level | Integration | Navmesh asset produced and registered |
| G13-F02 Runtime Navigation Queries | Find path/nearest/random/walkability | Unit/Integration | Query results match expected geometry |
| G13-F03 Path Following | Follow waypoint corridor | Integration | Agent reaches target within tolerance |
| G13-F04 AI Agent Component | Save/load agent state | Unit | Agent data persists |
| G13-F04 AI Agent Component | Agent drives controller | Integration | Agent uses controller movement API |
| G13-F05 Minimal Behavior Runtime | Patrol/chase behavior asset | Integration | Behavior ticks deterministically |
| G13-F06 Editor And C# Diagnostics | Path debug and C# set target | Manual/Integration | Debug draw and C# API work |

## Gate Integration Tests

1. Navmesh cook/load test
   - Cook level geometry and load navmesh through registry.
2. AI patrol/chase scene
   - Multiple agents follow paths and drive controllers.
3. Behavior runtime test
   - Tick behavior assets and verify state transitions.
4. Debug visualization test
   - Show navmesh, path, target, and agent radius.

## Required Evidence

- Navmesh cook output.
- Agent movement capture/log.
- Behavior tick logs.
- Debug draw screenshot/log.

## Failure Criteria

- AI writes transforms directly.
- Navmesh bypasses asset registry.
- Behavior runtime becomes broad scripting.
- Agent dimensions mismatch controller dimensions without validation.

## Test Fixtures

- `levels/gate13_nav_level.glb` or equivalent source geometry.
- `nav/gate13_settings.ron`: agent radius/height and walkable slope settings.
- `scenes/gate13_patrol.scene`: static navmesh, patrol route, one agent.
- `scenes/gate13_chase.scene`: target entity and chasing agent.
- `logic/gate13_patrol.behavior` and `logic/gate13_chase.behavior`.

## Executable Integration Cases

### IT-G13-01 Navmesh Cook And Query

Steps:
1. Cook navmesh from source geometry and settings.
2. Load cooked navmesh through asset registry.
3. Run nearest point, random point, walkability, and path queries.

Expected:
- Navmesh asset registers in asset registry.
- Queries return deterministic, valid points/paths for fixture geometry.

Evidence:
- Cook log.
- Query result JSON.
- Navmesh debug capture.

### IT-G13-02 Agent Patrol Through Controller

Steps:
1. Load patrol scene.
2. Run behavior runtime for fixed ticks.
3. Confirm agent requests movement from character controller.
4. Record path and controller state.

Expected:
- Agent follows patrol route.
- No direct transform writes from AI.
- Controller handles movement/animation.

Evidence:
- Agent movement trace.
- Path debug capture.

### IT-G13-03 Behavior Runtime Narrowness

Steps:
1. Load patrol/chase behavior assets.
2. Validate node/state schemas.
3. Run deterministic ticks.

Expected:
- Behavior runtime supports only scoped nodes/states.
- Invalid behavior assets fail schema validation.

Evidence:
- Behavior validation report.
