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

/// A keyboard key identifier (scancode-based, layout-independent).
pub type KeyCode = u32;

/// Mouse button identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Platform-level events delivered to a [`WindowApp`].
#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    // ── Lifecycle ──
    Resumed,
    Suspended,
    Resized { width: u32, height: u32 },
    Redraw,
    CloseRequested,

    // ── Keyboard ──
    KeyPressed {
        key: KeyCode,
        modifiers: Modifiers,
    },
    KeyReleased {
        key: KeyCode,
        modifiers: Modifiers,
    },

    // ── Mouse ──
    MouseMoved { x: f64, y: f64 },
    MousePressed {
        button: MouseButton,
        x: f64,
        y: f64,
    },
    MouseReleased {
        button: MouseButton,
        x: f64,
        y: f64,
    },
    MouseWheelScrolled { delta: (f32, f32) },

    // ── Text input ──
    CharacterTyped { character: char },
}

/// Keyboard modifier flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Modifiers {
    pub fn from_winit(mods: &winit::keyboard::ModifiersState) -> Self {
        Self {
            shift: mods.shift_key(),
            ctrl: mods.control_key(),
            alt: mods.alt_key(),
            logo: mods.super_key(),
        }
    }
}

pub use self::input_types::*;
mod input_types {
    /// Key codes (subset of winit's VirtualKeyCode for engine use).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[allow(dead_code)]
    pub enum KeyCode {
        Escape, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
        Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
        A, B, C, D, E, F, G, H, I, J, K, L, M,
        N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        Space, Enter, Backspace, Tab, Delete,
        Left, Right, Up, Down,
        LShift, RShift, LControl, RControl, LAlt, RAlt,
        Other(u32),
    }

    impl From<winit::keyboard::KeyCode> for KeyCode {
        fn from(k: winit::keyboard::KeyCode) -> Self {
            match k {
                winit::keyboard::KeyCode::Escape => Self::Escape,
                winit::keyboard::KeyCode::F1 => Self::F1,
                winit::keyboard::KeyCode::F2 => Self::F2,
                winit::keyboard::KeyCode::F3 => Self::F3,
                winit::keyboard::KeyCode::F4 => Self::F4,
                winit::keyboard::KeyCode::F5 => Self::F5,
                winit::keyboard::KeyCode::F6 => Self::F6,
                winit::keyboard::KeyCode::F7 => Self::F7,
                winit::keyboard::KeyCode::F8 => Self::F8,
                winit::keyboard::KeyCode::F9 => Self::F9,
                winit::keyboard::KeyCode::F10 => Self::F10,
                winit::keyboard::KeyCode::F11 => Self::F11,
                winit::keyboard::KeyCode::F12 => Self::F12,
                winit::keyboard::KeyCode::Digit0 => Self::Key0,
                winit::keyboard::KeyCode::Digit1 => Self::Key1,
                winit::keyboard::KeyCode::Digit2 => Self::Key2,
                winit::keyboard::KeyCode::Digit3 => Self::Key3,
                winit::keyboard::KeyCode::Digit4 => Self::Key4,
                winit::keyboard::KeyCode::Digit5 => Self::Key5,
                winit::keyboard::KeyCode::Digit6 => Self::Key6,
                winit::keyboard::KeyCode::Digit7 => Self::Key7,
                winit::keyboard::KeyCode::Digit8 => Self::Key8,
                winit::keyboard::KeyCode::Digit9 => Self::Key9,
                winit::keyboard::KeyCode::KeyA => Self::A,
                winit::keyboard::KeyCode::KeyB => Self::B,
                winit::keyboard::KeyCode::KeyC => Self::C,
                winit::keyboard::KeyCode::KeyD => Self::D,
                winit::keyboard::KeyCode::KeyE => Self::E,
                winit::keyboard::KeyCode::KeyF => Self::F,
                winit::keyboard::KeyCode::KeyG => Self::G,
                winit::keyboard::KeyCode::KeyH => Self::H,
                winit::keyboard::KeyCode::KeyI => Self::I,
                winit::keyboard::KeyCode::KeyJ => Self::J,
                winit::keyboard::KeyCode::KeyK => Self::K,
                winit::keyboard::KeyCode::KeyL => Self::L,
                winit::keyboard::KeyCode::KeyM => Self::M,
                winit::keyboard::KeyCode::KeyN => Self::N,
                winit::keyboard::KeyCode::KeyO => Self::O,
                winit::keyboard::KeyCode::KeyP => Self::P,
                winit::keyboard::KeyCode::KeyQ => Self::Q,
                winit::keyboard::KeyCode::KeyR => Self::R,
                winit::keyboard::KeyCode::KeyS => Self::S,
                winit::keyboard::KeyCode::KeyT => Self::T,
                winit::keyboard::KeyCode::KeyU => Self::U,
                winit::keyboard::KeyCode::KeyV => Self::V,
                winit::keyboard::KeyCode::KeyW => Self::W,
                winit::keyboard::KeyCode::KeyX => Self::X,
                winit::keyboard::KeyCode::KeyY => Self::Y,
                winit::keyboard::KeyCode::KeyZ => Self::Z,
                winit::keyboard::KeyCode::Space => Self::Space,
                winit::keyboard::KeyCode::Enter => Self::Enter,
                winit::keyboard::KeyCode::Backspace => Self::Backspace,
                winit::keyboard::KeyCode::Tab => Self::Tab,
                winit::keyboard::KeyCode::Delete => Self::Delete,
                winit::keyboard::KeyCode::ArrowLeft => Self::Left,
                winit::keyboard::KeyCode::ArrowRight => Self::Right,
                winit::keyboard::KeyCode::ArrowUp => Self::Up,
                winit::keyboard::KeyCode::ArrowDown => Self::Down,
                winit::keyboard::KeyCode::ShiftLeft => Self::LShift,
                winit::keyboard::KeyCode::ShiftRight => Self::RShift,
                winit::keyboard::KeyCode::ControlLeft => Self::LControl,
                winit::keyboard::KeyCode::ControlRight => Self::RControl,
                winit::keyboard::KeyCode::AltLeft => Self::LAlt,
                winit::keyboard::KeyCode::AltRight => Self::RAlt,
                other => Self::Other(other as u32),
            }
        }
    }

