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

#[cfg(test)]
mod tests {
    use super::*;

    // ── WindowDescriptor tests ───────────────────────────────────────────

    #[test]
    fn window_descriptor_defaults() {
        let desc = WindowDescriptor::default();
        assert_eq!(desc.title, "Engine Sandbox");
        assert_eq!(desc.width, 1280);
        assert_eq!(desc.height, 720);
    }

    #[test]
    fn window_descriptor_custom() {
        let desc = WindowDescriptor {
            title: "My Game".to_string(),
            width: 1920,
            height: 1080,
        };
        assert_eq!(desc.title, "My Game");
        assert_eq!(desc.width, 1920);
        assert_eq!(desc.height, 1080);
    }

    #[test]
    fn window_descriptor_partial_eq() {
        let a = WindowDescriptor::default();
        let b = WindowDescriptor::default();
        let c = WindowDescriptor {
            title: "Custom".to_string(),
            width: 800,
            height: 600,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn window_descriptor_debug() {
        let desc = WindowDescriptor::default();
        let debug = format!("{:?}", desc);
        assert!(debug.contains("WindowDescriptor"));
        assert!(debug.contains("Engine Sandbox"));
    }

    #[test]
    fn window_descriptor_clone() {
        let desc = WindowDescriptor::default();
        let cloned = desc.clone();
        assert_eq!(desc, cloned);
    }

    // ── EventFlow tests ──────────────────────────────────────────────────

    #[test]
    fn event_flow_continue_vs_exit() {
        assert_eq!(EventFlow::Continue, EventFlow::Continue);
        assert_eq!(EventFlow::Exit, EventFlow::Exit);
        assert_ne!(EventFlow::Continue, EventFlow::Exit);
    }

    #[test]
    fn event_flow_debug() {
        assert_eq!(format!("{:?}", EventFlow::Continue), "Continue");
        assert_eq!(format!("{:?}", EventFlow::Exit), "Exit");
    }

    #[test]
    fn event_flow_copy_clone() {
        let a = EventFlow::Continue;
        let b = a;
        let c = a.clone();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    // ── PlatformEvent tests ──────────────────────────────────────────────

    #[test]
    fn platform_event_resumed() {
        assert_eq!(PlatformEvent::Resumed, PlatformEvent::Resumed);
        assert_ne!(PlatformEvent::Resumed, PlatformEvent::Suspended);
    }

    #[test]
    fn platform_event_suspended() {
        assert_eq!(PlatformEvent::Suspended, PlatformEvent::Suspended);
    }

    #[test]
    fn platform_event_resized() {
        let a = PlatformEvent::Resized {
            width: 1920,
            height: 1080,
        };
        let b = PlatformEvent::Resized {
            width: 1920,
            height: 1080,
        };
        let c = PlatformEvent::Resized {
            width: 800,
            height: 600,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn platform_event_redraw() {
        assert_eq!(PlatformEvent::Redraw, PlatformEvent::Redraw);
    }

    #[test]
    fn platform_event_close_requested() {
        assert_eq!(PlatformEvent::CloseRequested, PlatformEvent::CloseRequested);
    }

    #[test]
    fn platform_event_debug() {
        assert_eq!(format!("{:?}", PlatformEvent::Redraw), "Redraw");
        assert_eq!(format!("{:?}", PlatformEvent::CloseRequested), "CloseRequested");
        assert_eq!(
            format!("{:?}", PlatformEvent::Resized { width: 100, height: 200 }),
            "Resized { width: 100, height: 200 }"
        );
    }

    #[test]
    fn platform_event_clone() {
        let ev = PlatformEvent::Resized {
            width: 640,
            height: 480,
        };
        let cloned = ev.clone();
        assert_eq!(ev, cloned);
    }

    // ── PlatformError tests ──────────────────────────────────────────────

    #[test]
    fn platform_error_event_loop_display() {
        let err = PlatformError::EventLoop("init failed".to_string());
        assert_eq!(
            err.to_string(),
            "event loop initialization failed: init failed"
        );
    }

    #[test]
    fn platform_error_window_creation_display() {
        let err = PlatformError::WindowCreation("no display".to_string());
        assert_eq!(err.to_string(), "window creation failed: no display");
    }

    #[test]
    fn platform_error_debug() {
        let err = PlatformError::EventLoop("err".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("EventLoop"));
    }
}
