# Gate 13 Session Prompts

Gate 13 adds navigation and AI above the character controller.

## Session 13A: Navmesh Asset Owner

Goal: Implement navmesh asset type, cooker, and runtime loading.

Owns:
- `crates/engine-nav` asset/cook modules

Must not edit:
- character controller internals

Expected output:
- validation level cooks to navmesh asset

Validation:
- navmesh cook/load tests

## Session 13B: Pathfinding And Agent Owner

Goal: Implement path queries, path following, and AI agent component.

Owns:
- navigation query/path/agent modules

Must not edit:
- controller internals
- physics internals

Expected output:
- agents follow paths by issuing controller commands

Validation:
- patrol/chase path scene

## Session 13C: Behavior Runtime And Debug Owner

Goal: Implement minimal FSM/behavior tree and diagnostics.

Owns:
- behavior runtime modules
- nav/agent editor and C# diagnostics

Must not edit:
- visual behavior editor systems

Expected output:
- patrol/chase behavior asset runs
- navmesh/path debug draw

Validation:
- behavior runtime tests
