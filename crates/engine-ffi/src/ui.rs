//! UI canvas FFI — forwarding layer.
//!
//! These `#[no_mangle] extern "C"` functions are the C# entry points for
//! runtime UI.  They delegate to `engine-ui`'s Canvas API.
//!
//! # Safety policy
//!
//! Every function that accepts a raw `canvas` pointer documents its safety
//! contract with a `// SAFETY:` comment.  Null pointers are handled
//! gracefully (no-op / zero / `INVALID` return).

use std::ffi::CStr;
use std::os::raw::c_char;

use engine_ui::Color;
use engine_ui::{Canvas, ElementId, Layout, ScaleMode, UiElement, UiElementKind};

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create a new UI canvas with the given logical dimensions.
///
/// Returns an opaque pointer that must be freed via [`ui_canvas_destroy`].
///
/// # Safety
/// `canvas` is an output parameter that receives the newly created canvas
/// pointer.  The caller is responsible for calling `ui_canvas_destroy`.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_create(w: f32, h: f32) -> *mut std::ffi::c_void {
    let canvas = Box::new(Canvas::new(w, h));
    Box::into_raw(canvas) as *mut std::ffi::c_void
}

/// Destroy a canvas previously created by [`ui_canvas_create`].
///
/// # Safety
/// `canvas` must be a valid pointer returned by `ui_canvas_create`, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_destroy(canvas: *mut std::ffi::c_void) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    drop(Box::from_raw(canvas as *mut Canvas));
}

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

/// Add a button element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
/// `label` must be a valid null-terminated UTF-8 string, or null.
/// `callback_id` may be null (no callback) or a valid null-terminated string.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_button(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: *const c_char,
    callback_id: *const c_char,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);

    let label_str = if label.is_null() {
        String::new()
    } else {
        // SAFETY: Caller guarantees a valid null-terminated string.
        CStr::from_ptr(label).to_string_lossy().into_owned()
    };

    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );

    let cid = if callback_id.is_null() {
        None
    } else {
        let s = CStr::from_ptr(callback_id).to_string_lossy().into_owned();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };

    let element = UiElement::new(
        UiElementKind::Button {
            label: label_str,
            normal_color: Color::new(180, 180, 200, 255),
            hover_color: Color::new(200, 200, 220, 255),
            pressed_color: Color::new(140, 140, 160, 255),
            callback_id: cid,
        },
        layout,
    );

    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add a toggle element to the canvas.
