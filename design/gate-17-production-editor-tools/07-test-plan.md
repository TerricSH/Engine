# Gate 17 Test Plan

## Test Strategy

Gate 17 tests prove the editor can support production authoring through plugin surfaces and command-based mutation.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G17-F01 Editor Panel Host | Persist and restore panel layout | Manual/Integration | Layout restores |
| G17-F02 Command-Based Editing | Undo/redo transform and asset edits | Integration | Scene state returns to expected values |
| G17-F03 Transform And Debug Gizmos | Move/rotate/scale entity | Manual/Integration | Operation is undoable |
| G17-F04 Asset Browser | Search and assign asset | Manual/Integration | Assignment uses registry metadata |
| G17-F05 Material And Shader Tools | Edit material parameter | Manual/Integration | Preview updates and persists |
| G17-F06 Animation And Prefab Tools | Preview animation and apply prefab override | Manual/Integration | Tools operate through plugin APIs |
| G17-F07 Performance Inspector | Display runtime/editor stats | Runtime | Stats update while editor runs |

## Gate Integration Tests

1. Build a production-style scene using editor tools only.
2. Save/reload scene and verify no data loss.
3. Use at least one tool from each subsystem plugin.

## Failure Criteria

- Core editor hardcodes subsystem internals.
- Edits bypass command history.
- Asset browser bypasses registry.

## Test Fixtures

> **Fixture implementation status: Pending.** The fixtures below are referenced by integration and manual test cases. As of the current gate review, the following items do not yet exist on disk and must be created during the integration test implementation phase.

- `scenes/gate17_editor_workflow.scene`: mixed scene with renderables, prefab, animation, material, physics object. *(not yet created)*
- `assets/gate17/`: material, texture, animation, prefab fixtures. *(not yet created)*
- Deterministic editor action script for create/edit/undo/save/reload operations. *(not yet created)*

## Executable Integration Cases

> **Implementation status: Pending.** The integration tests IT-G17-01/02/03 are not yet implemented. They are described below as the target specification; the runtime automation and fixtures must be created before gate exit.

### IT-G17-01 Command And Undo Workflow

Steps:
1. Load editor workflow scene.
2. Move entity with gizmo.
3. Assign material through asset browser.
4. Apply prefab override.
5. Undo all operations.
6. Redo all operations.

Expected:
- Scene state matches expected snapshots after undo and redo.
- Every mutation has a command entry.

Evidence:
- Command history log.
- Scene state snapshots.

### IT-G17-02 Plugin Panel Coverage

Steps:
1. Open physics, animation, prefab, material, and performance panels.
2. Confirm each panel reads data through plugin/public APIs.

Expected:
- Core editor does not import subsystem internals directly.
- Panels show valid data.

Evidence:
- Plugin registry dump.
- Panel screenshot or capture.

### IT-G17-03 Asset Browser Registry Use

Steps:
1. Search asset by name/type.
2. Assign asset to compatible field.
3. Break asset reference and repair through browser.

Expected:
- Search uses registry metadata.
- Broken reference diagnostics appear.

Evidence:
- Asset assignment log.
