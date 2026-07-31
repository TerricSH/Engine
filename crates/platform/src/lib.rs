#![forbid(unsafe_code)]

//! Platform layer.
//!
//! This crate owns the winit-based desktop window and event loop and
//! exposes a minimal [`WindowApp`] callback surface so renderer crates do
//! not need to depend on winit directly.
//!
//! # Profiles & Adapter (Gate 7)
//!
//! * [`profile`] — [`PlatformProfile`] constants describing desktop,
//!   Android, and iOS runtime capabilities (JIT/AOT, texture limits, …).
//! * [`adapter`] — [`PlatformAdapter`] trait for lifecycle, IME,
//!   safe-area, low-memory, and touch events, plus [`TouchEvent`] /
//!   [`TouchPhase`].

use std::{
    fmt,
    marker::PhantomData,
    num::{NonZeroIsize, NonZeroU32, NonZeroUsize},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent as WinitWindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

// ── Gate 7: Platform profiles & adapter ──────────────────────────────────
pub mod adapter;
pub mod profile;

pub use adapter::{PlatformAdapter, TouchEvent, TouchPhase};
pub use profile::{PlatformFamily, PlatformProfile, ANDROID_PROFILE, DESKTOP_PROFILE, IOS_PROFILE};

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
    Focused(bool),
    Redraw,
    CloseRequested,

    // ── Keyboard ──
    KeyPressed { key: KeyCode, modifiers: Modifiers },
    KeyReleased { key: KeyCode, modifiers: Modifiers },

    // ── Mouse ──
    MouseMoved { x: f64, y: f64 },
    MousePressed { button: MouseButton, x: f64, y: f64 },
    MouseReleased { button: MouseButton, x: f64, y: f64 },
    MouseWheelScrolled { delta: (f32, f32) },

    // ── Text input ──
    CharacterTyped { character: char },
    FileDropped { path: PathBuf },
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
    fn from_winit(mods: &winit::keyboard::ModifiersState) -> Self {
        Self {
            shift: mods.shift_key(),
            ctrl: mods.control_key(),
            alt: mods.alt_key(),
            logo: mods.super_key(),
        }
    }
}

pub use self::input_types::KeyCode;
mod input_types {
    /// Key codes (subset of winit's VirtualKeyCode for engine use).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum KeyCode {
        Escape,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
        Key0,
        Key1,
        Key2,
        Key3,
        Key4,
        Key5,
        Key6,
        Key7,
        Key8,
        Key9,
        A,
        B,
        C,
        D,
        E,
        F,
        G,
        H,
        I,
        J,
        K,
        L,
        M,
        N,
        O,
        P,
        Q,
        R,
        S,
        T,
        U,
        V,
        W,
        X,
        Y,
        Z,
        Space,
        Enter,
        Backspace,
        Tab,
        Delete,
        Left,
        Right,
        Up,
        Down,
        LShift,
        RShift,
        LControl,
        RControl,
        LAlt,
        RAlt,
        Other(u32),
    }
}

