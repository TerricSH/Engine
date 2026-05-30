# Gate 17 Feature Requirements And Execution Boundaries

## Gate Objective

Upgrade the editor from minimal scene editing to practical production tooling using plugin surfaces, command-based mutation, and subsystem-owned panels.

## Required Features

### G17-F01 Editor Panel Host

Required behavior:
- Implement or refine dock/panel hosting at agreed scope.
- Persist basic layout state.

Minimum output:
- Editor opens with stable panel layout and can restore it.

### G17-F02 Command-Based Editing

Required behavior:
- Ensure scene/tool mutations go through commands with apply/revert behavior.
- Integrate undo/redo with transform and asset/prefab edits.

Minimum output:
- Common editor operations are undoable.

### G17-F03 Transform And Debug Gizmos

Required behavior:
- Add move/rotate/scale gizmos, snapping/pivot basics, and subsystem debug overlays.

Minimum output:
- Selected entity can be transformed through gizmo with undo.

### G17-F04 Asset Browser

Required behavior:
- Search asset registry metadata.
- Drag/drop or assign assets to compatible fields.
- Show missing reference diagnostics.

Minimum output:
- Asset assignment workflow works through registry.

### G17-F05 Material And Shader Tools

Required behavior:
- Edit material parameters at agreed scope.
- Preview material changes.
- Show shader/hot reload diagnostics.

Minimum output:
- Material edit updates preview or validation scene.

### G17-F06 Animation And Prefab Tools

Required behavior:
- Preview animation clips and scrub timeline at agreed scope.
- Show prefab override diff/apply/revert UI.

Minimum output:
- Animation preview and prefab override workflow are usable.

### G17-F07 Performance Inspector

Required behavior:
- Show frame stats, draw calls, memory summary, asset count, physics bodies, animation count, AI agents, and hot reload errors.

Minimum output:
- Inspector updates during editor session.

## Target Effects

- Editor can author production-style scenes with assets, prefabs, materials, animation preview, and debug tools.
- Subsystem tools plug in without core editor hardcoding.

## Explicit Non-Goals

- No multiplayer collaborative editing.
- No full visual scripting editor.
- No full node-based shader authoring unless explicitly scoped.
- No production flamegraph profiler UI.

## AI Execution Rules

- Core editor hosts panels; subsystem plugins own details.
- All mutations go through commands.
- Asset browser consumes registry metadata.
- Gizmos emit undoable commands.
- Do not mutate schemas directly from editor tools.

## Completion Signal

Gate 17 is complete when production editor workflows can build/edit a scene using plugin panels, asset browser, gizmos, prefab tools, and diagnostics without bypassing core APIs.
