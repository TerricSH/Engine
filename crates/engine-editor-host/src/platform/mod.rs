use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use tao::window::Window;
use wry::{WebView, WebViewBuilder};

use crate::HostError;

#[cfg(target_os = "linux")]
mod linux;

pub(crate) struct NativeSurface {
    pub(crate) window_handle: RawWindowHandle,
    pub(crate) display_handle: RawDisplayHandle,
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
    Ok(NativeSurface {
        window_handle: window.window_handle()?.as_raw(),
        display_handle: window.display_handle()?.as_raw(),
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn prepare(window: &Window) -> Result<NativeSurface, HostError> {
    let linux = linux::LinuxSurface::new(window)?;
    let (window_handle, display_handle) = linux.raw_handles()?;
    Ok(NativeSurface {
        window_handle,
        display_handle,
        linux,
    })
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
