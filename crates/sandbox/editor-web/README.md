# React editor shell

This package is the production editor chrome. React owns menus, docking, panels, inspectors, asset browsing, console, animation, profiling, build UI, and settings. Rust remains authoritative for project and scene state. The Scene and Game panel bodies are transparent native viewport slots; they do not create a WebGL renderer.

## Build

```powershell
npm install
npm run check
npm run build
```

`npm run check` also compares every canonical command in the Rust dispatcher
with `EditorCommandMap` and rejects UI calls outside that catalog.

For hot reload inside the real native editor, run `npm run dev` and launch a
debug editor with:

```powershell
$env:ENGINE_EDITOR_WEB_DEV_URL = 'http://127.0.0.1:4319'
cargo run -p sandbox --features backend-vulkan,tooling-editor,target-desktop -- editor <project>
```

The native host accepts only an explicit loopback HTTP development URL. The
Vulkan surface, typed bridge, and command path are unchanged.

The stable output contract is:

- `dist/index.html`
- `dist/assets/editor.js`
- `dist/assets/editor.css`

`npm run build` also writes `dist/build-manifest.json`. Tooling-editor builds reject missing,
modified, or source-stale bundles; the only debug bypass is an explicit loopback
`ENGINE_EDITOR_WEB_DEV_URL` development server.

The Rust host should expose these resources through Wry's custom protocol with the correct MIME types. Do not pass the bundled JavaScript to `with_html`; Windows WebView2 limits that API to roughly 2 MB.

## Native bridge

The shell sends JSON only through Wry's canonical `window.ipc.postMessage` transport:

```json
{
  "protocol": "EngineEditorIpc-v1",
  "id": "request-id",
  "method": "document.save",
  "params": {},
  "sessionId": "host-session",
  "baseRevision": 42
}
```

The host replies only by calling `window.__ENGINE_EDITOR_RECEIVE__(envelope)`. Every envelope carries `EngineEditorIpc-v1`. Responses use `id`, `result` or `error`, plus the authoritative `sessionId` and `revision`. The three typed events are `project.changed` (a complete authoritative snapshot), `editor.telemetry` (complete replacements for the `performance`, `animation`, and `build` domains), and `ui.openPanel` (native-requested dock navigation). Telemetry is accepted only for the current snapshot revision, so a mutation response can never apply telemetry to stale React state. All events use monotonic `sequence` and `revision` counters. The complete typed contract lives in `src/bridge/protocol.ts`; the shell intentionally has no generic string-command or DOM-event compatibility path.

`viewport.bounds` reports the active Scene or Game viewport rectangle in CSS pixels. `viewport.input` forwards pointer, wheel, focus, and keyboard input for that transparent region. The native host owns rendering and must keep using the normal `EngineRuntime -> Renderer -> SceneRenderer` pipeline.

Opening this package directly in a browser renders the real shell but leaves it disconnected. It never installs mock project state or a test-only command path into the production bundle.