///
/// `is_on`: 1 for on, 0 for off.
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_toggle(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: *const c_char,
    is_on: i32,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    let canvas = &mut *(canvas as *mut Canvas);
    let label_str = if label.is_null() {
        String::new()
    } else {
        CStr::from_ptr(label).to_string_lossy().into_owned()
    };
    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );
    let element = UiElement::new(
        UiElementKind::Toggle {
            label: label_str,
            is_on: is_on != 0,
            color_on: Color::new(100, 200, 100, 255),
            color_off: Color::new(100, 100, 100, 255),
            callback_id: None,
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add a checkbox element to the canvas.
///
/// `checked`: 1 for checked, 0 for unchecked.
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_checkbox(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: *const c_char,
    checked: i32,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    let canvas = &mut *(canvas as *mut Canvas);
    let label_str = if label.is_null() {
        String::new()
    } else {
        CStr::from_ptr(label).to_string_lossy().into_owned()
    };
    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );
    let element = UiElement::new(
        UiElementKind::Checkbox {
            label: label_str,
            checked: checked != 0,
            color: Color::new(180, 180, 180, 255),
            callback_id: None,
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add a slider element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_slider(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: *const c_char,
    value: f32,
    min: f32,
    max: f32,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    let canvas = &mut *(canvas as *mut Canvas);
    let label_str = if label.is_null() {
        String::new()
    } else {
        CStr::from_ptr(label).to_string_lossy().into_owned()
    };
    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );
    let element = UiElement::new(
        UiElementKind::Slider {
            label: label_str,
            value,
            min,
            max,
            callback_id: None,
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add a scroll-view element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_scroll_view(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    let canvas = &mut *(canvas as *mut Canvas);
    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );
    let element = UiElement::new(
        UiElementKind::ScrollView {
            scroll_x: 0.0,
            scroll_y: 0.0,
            content_width: w,
            content_height: h,
            color: Color::new(50, 50, 50, 255),
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Set the text content of a text element.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
/// `text` must be a valid null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_set_text(
    canvas: *mut std::ffi::c_void,
    element_id: u64,
    text: *const c_char,
) {
    if canvas.is_null() || text.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees valid pointers or null.
    let canvas = &mut *(canvas as *mut Canvas);
    let text_str = CStr::from_ptr(text).to_string_lossy().into_owned();
    let id = ElementId(element_id as u32);

    if let Some(el) = canvas.get_element_mut(id) {
        if let UiElementKind::Text { content, .. } = &mut el.kind {
            *content = text_str;
        }
    }
}

/// Enable or disable an element.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_element_set_enabled(
    canvas: *mut std::ffi::c_void,
    element_id: u64,
    enabled: bool,
) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    let id = ElementId(element_id as u32);

    if let Some(el) = canvas.get_element_mut(id) {
        el.enabled = enabled;
    }
}

/// Add a panel element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_panel(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);

    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );

    let element = UiElement::new(
        UiElementKind::Panel {
            color: Color::new(r, g, b, a),
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add a text element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
/// `text` must be a valid null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_text(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: *const c_char,
    font_size: f32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> u64 {
    if canvas.is_null() || text.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    // SAFETY: Null-checked above; caller guarantees valid pointers or null.
    let canvas = &mut *(canvas as *mut Canvas);
    let text_str = CStr::from_ptr(text).to_string_lossy().into_owned();

    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );

    let element = UiElement::new(
        UiElementKind::Text {
            content: text_str,
            font_size,
            color: Color::new(r, g, b, a),
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Add an image element to the canvas.
///
/// Returns the assigned [`ElementId`] as a `u64`.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
/// `texture_id` must be a valid null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_add_image(
    canvas: *mut std::ffi::c_void,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    texture_id: *const c_char,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) -> u64 {
    if canvas.is_null() || texture_id.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    // SAFETY: Null-checked above; caller guarantees valid pointers or null.
    let canvas = &mut *(canvas as *mut Canvas);
    let texture = CStr::from_ptr(texture_id).to_string_lossy().into_owned();

    let layout = Layout::new(
        glam::Vec2::ZERO,
        glam::Vec2::ZERO,
        glam::Vec2::new(x, y),
        glam::Vec2::new(x + w, y + h),
    );

    let element = UiElement::new(
        UiElementKind::Image {
            texture_id: texture,
            color: Color::new(r, g, b, a),
        },
        layout,
    );
    let id = canvas.add_element(element);
    id.0 as u64
}

/// Remove an element from the canvas.
///
/// Returns 1 if the element was found and removed, 0 otherwise.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_remove_element(
    canvas: *mut std::ffi::c_void,
    element_id: u64,
) -> i32 {
    if canvas.is_null() {
        return 0;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    if canvas.remove_element(ElementId(element_id as u32)) {
        1
    } else {
        0
    }
}

/// Clear all elements from the canvas.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_clear(canvas: *mut std::ffi::c_void) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    canvas.clear();
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Re-compute all element positions from their layouts.
///
/// Must be called after adding/moving elements and before rendering.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_layout(canvas: *mut std::ffi::c_void) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    canvas.layout_all();
}

/// Resize the canvas.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_resize(canvas: *mut std::ffi::c_void, width: f32, height: f32) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    canvas.resize(width, height);
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Returns the element at the given position, or `u64::MAX` (ElementId::INVALID)
/// if nothing is found.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_hit_test(
    canvas: *const std::ffi::c_void,
    px: f32,
    py: f32,
) -> u64 {
    if canvas.is_null() {
        return ElementId::INVALID.0 as u64;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &*(canvas as *const Canvas);
    match engine_ui::hit_test(canvas, px, py) {
        Some(id) => id.0 as u64,
        None => ElementId::INVALID.0 as u64,
    }
}

/// Set the scale mode of the canvas.
///
/// # Safety
/// `canvas` must be a valid pointer to a Canvas, or null.
#[no_mangle]
pub unsafe extern "C" fn ui_canvas_set_scale_mode(canvas: *mut std::ffi::c_void, mode: i32) {
    if canvas.is_null() {
        return;
    }
    // SAFETY: Null-checked above; caller guarantees a valid Canvas or null.
    let canvas = &mut *(canvas as *mut Canvas);
    canvas.scale_mode = match mode {
        1 => ScaleMode::FitWidth,
        2 => ScaleMode::FitHeight,
        _ => ScaleMode::Fixed,
    };
}