fn key_code_from_winit(k: winit::keyboard::KeyCode) -> KeyCode {
    match k {
        winit::keyboard::KeyCode::Escape => KeyCode::Escape,
        winit::keyboard::KeyCode::F1 => KeyCode::F1,
        winit::keyboard::KeyCode::F2 => KeyCode::F2,
        winit::keyboard::KeyCode::F3 => KeyCode::F3,
        winit::keyboard::KeyCode::F4 => KeyCode::F4,
        winit::keyboard::KeyCode::F5 => KeyCode::F5,
        winit::keyboard::KeyCode::F6 => KeyCode::F6,
        winit::keyboard::KeyCode::F7 => KeyCode::F7,
        winit::keyboard::KeyCode::F8 => KeyCode::F8,
        winit::keyboard::KeyCode::F9 => KeyCode::F9,
        winit::keyboard::KeyCode::F10 => KeyCode::F10,
        winit::keyboard::KeyCode::F11 => KeyCode::F11,
        winit::keyboard::KeyCode::F12 => KeyCode::F12,
        winit::keyboard::KeyCode::Digit0 => KeyCode::Key0,
        winit::keyboard::KeyCode::Digit1 => KeyCode::Key1,
        winit::keyboard::KeyCode::Digit2 => KeyCode::Key2,
        winit::keyboard::KeyCode::Digit3 => KeyCode::Key3,
        winit::keyboard::KeyCode::Digit4 => KeyCode::Key4,
        winit::keyboard::KeyCode::Digit5 => KeyCode::Key5,
        winit::keyboard::KeyCode::Digit6 => KeyCode::Key6,
        winit::keyboard::KeyCode::Digit7 => KeyCode::Key7,
        winit::keyboard::KeyCode::Digit8 => KeyCode::Key8,
        winit::keyboard::KeyCode::Digit9 => KeyCode::Key9,
        winit::keyboard::KeyCode::KeyA => KeyCode::A,
        winit::keyboard::KeyCode::KeyB => KeyCode::B,
        winit::keyboard::KeyCode::KeyC => KeyCode::C,
        winit::keyboard::KeyCode::KeyD => KeyCode::D,
        winit::keyboard::KeyCode::KeyE => KeyCode::E,
        winit::keyboard::KeyCode::KeyF => KeyCode::F,
        winit::keyboard::KeyCode::KeyG => KeyCode::G,
        winit::keyboard::KeyCode::KeyH => KeyCode::H,
        winit::keyboard::KeyCode::KeyI => KeyCode::I,
        winit::keyboard::KeyCode::KeyJ => KeyCode::J,
        winit::keyboard::KeyCode::KeyK => KeyCode::K,
        winit::keyboard::KeyCode::KeyL => KeyCode::L,
        winit::keyboard::KeyCode::KeyM => KeyCode::M,
        winit::keyboard::KeyCode::KeyN => KeyCode::N,
        winit::keyboard::KeyCode::KeyO => KeyCode::O,
        winit::keyboard::KeyCode::KeyP => KeyCode::P,
        winit::keyboard::KeyCode::KeyQ => KeyCode::Q,
        winit::keyboard::KeyCode::KeyR => KeyCode::R,
        winit::keyboard::KeyCode::KeyS => KeyCode::S,
        winit::keyboard::KeyCode::KeyT => KeyCode::T,
        winit::keyboard::KeyCode::KeyU => KeyCode::U,
        winit::keyboard::KeyCode::KeyV => KeyCode::V,
        winit::keyboard::KeyCode::KeyW => KeyCode::W,
        winit::keyboard::KeyCode::KeyX => KeyCode::X,
        winit::keyboard::KeyCode::KeyY => KeyCode::Y,
        winit::keyboard::KeyCode::KeyZ => KeyCode::Z,
        winit::keyboard::KeyCode::Space => KeyCode::Space,
        winit::keyboard::KeyCode::Enter | winit::keyboard::KeyCode::NumpadEnter => KeyCode::Enter,
        winit::keyboard::KeyCode::Backspace => KeyCode::Backspace,
        winit::keyboard::KeyCode::Tab => KeyCode::Tab,
        winit::keyboard::KeyCode::Delete => KeyCode::Delete,
        winit::keyboard::KeyCode::ArrowLeft => KeyCode::Left,
        winit::keyboard::KeyCode::ArrowRight => KeyCode::Right,
        winit::keyboard::KeyCode::ArrowUp => KeyCode::Up,
        winit::keyboard::KeyCode::ArrowDown => KeyCode::Down,
        winit::keyboard::KeyCode::ShiftLeft => KeyCode::LShift,
        winit::keyboard::KeyCode::ShiftRight => KeyCode::RShift,
        winit::keyboard::KeyCode::ControlLeft => KeyCode::LControl,
        winit::keyboard::KeyCode::ControlRight => KeyCode::RControl,
        winit::keyboard::KeyCode::AltLeft => KeyCode::LAlt,
        winit::keyboard::KeyCode::AltRight => KeyCode::RAlt,
        other => KeyCode::Other(other as u32),
    }
}

/// Opaque owner for the native desktop window.
///
/// Consumers can query the drawable size and request redraws, but cannot reach
/// the underlying winit object. Graphics backends receive a separate
/// [`PlatformSurface`] through the platform-owned adapter seam.
#[derive(Clone)]
pub struct PlatformWindow {
    inner: Arc<Window>,
}

