#![forbid(unsafe_code)]

//! Platform layer for Gate 2.
//!
//! This crate owns the winit-based desktop window and event loop and
//! exposes a minimal [`WindowApp`] callback surface so renderer crates do
//! not need to depend on winit directly. Per `FD-013` the public boundary
//! is winit-only at this gate; the `PlatformAdapter` trait planned for
//! mobile/console comes in Gate 7.

use std::sync::Arc;

use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

pub use winit;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: "Engine Sandbox".to_string(),
            width: 1280,
            height: 720,
        }
    }
}

/// Platform-level events delivered to a [`WindowApp`].
///
/// Translated from winit so the renderer only sees the small set of
/// signals Gate 2 needs: resize, close, suspend/resume, and redraw.
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    Resumed,
    Suspended,
    Resized { width: u32, height: u32 },
    Redraw,
    CloseRequested,
}

/// Returned from [`WindowApp::on_event`] to request continuation or exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventFlow {
    Continue,
    Exit,
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("event loop initialization failed: {0}")]
    EventLoop(String),
    #[error("window creation failed: {0}")]
    WindowCreation(String),
}

/// Callback surface implemented by the consumer (sandbox / renderer).
///
/// `on_create` is invoked exactly once with the newly created window so
/// the consumer can build GPU resources tied to the window's raw handle.
pub trait WindowApp: 'static {
    fn on_create(&mut self, window: Arc<Window>);
    fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow;
}

/// Run the platform event loop, blocking the calling thread until exit.
pub fn run<A: WindowApp>(descriptor: WindowDescriptor, app: A) -> Result<(), PlatformError> {
    let event_loop = EventLoop::new().map_err(|e| PlatformError::EventLoop(e.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut wrapper = Wrapper {
        descriptor,
        app,
        window: None,
        created: false,
    };
    event_loop
        .run_app(&mut wrapper)
        .map_err(|e| PlatformError::EventLoop(e.to_string()))
}

struct Wrapper<A: WindowApp> {
    descriptor: WindowDescriptor,
    app: A,
    window: Option<Arc<Window>>,
    created: bool,
}

impl<A: WindowApp> ApplicationHandler for Wrapper<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = WindowAttributes::default()
                .with_title(&self.descriptor.title)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.descriptor.width,
                    self.descriptor.height,
                ));
            match event_loop.create_window(attrs) {
                Ok(window) => {
                    let window = Arc::new(window);
                    self.window = Some(window.clone());
                    if !self.created {
                        self.app.on_create(window);
                        self.created = true;
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "failed to create window");
                    event_loop.exit();
                    return;
                }
            }
        }
        if let Some(window) = self.window.as_ref() {
            if self.created {
                let _ = self.app.on_event(window, PlatformEvent::Resumed);
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            let _ = self.app.on_event(window, PlatformEvent::Suspended);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WinitWindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let translated = match event {
            WinitWindowEvent::Resized(size) => Some(PlatformEvent::Resized {
                width: size.width,
                height: size.height,
            }),
            WinitWindowEvent::CloseRequested => Some(PlatformEvent::CloseRequested),
            WinitWindowEvent::RedrawRequested => Some(PlatformEvent::Redraw),
            _ => None,
        };
        if let Some(ev) = translated {
            let flow = self.app.on_event(&window, ev);
            if matches!(flow, EventFlow::Exit) {
                event_loop.exit();
            }
        }
        window.request_redraw();
    }
}
