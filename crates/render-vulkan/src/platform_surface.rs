use std::{ffi::c_void, path::Path, ptr::NonNull};

use platform::{PlatformSurface, PlatformSurfaceFactory, PlatformSurfaceSnapshot};
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, AppKitDisplayHandle, AppKitWindowHandle,
    DrmDisplayHandle, DrmWindowHandle, GbmDisplayHandle, GbmWindowHandle, HaikuDisplayHandle,
    HaikuWindowHandle, OhosDisplayHandle, OhosNdkWindowHandle, OrbitalDisplayHandle,
    OrbitalWindowHandle, RawDisplayHandle, RawWindowHandle, UiKitDisplayHandle, UiKitWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle, WebCanvasWindowHandle, WebDisplayHandle,
    WebOffscreenCanvasWindowHandle, WebWindowHandle, Win32WindowHandle, WinRtWindowHandle,
    WindowsDisplayHandle, XcbDisplayHandle, XcbWindowHandle, XlibDisplayHandle, XlibWindowHandle,
};

use crate::{create_backend_renderer, VulkanError};

pub(crate) fn create_renderer(
    surface: PlatformSurface,
    width: u32,
    height: u32,
    enable_validation: bool,
    cache_dir: Option<&Path>,
) -> Result<Box<dyn engine_renderer::BackendRenderer>, VulkanError> {
    surface.create_with(VulkanSurfaceFactory {
        width,
        height,
        enable_validation,
        cache_dir,
    })
}

struct VulkanSurfaceFactory<'a> {
    width: u32,
    height: u32,
    enable_validation: bool,
    cache_dir: Option<&'a Path>,
}

impl PlatformSurfaceFactory for VulkanSurfaceFactory<'_> {
    type Output = Result<Box<dyn engine_renderer::BackendRenderer>, VulkanError>;

    fn create_for_platform_surface(self, surface: PlatformSurfaceSnapshot) -> Self::Output {
        let (display, window) = raw_handles(surface)?;
        create_backend_renderer(
            display,
            window,
            self.width,
            self.height,
            self.enable_validation,
            self.cache_dir,
        )
    }
}

fn pointer(token: std::num::NonZeroUsize) -> NonNull<c_void> {
    NonNull::new(token.get() as *mut c_void).expect("platform pointer tokens are non-zero")
}

#[allow(clippy::too_many_lines)]
fn raw_handles(
    surface: PlatformSurfaceSnapshot,
) -> Result<(RawDisplayHandle, RawWindowHandle), VulkanError> {
    let handles = match surface {
        PlatformSurfaceSnapshot::Win32 { hwnd, hinstance } => {
            let mut window = Win32WindowHandle::new(hwnd);
            window.hinstance = hinstance;
            (
                RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                RawWindowHandle::Win32(window),
            )
        }
        PlatformSurfaceSnapshot::WinRt { core_window } => (
            RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
            RawWindowHandle::WinRt(WinRtWindowHandle::new(pointer(core_window))),
        ),
        PlatformSurfaceSnapshot::AppKit { ns_view } => (
            RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
            RawWindowHandle::AppKit(AppKitWindowHandle::new(pointer(ns_view))),
        ),
        PlatformSurfaceSnapshot::UiKit {
            ui_view,
            ui_view_controller,
        } => {
            let mut window = UiKitWindowHandle::new(pointer(ui_view));
            window.ui_view_controller = ui_view_controller.map(pointer);
            (
                RawDisplayHandle::UiKit(UiKitDisplayHandle::new()),
                RawWindowHandle::UiKit(window),
            )
        }
        PlatformSurfaceSnapshot::Xlib {
            display,
            screen,
            window,
            visual_id,
        } => {
            let display = XlibDisplayHandle::new(display.map(pointer), screen);
            let mut window_handle = XlibWindowHandle::new(window as _);
            window_handle.visual_id = visual_id as _;
            (
                RawDisplayHandle::Xlib(display),
                RawWindowHandle::Xlib(window_handle),
            )
        }
        PlatformSurfaceSnapshot::Xcb {
            connection,
            screen,
            window,
            visual_id,
        } => {
            let display = XcbDisplayHandle::new(connection.map(pointer), screen);
            let mut window_handle = XcbWindowHandle::new(window);
            window_handle.visual_id = visual_id;
            (
                RawDisplayHandle::Xcb(display),
                RawWindowHandle::Xcb(window_handle),
            )
        }
        PlatformSurfaceSnapshot::Wayland { display, surface } => (
            RawDisplayHandle::Wayland(WaylandDisplayHandle::new(pointer(display))),
            RawWindowHandle::Wayland(WaylandWindowHandle::new(pointer(surface))),
        ),
        PlatformSurfaceSnapshot::Drm { fd, plane } => (
            RawDisplayHandle::Drm(DrmDisplayHandle::new(fd)),
            RawWindowHandle::Drm(DrmWindowHandle::new(plane)),
        ),
        PlatformSurfaceSnapshot::Gbm { device, surface } => (
            RawDisplayHandle::Gbm(GbmDisplayHandle::new(pointer(device))),
            RawWindowHandle::Gbm(GbmWindowHandle::new(pointer(surface))),
        ),
        PlatformSurfaceSnapshot::AndroidNdk { native_window } => (
            RawDisplayHandle::Android(AndroidDisplayHandle::new()),
            RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(pointer(native_window))),
        ),
        PlatformSurfaceSnapshot::OhosNdk { native_window } => (
            RawDisplayHandle::Ohos(OhosDisplayHandle::new()),
            RawWindowHandle::OhosNdk(OhosNdkWindowHandle::new(pointer(native_window))),
        ),
        PlatformSurfaceSnapshot::Haiku {
            window,
            direct_window,
        } => {
            let mut window_handle = HaikuWindowHandle::new(pointer(window));
            window_handle.b_direct_window = direct_window.map(pointer);
            (
                RawDisplayHandle::Haiku(HaikuDisplayHandle::new()),
                RawWindowHandle::Haiku(window_handle),
            )
        }
        PlatformSurfaceSnapshot::Orbital { window } => (
            RawDisplayHandle::Orbital(OrbitalDisplayHandle::new()),
            RawWindowHandle::Orbital(OrbitalWindowHandle::new(pointer(window))),
        ),
        PlatformSurfaceSnapshot::Web { id } => (
            RawDisplayHandle::Web(WebDisplayHandle::new()),
            RawWindowHandle::Web(WebWindowHandle::new(id)),
        ),
        PlatformSurfaceSnapshot::WebCanvas { object } => (
            RawDisplayHandle::Web(WebDisplayHandle::new()),
            RawWindowHandle::WebCanvas(WebCanvasWindowHandle::new(pointer(object))),
        ),
        PlatformSurfaceSnapshot::WebOffscreenCanvas { object } => (
            RawDisplayHandle::Web(WebDisplayHandle::new()),
            RawWindowHandle::WebOffscreenCanvas(WebOffscreenCanvasWindowHandle::new(pointer(
                object,
            ))),
        ),
        _ => {
            return Err(VulkanError::UnsupportedWindow(
                "unknown platform surface snapshot",
            ))
        }
    };
    Ok(handles)
}