impl fmt::Debug for PlatformWindow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlatformWindow")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl PlatformWindow {
    fn new(window: Arc<Window>) -> Self {
        Self { inner: window }
    }

    pub fn size(&self) -> (u32, u32) {
        let size = self.inner.inner_size();
        (size.width, size.height)
    }

    pub fn request_redraw(&self) {
        self.inner.request_redraw();
    }

    pub fn surface(&self) -> Result<PlatformSurface, PlatformError> {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

        let display_handle = self
            .inner
            .display_handle()
            .map_err(|error| PlatformError::NativeHandle(error.to_string()))?
            .as_raw();
        let window_handle = self
            .inner
            .window_handle()
            .map_err(|error| PlatformError::NativeHandle(error.to_string()))?
            .as_raw();
        PlatformSurface::from_raw_handles(display_handle, window_handle)
    }
}

/// Opaque copy of a native presentation surface identity.
///
/// The window or editor-host surface that produced this value must remain
/// alive while a graphics backend uses it. Only platform adapters and concrete
/// backends should construct or consume this type.
#[derive(Clone, Copy)]
pub struct PlatformSurface {
    snapshot: PlatformSurfaceSnapshot,
    // Native window handles are generally UI-thread-bound. Preserve that
    // invariant even though the public snapshot stores pointer values as
    // integer tokens.
    _thread_bound: PhantomData<Rc<()>>,
}

impl fmt::Debug for PlatformSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PlatformSurface(..)")
    }
}

impl PlatformSurface {
    fn from_raw_handles(
        display_handle: RawDisplayHandle,
        window_handle: RawWindowHandle,
    ) -> Result<Self, PlatformError> {
        let snapshot = snapshot_from_raw_handles(display_handle, window_handle)?;
        Ok(Self::from_snapshot(snapshot))
    }

    /// Build an opaque surface from a platform-owned snapshot.
    ///
    /// This constructor is for native host adapters such as
    /// `engine-editor-host`; ordinary subsystems should only forward a surface
    /// received in an event.
    pub fn from_snapshot(snapshot: PlatformSurfaceSnapshot) -> Self {
        Self {
            snapshot,
            _thread_bound: PhantomData,
        }
    }

    /// Hand this surface to a concrete backend without exposing native
    /// handles to the application composition root.
    pub fn create_with<F>(self, factory: F) -> F::Output
    where
        F: PlatformSurfaceFactory,
    {
        factory.create_for_platform_surface(self.snapshot)
    }
}

/// Platform-owned, backend-neutral snapshot of a presentation surface.
///
/// Pointer-shaped values are kept in non-zero integer tokens so neither winit
/// nor raw-window-handle types appear in the public platform contract.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformSurfaceSnapshot {
    Win32 {
        hwnd: NonZeroIsize,
        hinstance: Option<NonZeroIsize>,
    },
    WinRt {
        core_window: NonZeroUsize,
    },
    AppKit {
        ns_view: NonZeroUsize,
    },
    UiKit {
        ui_view: NonZeroUsize,
        ui_view_controller: Option<NonZeroUsize>,
    },
    Xlib {
        display: Option<NonZeroUsize>,
        screen: i32,
        window: u64,
        visual_id: u64,
    },
    Xcb {
        connection: Option<NonZeroUsize>,
        screen: i32,
        window: NonZeroU32,
        visual_id: Option<NonZeroU32>,
    },
    Wayland {
        display: NonZeroUsize,
        surface: NonZeroUsize,
    },
    Drm {
        fd: i32,
        plane: u32,
    },
    Gbm {
        device: NonZeroUsize,
        surface: NonZeroUsize,
    },
    AndroidNdk {
        native_window: NonZeroUsize,
    },
    OhosNdk {
        native_window: NonZeroUsize,
    },
    Haiku {
        window: NonZeroUsize,
        direct_window: Option<NonZeroUsize>,
    },
    Orbital {
        window: NonZeroUsize,
    },
    Web {
        id: u32,
    },
    WebCanvas {
        object: NonZeroUsize,
    },
    WebOffscreenCanvas {
        object: NonZeroUsize,
    },
}

/// Concrete graphics-backend seam for an opaque [`PlatformSurface`].
///
/// Implementations belong in backend crates. Application and subsystem crates
/// pass the opaque surface through without importing raw-window-handle.
pub trait PlatformSurfaceFactory {
    type Output;

    fn create_for_platform_surface(self, surface: PlatformSurfaceSnapshot) -> Self::Output;
}

