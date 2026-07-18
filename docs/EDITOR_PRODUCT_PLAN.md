# Cross-platform Editor Product Plan

## Product outcome

The editor is complete only when a user can create, author, debug, build, and
package a game project without editing scene or project files by hand. Visual
similarity to Unity is not an acceptance criterion by itself; every visible
control must execute a real editor command or open a complete workflow.

The editor is cross-platform and uses one production UI and render path:

```text
Tao/Wry native host
  -> React docked editor shell
  -> typed, versioned IPC
  -> editor commands / project services
  -> EngineRuntime
  -> Renderer
  -> SceneRenderer
```

`engine-ui` remains the runtime game Canvas system. It is not an editor widget
toolkit. React leaves the active Scene/Game slot transparent so the one native
renderer remains visible below the WebView; it never introduces a WebGL or
off-screen mirror renderer. The former `EditorUi` and egui implementations,
their input replay systems, and all legacy panel draw functions are deleted
rather than retained as fallbacks.

## Non-negotiable architecture rules

1. There is exactly one editor input, layout, and paint implementation:
   React in the Wry host, with typed viewport input forwarded to Rust.
2. Scene mutations go through undoable editor commands. Direct World mutation
   is reserved for runtime preview and is never authoring state.
3. Persistent assets are owned by `AssetRegistry`. Tool-generated previews use
   an explicit temporary ownership API and never appear as project content.
4. Panels consume typed snapshots and commands. Domain models never depend on
   React, Wry, or another UI toolkit.
5. Every async or destructive operation exposes progress, diagnostics, cancel
   behavior where possible, and an explicit result.
6. No placeholder buttons, test-only production injection, hidden old route,
   `allow(dead_code)`, or disabled tests may be used to claim completion.
7. Layout, open documents, selection, filters, and project preferences survive
   an editor restart.

## Legacy implementation deletion inventory

The following are migration sources only and must not remain when their data
model has moved:

- `engine-editor/src/editor_ui.rs` and its `EditorUi`, `UiKey`, interaction
  stamp, synthetic pointer capture, widget layout, and batch generation.
- The legacy editor render block in `sandbox/src/editor_app.rs`, including
  `self.ui`, ordered legacy toolbar/panel replay, pointer blockers, and legacy
  panel rendering.
- `draw_asset_browser`, `draw_material_editor`, and every `ui(&mut EditorUi)` or
  `ui_with_*(&mut EditorUi)` implementation.
- Old `EditorPanel`/`ComponentInspector` traits whose rendering contract is
  tied to `EditorUi`.
- Tests that verify the old widget toolkit. They are replaced with model,
  command, React contract, host integration, and end-to-end editor tests.
- `sandbox/src/editor_app/egui_editor.rs`,
  `engine-editor/src/egui_bridge.rs`, their texture conversion path, and all
  egui dependencies.

An architecture test will reject new production references to `EditorUi`,
`editor_ui`, or the removed legacy draw APIs.

## Editor shell and workspaces

The shell provides:

- main menu: File, Edit, Assets, GameObject, Component, Window, Build, Help;
- dockable, closable, tabbed panels with saved layouts and reset-to-default;
- named workspaces for Scene, Scripting, Animation, Shading, and Debugging;
- command palette and a centralized command/shortcut registry;
- status bar for background jobs, import/build state, renderer status, and
  diagnostics counts;
- modal service for unsaved changes, destructive actions, errors, and progress;
- editor preferences and project settings separated by ownership.

Acceptance:

- layout and workspace restoration are deterministic;
- no command exists only as a menu item without an executable handler;
- shortcuts and menu commands invoke the same command object;
- keyboard focus, IME text, clipboard, cursor icons, and DPI scaling work on
  Windows, Linux, and macOS integrations.

## Project and scene lifecycle

Features:

- create/open/recent project;
- new/open/save/save-as/duplicate scene;
- multiple open scene documents with independent dirty state;
- startup scene selection and project scene catalog management;
- atomic save, recovery snapshots, crash recovery, and unsaved-close workflow;
- scene validation and dependency diagnostics before Play or Build.

Acceptance:

- a project and all its scenes can be managed without editing JSON/RON;
- failed load/save never replaces the current good document;
- recovery restores the latest autosave without corrupting the source scene.

## Hierarchy and prefab authoring

Features:

- real parent/child tree, expansion state, search, filters, and multi-selection;
- create menus for empty objects and registered component templates;
- rename, duplicate, delete, copy/paste, cut, and drag-to-reparent;
- sibling ordering and world/local transform preservation on reparent;
- prefab create/open/instantiate/unpack/apply/revert and override visualization;
- context menus and keyboard navigation.

Acceptance:

- every structural edit is one undoable transaction;
- recursive delete and reparent cannot create dangling references or cycles;
- prefab overrides round-trip through save/reload.

## Scene and Game views

Features:

- orbit, pan, zoom, fly, focus-selection, frame-all, and camera speed controls;
- perspective/orthographic modes and standard axis views;
- single/multi-selection picking, marquee selection, and selection outline;
- translate/rotate/scale/rect gizmos, pivot/center, local/global, numeric entry,
  snapping, surface snap, and vertex snap;