    /// Pressed or released.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ElementState {
        Pressed,
        Released,
    }

    /// Mouse button identifiers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    #[allow(dead_code)]
    pub enum MouseButton {
        Left, Right, Middle, Other(u16),
    }

    impl From<winit::event::MouseButton> for MouseButton {
        fn from(b: winit::event::MouseButton) -> Self {
            match b {
                winit::event::MouseButton::Left => Self::Left,
                winit::event::MouseButton::Right => Self::Right,
                winit::event::MouseButton::Middle => Self::Middle,
                _ => Self::Other(0),
            }
        }
    }

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
        if let Some(ev) = self.translate_event(&event) {
            let flow = self.app.on_event(&window, ev);
            if matches!(flow, EventFlow::Exit) {
                event_loop.exit();
            }
        }
        window.request_redraw();
    }
}

impl<A: WindowApp> Wrapper<A> {
    fn translate_event(&self, event: &WinitWindowEvent) -> Option<PlatformEvent> {
        use winit::event::ElementState as WinitState;
        use winit::event::MouseScrollDelta;
        match event {
            WinitWindowEvent::Resized(size) => Some(PlatformEvent::Resized {
                width: size.width,
                height: size.height,
            }),
            WinitWindowEvent::CloseRequested => Some(PlatformEvent::CloseRequested),
            WinitWindowEvent::RedrawRequested => Some(PlatformEvent::Redraw),

            // ── Keyboard ──────────────────────────────────────────────
            WinitWindowEvent::KeyboardInput { event: ke, .. } => {
                let modifiers = Modifiers {
                    shift: false, ctrl: false, alt: false, logo: false,
                };
                let key = match ke.physical_key {
                    winit::keyboard::PhysicalKey::Code(c) => c as u32,
                    _ => 0,
                };
                match ke.state {
                    WinitState::Pressed => {
                        Some(PlatformEvent::KeyPressed { key, modifiers })
                    }
                    WinitState::Released => {
                        Some(PlatformEvent::KeyReleased { key, modifiers })
                    }
                }
            }

            // ── Mouse ─────────────────────────────────────────────────
            WinitWindowEvent::CursorMoved { position, .. } => {
                Some(PlatformEvent::MouseMoved {
                    x: position.x,
                    y: position.y,
                })
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    _ => MouseButton::Other(0),
                };
                match state {
                    WinitState::Pressed => Some(PlatformEvent::MousePressed {
                        button: btn, x: 0.0, y: 0.0,
                    }),
                    WinitState::Released => Some(PlatformEvent::MouseReleased {
                        button: btn, x: 0.0, y: 0.0,
                    }),
                }
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                Some(PlatformEvent::MouseWheelScrolled { delta: (dx, dy) })
            }

            _ => None,
        }
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
        let c = a;
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