fn pointer_token(pointer: std::ptr::NonNull<std::ffi::c_void>) -> NonZeroUsize {
    NonZeroUsize::new(pointer.as_ptr() as usize).expect("native pointers are non-null")
}

fn snapshot_from_raw_handles(
    display: RawDisplayHandle,
    window: RawWindowHandle,
) -> Result<PlatformSurfaceSnapshot, PlatformError> {
    let snapshot = match (display, window) {
        (RawDisplayHandle::Windows(_), RawWindowHandle::Win32(window)) => {
            PlatformSurfaceSnapshot::Win32 {
                hwnd: window.hwnd,
                hinstance: window.hinstance,
            }
        }
        (RawDisplayHandle::Windows(_), RawWindowHandle::WinRt(window)) => {
            PlatformSurfaceSnapshot::WinRt {
                core_window: pointer_token(window.core_window),
            }
        }
        (RawDisplayHandle::AppKit(_), RawWindowHandle::AppKit(window)) => {
            PlatformSurfaceSnapshot::AppKit {
                ns_view: pointer_token(window.ns_view),
            }
        }
        (RawDisplayHandle::UiKit(_), RawWindowHandle::UiKit(window)) => {
            PlatformSurfaceSnapshot::UiKit {
                ui_view: pointer_token(window.ui_view),
                ui_view_controller: window.ui_view_controller.map(pointer_token),
            }
        }
        (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => {
            PlatformSurfaceSnapshot::Xlib {
                display: display.display.map(pointer_token),
                screen: display.screen,
                window: window.window as u64,
                visual_id: window.visual_id as u64,
            }
        }
        (RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(window)) => {
            PlatformSurfaceSnapshot::Xcb {
                connection: display.connection.map(pointer_token),
                screen: display.screen,
                window: window.window,
                visual_id: window.visual_id,
            }
        }
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => {
            PlatformSurfaceSnapshot::Wayland {
                display: pointer_token(display.display),
                surface: pointer_token(window.surface),
            }
        }
        (RawDisplayHandle::Drm(display), RawWindowHandle::Drm(window)) => {
            PlatformSurfaceSnapshot::Drm {
                fd: display.fd,
                plane: window.plane,
            }
        }
        (RawDisplayHandle::Gbm(display), RawWindowHandle::Gbm(window)) => {
            PlatformSurfaceSnapshot::Gbm {
                device: pointer_token(display.gbm_device),
                surface: pointer_token(window.gbm_surface),
            }
        }
        (RawDisplayHandle::Android(_), RawWindowHandle::AndroidNdk(window)) => {
            PlatformSurfaceSnapshot::AndroidNdk {
                native_window: pointer_token(window.a_native_window),
            }
        }
        (RawDisplayHandle::Ohos(_), RawWindowHandle::OhosNdk(window)) => {
            PlatformSurfaceSnapshot::OhosNdk {
                native_window: pointer_token(window.native_window),
            }
        }
        (RawDisplayHandle::Haiku(_), RawWindowHandle::Haiku(window)) => {
            PlatformSurfaceSnapshot::Haiku {
                window: pointer_token(window.b_window),
                direct_window: window.b_direct_window.map(pointer_token),
            }
        }
        (RawDisplayHandle::Orbital(_), RawWindowHandle::Orbital(window)) => {
            PlatformSurfaceSnapshot::Orbital {
                window: pointer_token(window.window),
            }
        }
        (RawDisplayHandle::Web(_), RawWindowHandle::Web(window)) => {
            PlatformSurfaceSnapshot::Web { id: window.id }
        }
        (RawDisplayHandle::Web(_), RawWindowHandle::WebCanvas(window)) => {
            PlatformSurfaceSnapshot::WebCanvas {
                object: pointer_token(window.obj),
            }
        }
        (RawDisplayHandle::Web(_), RawWindowHandle::WebOffscreenCanvas(window)) => {
            PlatformSurfaceSnapshot::WebOffscreenCanvas {
                object: pointer_token(window.obj),
            }
        }
        (display, window) => {
            return Err(PlatformError::UnsupportedSurfacePair {
                display: format!("{display:?}"),
                window: format!("{window:?}"),
            });
        }
    };
    Ok(snapshot)
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
    #[error("native window handle acquisition failed: {0}")]
    NativeHandle(String),
    #[error("unsupported native display/window handle pair: display={display}, window={window}")]
    UnsupportedSurfacePair { display: String, window: String },
}

/// Callback surface implemented by the consumer (sandbox / renderer).
///
/// `on_create` is invoked exactly once with the newly created window so
/// the consumer can build GPU resources tied to the window's raw handle.
pub trait WindowApp: 'static {
    fn on_create(&mut self, window: &PlatformWindow);
    fn on_event(&mut self, window: &PlatformWindow, event: PlatformEvent) -> EventFlow;
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
        modifiers: Modifiers::default(),
        cursor_position: (0.0, 0.0),
    };
    event_loop
        .run_app(&mut wrapper)
        .map_err(|e| PlatformError::EventLoop(e.to_string()))
}

