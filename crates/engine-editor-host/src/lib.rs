//! Native desktop host for the production React editor.
//!
//! This crate owns the Tao window and the single Wry WebView. It intentionally knows nothing
//! about scenes, assets, rendering implementations or editor commands. The engine initializes
//! its renderer from [`HostEvent::SurfaceReady`] and exchanges serialized messages through
//! [`HostEvent::Ipc`] and [`HostDirective::EvaluateScript`].

mod platform;
mod protocol;

use std::path::PathBuf;

use protocol::AssetRouter;
use raw_window_handle::HandleError;
use tao::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use wry::{DragDropEvent, NewWindowResponse, Rect, WebView, WebViewBuilder};

/// The sole custom protocol used by the production editor.
pub const EDITOR_PROTOCOL: &str = "engine-editor";

/// Response policy for every embedded editor resource.
pub const EDITOR_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; media-src 'none'; object-src 'none'; child-src 'none'; worker-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// A compile-time web resource. MIME types are derived from the canonical path by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebAsset {
    pub path: &'static str,
    pub bytes: &'static [u8],
}

impl WebAsset {
    pub const fn new(path: &'static str, bytes: &'static [u8]) -> Self {
        Self { path, bytes }
    }
}

/// Immutable startup configuration for the native editor host.
#[derive(Debug, Clone)]
pub struct EditorHostConfig {
    title: String,
    initial_size: PhysicalSize<u32>,
    minimum_size: PhysicalSize<u32>,
    entry_path: &'static str,
    assets: &'static [WebAsset],
    initialization_script: Option<&'static str>,
    development_url: Option<String>,
}

impl EditorHostConfig {
    pub fn new(title: impl Into<String>, assets: &'static [WebAsset]) -> Self {
        Self {
            title: title.into(),
            initial_size: PhysicalSize::new(1600, 1000),
            minimum_size: PhysicalSize::new(1024, 640),
            entry_path: "index.html",
            assets,
            initialization_script: None,
            development_url: None,
        }
    }

    pub fn with_initial_size(mut self, width: u32, height: u32) -> Self {
        self.initial_size = PhysicalSize::new(width, height);
        self
    }

    pub fn with_minimum_size(mut self, width: u32, height: u32) -> Self {
        self.minimum_size = PhysicalSize::new(width, height);
        self
    }

    pub fn with_entry_path(mut self, entry_path: &'static str) -> Self {
        self.entry_path = entry_path;
        self
    }

    /// Adds a document-start script. Keep this limited to a versioned bridge bootstrap; editor
    /// state and commands should still cross the typed IPC protocol.
    pub fn with_initialization_script(mut self, script: &'static str) -> Self {
        self.initialization_script = Some(script);
        self
    }

    /// Loads the same React application from an explicitly configured loopback development
    /// server. This is intended for Vite hot reload during editor UI development; production
    /// builds continue to use the compile-time assets and locked-down custom protocol.
    pub fn with_development_url(mut self, url: impl Into<String>) -> Self {
        self.development_url = Some(url.into());
        self
    }

    fn validate(&self) -> Result<ValidatedConfig, HostError> {
        if self.title.trim().is_empty() {
            return Err(HostError::InvalidConfig(
                "editor window title cannot be empty".into(),
            ));
        }
        if self.initial_size.width == 0
            || self.initial_size.height == 0
            || self.minimum_size.width == 0
            || self.minimum_size.height == 0
        {
            return Err(HostError::InvalidConfig(
                "editor window dimensions must be non-zero".into(),
            ));
        }
        let router = AssetRouter::new(EDITOR_PROTOCOL, self.entry_path, self.assets)?;
        let development = self
            .development_url
            .as_deref()
            .map(validate_development_url)
            .transpose()?;
        let initial_url = development
            .as_ref()
            .map(|development| development.url.clone())
            .unwrap_or_else(|| router.initial_url());
        Ok(ValidatedConfig {
            router,
            initial_url,
            development_origin: development.map(|development| development.origin),
        })
    }
}

#[derive(Debug)]
struct DevelopmentUrl {
    url: String,
    origin: String,
}

#[derive(Debug)]
struct ValidatedConfig {
    router: AssetRouter,
    initial_url: String,
    development_origin: Option<String>,
}

