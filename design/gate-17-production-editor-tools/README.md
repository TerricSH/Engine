# Gate 17: Production Editor Tools

## Purpose

Move the editor from minimal scene editing to practical production authoring. Editor features should consume plugin surfaces from earlier gates instead of hardcoding subsystem internals.

## Entry Sync Point

- Minimal editor is stable.
- Prefab system is stable.
- Physics, animation, UI, and audio foundations are stable.

## Parallel Workstreams

1. Advanced Transform And Debug Gizmos
   - Adds move/rotate/scale gizmos, snapping, pivot modes, collider editing, light gizmos, skeleton/bone debug views, and navmesh path debug views.
2. Asset Browser And Assignment
   - Adds searchable asset browser, drag/drop assignment, dependency view, missing reference repair, and preview thumbnails where practical.
3. Material And Shader Tools
   - Adds material editing UI, shader parameter inspection, preview mesh, and hot reload diagnostics.
4. Animation Preview Tools
   - Adds animation preview panel, timeline scrubber, blend/state visualization, and event/notifies display.
5. Prefab And Scene Composition Tools
   - Adds override diff view, apply/revert UI, variant tree, and validation reports.
6. Performance Inspector Panel
   - Adds frame stats, draw calls, memory summaries, asset counts, physics bodies, animation count, AI agents, and hot reload errors.

## Exit Condition

- Editor supports practical scene authoring with assets, prefabs, material edits, animation preview, and debug gizmos.
- Editor tools consume plugin surfaces rather than hardcoding every subsystem into core editor code.
- Existing scenes remain loadable.

## Non-Goals

- Multiplayer collaborative editing, full visual scripting editor, node-based shader authoring if too large, and production-grade profiler flamegraph UI.

## Parallel Safety Notes

- Each subsystem provides its own editor plugin/panel.
- Core editor owns layout and panel hosting, not every subsystem detail.
- Editor tools should not mutate asset, ECS, or script schemas directly.
