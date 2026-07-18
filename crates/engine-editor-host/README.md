# Engine editor host

`engine-editor-host` owns the production desktop editor window and its single
transparent React WebView. The game engine remains native: the `SurfaceReady`
event gives the caller the raw-window-handle 0.6 handles used to create the
renderer surface.

The host deliberately has no dependency on `sandbox` or editor domain crates.
Commands cross the boundary as JSON strings over Wry IPC; the caller owns
schema validation and command dispatch.

## Embedded assets

Production web assets are compile-time data:

```rust,ignore
static ASSETS: &[WebAsset] = &[
    WebAsset::new("index.html", include_bytes!("../../editor-web/dist/index.html")),
    WebAsset::new("assets/index.js", include_bytes!("../../editor-web/dist/assets/index.js")),
];
```

Only known file extensions are served. Requests are percent-decoded once and
paths containing traversal components, backslashes, NULs, duplicate separators
or a second encoded layer are rejected. Every response receives CSP and
`nosniff` headers. Navigation is limited to the editor custom protocol and its
WebView2 compatibility origin.

## React development server

For hot reload, start Vite in `crates/sandbox/editor-web` and launch a debug
editor with `ENGINE_EDITOR_WEB_DEV_URL=http://127.0.0.1:4319`. The host accepts
only an explicit loopback HTTP URL. Without the variable, including every
production build, the same React application is served from the compile-time
embedded assets.

## Platform layout

- Windows and macOS: a Tao window is the native render surface and Wry is a
  transparent, full-size child WebView.
- Linux: a GTK `Overlay` contains a `DrawingArea` below a `Fixed` containing the
  WebView. The DrawingArea is the native render surface on both X11 and Wayland.
  The raw pointer conversion is isolated in `src/platform/linux.rs`, together
  with the lifetime and thread-safety invariants that make it sound.
