use std::num::NonZeroUsize;

use ::platform::{PlatformSurface, PlatformSurfaceSnapshot};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tao::window::Window;
use wry::{WebView, WebViewBuilder};

use crate::HostError;

#[cfg(target_os = "linux")]
mod linux;

pub(crate) struct NativeSurface {
    pub(crate) surface: PlatformSurface,
    #[cfg(target_os = "linux")]
    linux: linux::LinuxSurface,
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn initialize() -> Result<(), HostError> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn initialize() -> Result<(), HostError> {
    linux::initialize()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn prepare(window: &Window) -> Result<NativeSurface, HostError> {
    let surface = surface_from_raw(
        window.display_handle()?.as_raw(),
        window.window_handle()?.as_raw(),
    )?;
    Ok(NativeSurface { surface })
}

#[cfg(target_os = "linux")]
pub(crate) fn prepare(window: &Window) -> Result<NativeSurface, HostError> {
    let linux = linux::LinuxSurface::new(window)?;
    let (window_handle, display_handle) = linux.raw_handles()?;
    let surface = surface_from_raw(display_handle, window_handle)?;
    Ok(NativeSurface { surface, linux })
}

fn pointer_token(pointer: std::ptr::NonNull<std::ffi::c_void>) -> NonZeroUsize {
    NonZeroUsize::new(pointer.as_ptr() as usize).expect("native pointers are non-null")
}

fn surface_from_raw(
    display: RawDisplayHandle,
    window: RawWindowHandle,
) -> Result<PlatformSurface, HostError> {
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
        (display, window) => {
            return Err(HostError::Platform(format!(
                "unsupported editor surface pair: display={display:?}, window={window:?}"
            )));
        }
    };
    Ok(PlatformSurface::from_snapshot(snapshot))
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn build_webview(
    builder: WebViewBuilder<'_>,
    window: &Window,
    _surface: &NativeSurface,
) -> Result<WebView, HostError> {
    builder.build_as_child(window).map_err(HostError::WebView)
}

#[cfg(target_os = "linux")]
pub(crate) fn build_webview(
    builder: WebViewBuilder<'_>,
    _window: &Window,
    surface: &NativeSurface,
) -> Result<WebView, HostError> {
    surface.linux.build_webview(builder)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn pump_events() {}

#[cfg(target_os = "linux")]
pub(crate) fn pump_events() {
    linux::pump_events();
}