struct Wrapper<A: WindowApp> {
    descriptor: WindowDescriptor,
    app: A,
    window: Option<PlatformWindow>,
    created: bool,
    modifiers: Modifiers,
    cursor_position: (f64, f64),
}

fn character_events(text: &str) -> Vec<PlatformEvent> {
    text.chars()
        .filter(|character| !character.is_control())
        .map(|character| PlatformEvent::CharacterTyped { character })
        .collect()
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
                    window.set_ime_allowed(true);
                    let window = PlatformWindow::new(window);
                    self.window = Some(window.clone());
                    if !self.created {
                        self.app.on_create(&window);
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
        for ev in self.translate_events(&event) {
            let flow = self.app.on_event(&window, ev);
            if matches!(flow, EventFlow::Exit) {
                event_loop.exit();
                break;
            }
        }
        window.request_redraw();
    }
}

impl<A: WindowApp> Wrapper<A> {
    fn translate_events(&mut self, event: &WinitWindowEvent) -> Vec<PlatformEvent> {
        use winit::event::ElementState as WinitState;
        use winit::event::MouseScrollDelta;
        match event {
            WinitWindowEvent::Resized(size) => vec![PlatformEvent::Resized {
                width: size.width,
                height: size.height,
            }],
            WinitWindowEvent::Focused(focused) => vec![PlatformEvent::Focused(*focused)],
            WinitWindowEvent::CloseRequested => vec![PlatformEvent::CloseRequested],
            WinitWindowEvent::RedrawRequested => vec![PlatformEvent::Redraw],
            WinitWindowEvent::DroppedFile(path) => {
                vec![PlatformEvent::FileDropped { path: path.clone() }]
            }
            WinitWindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = Modifiers::from_winit(&modifiers.state());
                Vec::new()
            }

            // ── Keyboard ──────────────────────────────────────────────
            WinitWindowEvent::KeyboardInput { event: ke, .. } => {
                let key = match ke.physical_key {
                    winit::keyboard::PhysicalKey::Code(code) => key_code_from_winit(code),
                    winit::keyboard::PhysicalKey::Unidentified(_) => KeyCode::Other(u32::MAX),
                };
                match ke.state {
                    WinitState::Pressed => {
                        let mut events = vec![PlatformEvent::KeyPressed {
                            key,
                            modifiers: self.modifiers,
                        }];
                        if !self.modifiers.ctrl && !self.modifiers.logo {
                            if let Some(text) = &ke.text {
                                events.extend(character_events(text));
                            }
                        }
                        events
                    }
                    WinitState::Released => vec![PlatformEvent::KeyReleased {
                        key,
                        modifiers: self.modifiers,
                    }],
                }
            }
            WinitWindowEvent::Ime(winit::event::Ime::Commit(text)) => character_events(text),

            // ── Mouse ─────────────────────────────────────────────────
            WinitWindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);
                vec![PlatformEvent::MouseMoved {
                    x: position.x,
                    y: position.y,
                }]
            }
            WinitWindowEvent::MouseInput { state, button, .. } => {
                let btn = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    _ => MouseButton::Other(0),
                };
                match state {
                    WinitState::Pressed => vec![PlatformEvent::MousePressed {
                        button: btn,
                        x: self.cursor_position.0,
                        y: self.cursor_position.1,
                    }],
                    WinitState::Released => vec![PlatformEvent::MouseReleased {
                        button: btn,
                        x: self.cursor_position.0,
                        y: self.cursor_position.1,
                    }],
                }
            }
            WinitWindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
                };
                vec![PlatformEvent::MouseWheelScrolled { delta: (dx, dy) }]
            }

            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