fn validate_development_url(url: &str) -> Result<DevelopmentUrl, HostError> {
    let trimmed = url.trim();
    let authority_and_path = trimmed.strip_prefix("http://").ok_or_else(|| {
        HostError::InvalidConfig(
            "editor development URL must use http:// on a loopback host".into(),
        )
    })?;
    let authority = authority_and_path
        .split_once('/')
        .map_or(authority_and_path, |(authority, _)| authority);
    if authority.is_empty()
        || authority.contains('@')
        || authority_and_path.contains('?')
        || authority_and_path.contains('#')
    {
        return Err(HostError::InvalidConfig(
            "editor development URL must not contain credentials, a query, or a fragment".into(),
        ));
    }
    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    if !matches!(host, "127.0.0.1" | "localhost") {
        return Err(HostError::InvalidConfig(
            "editor development URL host must be localhost or 127.0.0.1".into(),
        ));
    }
    if let Some(port) = port {
        if port.parse::<u16>().ok().filter(|port| *port != 0).is_none() {
            return Err(HostError::InvalidConfig(
                "editor development URL has an invalid port".into(),
            ));
        }
    }
    let origin = format!("http://{authority}");
    let url = if authority_and_path.contains('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    };
    Ok(DevelopmentUrl { url, origin })
}

/// Native events delivered synchronously on the editor UI thread.
#[derive(Debug)]
pub enum HostEvent {
    /// The render surface is alive. The opaque surface remains valid until
    /// `run_editor_host` returns and must stay on this UI thread. On Linux it
    /// identifies the lower GTK DrawingArea, not the WebView overlay.
    SurfaceReady {
        surface: ::platform::PlatformSurface,
        size: PhysicalSize<u32>,
        scale_factor: f64,
    },
    Ipc(String),
    FileDropped {
        paths: Vec<PathBuf>,
        position: PhysicalPosition<i32>,
    },
    Resized(PhysicalSize<u32>),
    ScaleFactorChanged {
        scale_factor: f64,
        size: PhysicalSize<u32>,
    },
    /// Whether the native surface is fully occluded. Rendering is suspended while occluded and
    /// restarted with one frame when the surface becomes visible again.
    Occluded(bool),
    Focused(bool),
    Redraw,
    CloseRequested,
}

/// Action requested by the engine after handling one host event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostDirective {
    Continue,
    Exit,
    EvaluateScript(String),
    RequestRedraw,
    SetWebViewVisible(bool),
    SetWindowTitle(String),
    Batch(Vec<HostDirective>),
}

