use std::{ffi::c_void, ptr::NonNull};

use gtk::{
    glib::{prelude::Cast, translate::ToGlibPtr},
    prelude::*,
};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
    XlibDisplayHandle, XlibWindowHandle,
};
use tao::{platform::unix::WindowExtUnix, window::Window};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix};

use crate::HostError;

pub(super) struct LinuxSurface {
    _overlay: gtk::Overlay,
    drawing_area: gtk::DrawingArea,
    webview_container: gtk::Fixed,
}

pub(super) fn initialize() -> Result<(), HostError> {
    gtk::init().map_err(|error| HostError::Platform(format!("GTK initialization failed: {error}")))
}

impl LinuxSurface {
    pub(super) fn new(window: &Window) -> Result<Self, HostError> {
        let overlay = gtk::Overlay::new();
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);

        let drawing_area = gtk::DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        drawing_area.set_app_paintable(true);
        overlay.add(&drawing_area);

        let webview_container = gtk::Fixed::new();
        webview_container.set_hexpand(true);
        webview_container.set_vexpand(true);
        overlay.add_overlay(&webview_container);

        let vbox = window.default_vbox().ok_or_else(|| {
            HostError::Platform("Tao did not expose its GTK root container".into())
        })?;
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        pump_events();

        if drawing_area.window().is_none() {
            drawing_area.realize();
            pump_events();
        }

        Ok(Self {
            _overlay: overlay,
            drawing_area,
            webview_container,
        })
    }

    pub(super) fn build_webview(&self, builder: WebViewBuilder<'_>) -> Result<WebView, HostError> {
        builder
            .build_gtk(&self.webview_container)
            .map_err(HostError::WebView)
    }

    pub(super) fn raw_handles(&self) -> Result<(RawWindowHandle, RawDisplayHandle), HostError> {
        let gdk_window = self
            .drawing_area
            .window()
            .ok_or_else(|| HostError::Platform("GTK DrawingArea was not realized".into()))?;
        let display = gdk_window.display();

        let backend = display.backend();
        if backend.is_x11() {
            return x11_raw_handles(&gdk_window, &display);
        }
        if backend.is_wayland() {
            return wayland_raw_handles(&gdk_window, &display);
        }

        Err(HostError::Platform(
            "GTK display backend is neither X11 nor Wayland".into(),
        ))
    }
}

fn x11_raw_handles(
    window: &gtk::gdk::Window,
    display: &gtk::gdk::Display,
) -> Result<(RawWindowHandle, RawDisplayHandle), HostError> {
    let x11_window = window
        .clone()
        .dynamic_cast::<gdkx11::X11Window>()
        .map_err(|_| HostError::Platform("failed to cast DrawingArea window to X11".into()))?;
    let x11_display = display
        .clone()
        .dynamic_cast::<gdkx11::X11Display>()
        .map_err(|_| HostError::Platform("failed to cast GTK display to X11".into()))?;
    let window_handle = XlibWindowHandle::new(x11_window.xid());

    // SAFETY INVARIANTS:
    // - `x11_display` is a live, strong GObject reference owned by GTK on this UI thread.
    // - GTK identified the backend as X11 and the dynamic cast above verified its concrete type.
    // - GDK owns the returned `Display*`; we never free it and retain the Tao window, Overlay and
    //   DrawingArea until after the renderer and WebView are dropped.
    // - `run_editor_host` never moves GTK objects to another thread. The raw handle is valid only
    //   for the duration of that host invocation, as documented on `HostEvent::SurfaceReady`.
    let xdisplay =
        unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(x11_display.to_glib_none().0) };
    let xdisplay = NonNull::new(xdisplay.cast::<c_void>())
        .ok_or_else(|| HostError::Platform("GDK returned a null X11 display".into()))?;
    let screen = display.default_screen().number();
    let display_handle = XlibDisplayHandle::new(Some(xdisplay), screen);

    Ok((
        RawWindowHandle::Xlib(window_handle),
        RawDisplayHandle::Xlib(display_handle),
    ))
}

fn wayland_raw_handles(
    window: &gtk::gdk::Window,
    display: &gtk::gdk::Display,
) -> Result<(RawWindowHandle, RawDisplayHandle), HostError> {
    // SAFETY INVARIANTS:
    // - GTK reported a Wayland backend before these casts are made.
    // - `window` is the realized DrawingArea's GdkWindow and `display` is its owning GdkDisplay.
    // - The FFI functions borrow the underlying `wl_surface` and `wl_display`; ownership remains
    //   with GTK. `LinuxSurface` keeps the DrawingArea and Overlay alive for the entire event loop.
    // - All access occurs on the UI thread. Consumers may use these handles to create a Vulkan
    //   surface during this host invocation, but must not retain them after `run_editor_host`
    //   returns.
    let (surface, wl_display) = unsafe {
        let gdk_window: *mut gtk::gdk::ffi::GdkWindow = window.to_glib_none().0;
        let gdk_display: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
        (
            gdkwayland_sys::gdk_wayland_window_get_wl_surface(gdk_window.cast()),
            gdkwayland_sys::gdk_wayland_display_get_wl_display(gdk_display.cast()),
        )
    };
    let surface = NonNull::new(surface.cast::<c_void>())
        .ok_or_else(|| HostError::Platform("GDK returned a null Wayland surface".into()))?;
    let wl_display = NonNull::new(wl_display.cast::<c_void>())
        .ok_or_else(|| HostError::Platform("GDK returned a null Wayland display".into()))?;

    Ok((
        RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
        RawDisplayHandle::Wayland(WaylandDisplayHandle::new(wl_display)),
    ))
}

pub(super) fn pump_events() {
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}
