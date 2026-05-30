# Gate 17 Session Prompts

Gate 17 upgrades editor production workflows. All mutations should go through commands.

## Session 17A: Editor Shell And Commands Owner

Goal: Implement panel host, layout persistence, command history, undo/redo baseline.

Owns:
- core editor shell modules

Must not edit:
- subsystem internals

Expected output:
- stable panel layout and command-based mutation

Validation:
- command history tests

## Session 17B: Asset Material Prefab Tools Owner

Goal: Implement asset browser, material/shader tools, prefab override tooling.

Owns:
- relevant editor plugins/panels

Must not edit:
- asset registry schema
- prefab schema except through owner

Expected output:
- asset assignment, material preview, prefab diff/apply/revert

Validation:
- production editor workflow test

## Session 17C: Gizmos Animation Performance Owner

Goal: Implement transform/debug gizmos, animation preview, performance inspector.

Owns:
- gizmo tools
- animation preview panel
- performance inspector panel

Must not edit:
- renderer backend internals

Expected output:
- undoable gizmo transforms and diagnostic panels

Validation:
- editor interaction tests