/// Engine-side event sink. It stays on the Tao/GTK UI thread for the whole host lifetime.
pub trait EditorHostClient: 'static {
    fn on_host_event(&mut self, event: HostEvent) -> HostDirective;
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("invalid editor host configuration: {0}")]
    InvalidConfig(String),
    #[error("failed to create editor window: {0}")]
    Window(String),
    #[error("webview error: {0}")]
    WebView(#[source] wry::Error),
    #[error("native handle error: {0}")]
    Handle(#[from] HandleError),
    #[error("platform host error: {0}")]
    Platform(String),
    #[error("desktop event loop disconnected with code {0}")]
    EventLoopDisconnected(i32),
}

#[derive(Debug)]
enum UserEvent {
    Ipc(String),
    FileDropped {
        paths: Vec<PathBuf>,
        position: PhysicalPosition<i32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedrawGate {
    occluded: bool,
    zero_sized: bool,
}

impl RedrawGate {
    fn new(size: PhysicalSize<u32>) -> Self {
        Self {
            occluded: false,
            zero_sized: size.width == 0 || size.height == 0,
        }
    }

    fn allows_redraw(self) -> bool {
        !self.occluded && !self.zero_sized
    }

    fn update_size(&mut self, size: PhysicalSize<u32>) -> bool {
        let was_suspended = !self.allows_redraw();
        self.zero_sized = size.width == 0 || size.height == 0;
        was_suspended && self.allows_redraw()
    }

    fn update_occluded(&mut self, occluded: bool) -> bool {
        let was_suspended = !self.allows_redraw();
        self.occluded = occluded;
        was_suspended && self.allows_redraw()
    }
}

/// Runs the only production editor window and returns after the client requests exit.
///
/// This function must be called from the process main thread. It owns all native UI objects and
/// drops the WebView before the render window and Linux DrawingArea become invalid.
pub fn run_editor_host<C>(config: EditorHostConfig, client: C) -> Result<(), HostError>
where
    C: EditorHostClient,
{
    let validated = config.validate()?;
    let router = validated.router;
    platform::initialize()?;

    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(config.title.clone())
        .with_inner_size(config.initial_size)
        .with_min_inner_size(config.minimum_size)
        .with_resizable(true)
        .with_transparent(true)
        .build(&event_loop)
        .map_err(|error| HostError::Window(error.to_string()))?;
    let native_surface = platform::prepare(&window)?;
    let initial_size = window.inner_size();
    let mut redraw_gate = RedrawGate::new(initial_size);
    let mut redraw_pending = false;

    let ipc_proxy = proxy.clone();
    let drop_proxy = proxy;
    let navigation_router = router;
    let development_origin = validated.development_origin;
    let protocol_router = router;
    let mut builder = WebViewBuilder::new()
        .with_bounds(full_window_bounds(initial_size))
        .with_transparent(true)
        .with_clipboard(true)
        .with_hotkeys_zoom(false)
        .with_custom_protocol(EDITOR_PROTOCOL.into(), move |_id, request| {
            protocol_router.response(request)
        })
        .with_navigation_handler(move |url| {
            navigation_router.allows_navigation(&url)
                || development_origin
                    .as_deref()
                    .is_some_and(|origin| protocol::has_exact_origin(&url, origin))
        })
        .with_new_window_req_handler(|_url, _features| NewWindowResponse::Deny)
        .with_ipc_handler(move |request| {
            let _ = ipc_proxy.send_event(UserEvent::Ipc(request.body().clone()));
        })
        .with_drag_drop_handler(move |event| {
            if let DragDropEvent::Drop { paths, position } = event {
                let _ = drop_proxy.send_event(UserEvent::FileDropped {
                    paths,
                    position: PhysicalPosition::new(position.0, position.1),
                });
            }
            true
        })
        .with_url(validated.initial_url);
    if let Some(script) = config.initialization_script {
        builder = builder.with_initialization_script(script);
    }
    let webview = platform::build_webview(builder, &window, &native_surface)?;

    // Declare the client after every native surface owner. Rust's reverse drop order then tears
    // down the engine renderer held by the client before the WebView, GTK DrawingArea and Tao
    // window that back its graphics surface.
    let mut client = client;
    let mut runtime_error = None;
    let initial_directive = client.on_host_event(HostEvent::SurfaceReady {
        surface: native_surface.surface,
        size: initial_size,
        scale_factor: window.scale_factor(),
    });
    let mut initial_control_flow = ControlFlow::Wait;
    apply_directive(
        initial_directive,
        &window,
        &webview,
        &mut initial_control_flow,
        &mut runtime_error,
        &mut redraw_pending,
        redraw_gate.allows_redraw(),
    );
    if let Some(error) = runtime_error.take() {
        return Err(error);
    }
    if matches!(initial_control_flow, ControlFlow::ExitWithCode(_)) {
        return Ok(());
    }

    let exit_code = event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let mut resumed = false;
        let host_event = match event {
            Event::UserEvent(UserEvent::Ipc(message)) => Some(HostEvent::Ipc(message)),
            Event::UserEvent(UserEvent::FileDropped { paths, position }) => {
                Some(HostEvent::FileDropped { paths, position })
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Resized(size),
                ..
            } if window_id == window.id() => {
                resumed = redraw_gate.update_size(size);
                if size.width != 0 && size.height != 0 {
                    if let Err(error) = webview.set_bounds(full_window_bounds(size)) {
                        runtime_error = Some(HostError::WebView(error));
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
                Some(HostEvent::Resized(size))
            }
            Event::WindowEvent {
                window_id,
                event:
                    WindowEvent::ScaleFactorChanged {
                        scale_factor,
                        new_inner_size,
                    },
                ..
            } if window_id == window.id() => {
                let size = *new_inner_size;
                resumed = redraw_gate.update_size(size);
                if size.width != 0 && size.height != 0 {
                    if let Err(error) = webview.set_bounds(full_window_bounds(size)) {
                        runtime_error = Some(HostError::WebView(error));
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
                Some(HostEvent::ScaleFactorChanged { scale_factor, size })
            }
            Event::Suspended => {
                resumed = redraw_gate.update_occluded(true);
                Some(HostEvent::Occluded(true))
            }
            Event::Resumed => {
                resumed = redraw_gate.update_occluded(false);
                Some(HostEvent::Occluded(false))
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Focused(focused),
                ..
            } if window_id == window.id() => Some(HostEvent::Focused(focused)),
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if window_id == window.id() => Some(HostEvent::CloseRequested),
            Event::RedrawRequested(window_id) if window_id == window.id() => {
                Some(HostEvent::Redraw)
            }
            Event::MainEventsCleared => {
                platform::pump_events();
                if redraw_pending && redraw_gate.allows_redraw() {
                    redraw_pending = false;
                    window.request_redraw();
                }
                None
            }
            _ => None,
        };

        if let Some(host_event) = host_event {
            let directive = client.on_host_event(host_event);
            apply_directive(
                directive,
                &window,
                &webview,
                control_flow,
                &mut runtime_error,
                &mut redraw_pending,
                redraw_gate.allows_redraw(),
            );
            if resumed
                && runtime_error.is_none()
                && !matches!(control_flow, ControlFlow::ExitWithCode(_))
            {
                redraw_pending = true;
            }
        }
        // Tao may coalesce a request made while handling `RedrawRequested` on Windows. Keep the
        // loop alive until `MainEventsCleared` can issue the request outside that callback. A
        // failed render returns `Continue`, leaving this false and stopping the self-driven loop
        // until an external event asks for another frame.
        if redraw_pending && redraw_gate.allows_redraw() {
            *control_flow = ControlFlow::Poll;
        }
    });

    if let Some(error) = runtime_error {
        return Err(error);
    }
    if exit_code != 0 {
        return Err(HostError::EventLoopDisconnected(exit_code));
    }
    Ok(())
}

fn full_window_bounds(size: PhysicalSize<u32>) -> Rect {
    Rect {
        position: PhysicalPosition::new(0, 0).into(),
        size: size.into(),
    }
}

fn apply_directive(
    directive: HostDirective,
    window: &tao::window::Window,
    webview: &WebView,
    control_flow: &mut ControlFlow,
    runtime_error: &mut Option<HostError>,
    redraw_pending: &mut bool,
    redraw_allowed: bool,
) {
    match directive {
        HostDirective::Continue => {}
        HostDirective::Exit => *control_flow = ControlFlow::Exit,
        HostDirective::EvaluateScript(script) => {
            if let Err(error) = webview.evaluate_script(&script) {
                *runtime_error = Some(HostError::WebView(error));
                *control_flow = ControlFlow::Exit;
            }
        }
        HostDirective::RequestRedraw if redraw_allowed => *redraw_pending = true,
        HostDirective::RequestRedraw => {}
        HostDirective::SetWebViewVisible(visible) => {
            if let Err(error) = webview.set_visible(visible) {
                *runtime_error = Some(HostError::WebView(error));
                *control_flow = ControlFlow::Exit;
            }
        }
        HostDirective::SetWindowTitle(title) => window.set_title(&title),
        HostDirective::Batch(directives) => {
            for directive in directives {
                apply_directive(
                    directive,
                    window,
                    webview,
                    control_flow,
                    runtime_error,
                    redraw_pending,
                    redraw_allowed,
                );
                if runtime_error.is_some() || matches!(control_flow, ControlFlow::ExitWithCode(_)) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_zero_window_size() {
        static ASSETS: &[WebAsset] = &[WebAsset::new("index.html", b"ok")];
        let config = EditorHostConfig::new("Editor", ASSETS).with_initial_size(0, 800);
        assert!(matches!(
            config.validate(),
            Err(HostError::InvalidConfig(_))
        ));
    }

    #[test]
    fn config_rejects_missing_entry() {
        static ASSETS: &[WebAsset] = &[WebAsset::new("shell.html", b"ok")];
        let config = EditorHostConfig::new("Editor", ASSETS);
        assert!(matches!(
            config.validate(),
            Err(HostError::InvalidConfig(_))
        ));
    }

    #[test]
    fn development_url_is_restricted_to_loopback_http() {
        static ASSETS: &[WebAsset] = &[WebAsset::new("index.html", b"ok")];
        for rejected in [
            "https://localhost:4319",
            "http://example.com:4319",
            "http://user@localhost:4319",
            "http://localhost:4319/?token=secret",
            "http://localhost:0",
        ] {
            let config = EditorHostConfig::new("Editor", ASSETS).with_development_url(rejected);
            assert!(matches!(
                config.validate(),
                Err(HostError::InvalidConfig(_))
            ));
        }

        let validated = EditorHostConfig::new("Editor", ASSETS)
            .with_development_url("http://127.0.0.1:4319")
            .validate()
            .unwrap();
        assert_eq!(validated.initial_url, "http://127.0.0.1:4319/");
        assert_eq!(
            validated.development_origin.as_deref(),
            Some("http://127.0.0.1:4319")
        );
    }

    #[test]
    fn redraw_gate_resumes_once_after_minimize_and_occlusion() {
        let mut gate = RedrawGate::new(PhysicalSize::new(1600, 900));
        assert!(gate.allows_redraw());

        assert!(!gate.update_size(PhysicalSize::new(0, 0)));
        assert!(!gate.allows_redraw());
        assert!(!gate.update_size(PhysicalSize::new(0, 900)));
        assert!(gate.update_size(PhysicalSize::new(1600, 900)));
        assert!(gate.allows_redraw());
        assert!(!gate.update_size(PhysicalSize::new(1600, 900)));

        assert!(!gate.update_occluded(true));
        assert!(!gate.allows_redraw());
        assert!(!gate.update_occluded(true));
        assert!(gate.update_occluded(false));
        assert!(gate.allows_redraw());
        assert!(!gate.update_occluded(false));
    }
}