- grid configuration, lighting modes, wireframe, bounds, collider, navmesh,
  audio, and performance overlays;
- camera previews and aspect/resolution controls in Game view;
- drag-and-drop assets into the Scene view to instantiate suitable objects.

Acceptance:

- viewport input never leaks through editor chrome;
- all gizmo edits preview live and commit exactly one undo command;
- Scene camera state never mutates the authored game camera.

## Declarative Inspector and component catalog

The component catalog owns editor metadata for every registered component:

- stable type ID, display name, category, icon, schema version, dependencies,
  conflicts, default record, field metadata, and custom editor hooks;
- add/remove/enable/reset/copy/paste/reorder components;
- bool, integer, float, string, enum, vector, quaternion-as-Euler, color, asset,
  entity, list, map, optional, and nested struct editors;
- units, ranges, validation, tooltips, mixed values, and read-only runtime data;
- multi-object editing and prefab override indicators;
- searchable Add Component menu and automatic dependency offers.

Acceptance:

- no component-specific defaults are duplicated in the UI host;
- every serialized `Value` variant has a complete editor;
- adding, removing, editing, reset, and multi-edit are undoable and validated.

## Project and asset workflow

Features:

- folder tree, grid/list views, breadcrumbs, search, type/status filters;
- OS drag-and-drop import and an explicit Import dialog;
- create folder/material/shader/script/prefab/scene assets;
- rename, move, duplicate, delete-to-trash, reveal in file manager, and copy ID;
- thumbnails and previews generated asynchronously;
- reimport settings, importer diagnostics, dependency and reverse-dependency
  views, stale/failed/importing status, and cancel/retry;
- drag/drop asset and entity pickers with type checking.

Acceptance:

- filesystem, manifest, AssetId, registry, and dependency graph change as one
  recoverable operation;
- moving or renaming an asset preserves references or reports every blocked
  reference before changing data;
- tool-owned temporary assets never appear as project assets.

## Domain editors

Complete editors are required for:

- Material and Shader: parameters, texture slots, variants, live preview,
  compile diagnostics, source navigation, and save/reimport;
- Prefab: isolated stage, hierarchy, overrides, apply/revert;
- Animation: clips, timeline, curves, events, animator state graph, preview;
- Audio: clip preview, waveform, source settings, mixer routing;
- Physics: collider shapes, rigid body properties, joint visualization;
- Navigation: bake settings, navmesh visualization, agents and obstacles;
- Render settings: environment, lighting, shadows, post-processing and pass
  graph configuration.

## Scripting workflow

Features:

- create C# script from templates and attach it to entities;
- searchable class picker based on the built assembly;
- build, cancel, diagnostics navigation, and clear status;
- hot reload with state/compatibility reporting and rollback;
- open file at line in the configured IDE;
- serialized script fields in Inspector;
- runtime exception stack traces and entity/component context;
- play-time debug controls and explicit domain reload behavior.

Acceptance:

- a user can create, attach, compile, fix, reload, and run a script without
  leaving the editor except for source editing;
- a failed build or reload preserves the last working runtime.

## Play, diagnostics, profiling, and debugging

Features:

- Play/Pause/Step/Stop with a guaranteed authoring snapshot restore;
- configurable Play options and domain/scene reload behavior;
- structured Console filters, collapse, search, clear-on-play, source/entity
  navigation, export, and persistent history limits;
- frame CPU/GPU timing, render passes, draw calls, memory, assets, scripts,
  physics, audio, and navigation profiler modules;
- renderer and world inspection without writable backdoors.

## Project settings, input, and build/export

Features:

- typed project settings, startup scenes, quality/render settings, physics,
  audio, scripting, editor preferences, and input action editor;
- build profiles per target platform;
- scene inclusion/order, development flags, content cooking, script build,
  packaging, progress, cancel, diagnostics, and output reveal/run;
- preflight validation and a reproducible build report.

Acceptance:

- a clean project can be configured, built, packaged, and launched from the
  editor on every supported target;
- build output never silently falls back to editor/runtime-only assets.

## Extensibility

- editor commands, panels, component descriptors, importers, asset editors,
  menu entries, shortcuts, project validators, and build steps use versioned
  extension contracts;
- extensions receive narrow services rather than mutable global editor state;
- extension failures are isolated and diagnosed.

## Verification strategy

Each subsystem requires:

1. model and command unit tests;
2. persistence/undo/redo round-trip tests;
3. React protocol and native-host interaction tests for focus and event routing;
4. architecture tests for forbidden legacy dependencies;
5. end-to-end tests using a temporary project;
6. a real Vulkan editor run with screenshots and scripted input;
7. workspace and requested feature checks with no new warnings.

The final acceptance project must be authored from an empty project through
the UI and contain a scene, camera, light, renderable assets, material edits,
input actions, a compiled script, Play-mode behavior, diagnostics-free save,
and a packaged runnable build.
