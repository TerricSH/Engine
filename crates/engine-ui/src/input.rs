//! UI input handling — hit testing, hover detection, and click dispatch.
//!
//! The entry point is [`update_input`], which should be called once per frame
//! with the current pointer state.  It updates [`UiInputState`] and fires
//! callbacks for button clicks.

use tracing::debug;

use crate::types::{ElementId, UiElementKind};
use crate::Canvas;

// ---------------------------------------------------------------------------
// CallbackRegistry
// ---------------------------------------------------------------------------

/// A simple registry mapping callback IDs (strings) to closures.
///
/// The C# / scripting layer registers callbacks by ID.  When a button with a
/// matching `callback_id` is clicked, the closure is invoked.
#[derive(Default)]
pub struct CallbackRegistry {
    callbacks: std::collections::HashMap<String, Box<dyn FnMut() + Send>>,
}

impl CallbackRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            callbacks: std::collections::HashMap::new(),
        }
    }

    /// Register a callback under the given `id`.
    pub fn register(&mut self, id: impl Into<String>, callback: Box<dyn FnMut() + Send>) {
        self.callbacks.insert(id.into(), callback);
    }

    /// Unregister a callback by `id`.
    pub fn unregister(&mut self, id: &str) {
        self.callbacks.remove(id);
    }

    /// Invoke the callback identified by `id`, if one exists.
    ///
    /// Returns `true` if a callback was found and called.
    pub fn invoke(&mut self, id: &str) -> bool {
        if let Some(cb) = self.callbacks.get_mut(id) {
            cb();
            true
        } else {
            false
        }
    }

    /// Returns `true` if a callback is registered for the given ID.
    pub fn contains(&self, id: &str) -> bool {
        self.callbacks.contains_key(id)
    }
}

// ---------------------------------------------------------------------------
// UiInputState
// ---------------------------------------------------------------------------

/// Persistent input state for a canvas, updated once per frame.
///
/// This struct carries hover / press / click / focus / capture state across
/// frames.  Call [`update_input`] once per frame to advance the state.
pub struct UiInputState {
    /// The element currently under the pointer, if any.
    pub hovered: Option<ElementId>,
    /// The element that was pressed (on pointer-down), if any.
    pub pressed: Option<ElementId>,
    /// The element that was clicked (pressed + released on the same element),
    /// consumed after the frame.
    pub clicked: Option<ElementId>,
    /// The element that currently has keyboard focus, if any.
    /// Set by pointer interaction or programmatically.
    pub focused: Option<ElementId>,
    /// The element that has captured pointer events (e.g. during drag).
    /// While set, all pointer events go exclusively to this element.
    pub capture: Option<ElementId>,
    /// Touch / multi-touch state: maps pointer ID to the element it is
    /// currently tracking.  Simple UI uses pointer ID 0 for mouse.
    pub touch_slots: std::collections::HashMap<u64, ElementId>,
}

impl UiInputState {
    /// Create a new input state.
    pub fn new() -> Self {
        Self {
            hovered: None,
            pressed: None,
            clicked: None,
            focused: None,
            capture: None,
            touch_slots: std::collections::HashMap::new(),
        }
    }

    /// Capture pointer events to a specific element.
    /// While captured, all pointer events route exclusively to this element.
    /// Capture is automatically released on the next pointer-up.
    pub fn set_capture(&mut self, element_id: ElementId) {
        self.capture = Some(element_id);
    }

    /// Release any active capture without firing a click.
    pub fn release_capture(&mut self) {
        self.capture = None;
    }

    /// Drain the "clicked" event — returns the element ID that was clicked
    /// this frame, if any.
    pub fn consume_clicked(&mut self) -> Option<ElementId> {
        self.clicked.take()
    }

    /// Reset all transient state (useful when the pointer leaves the canvas).
    pub fn reset(&mut self) {
        self.hovered = None;
        self.pressed = None;
        self.clicked = None;
    }
}

impl Default for UiInputState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Find the topmost enabled element at the given pointer position.
///
/// Elements are tested in reverse draw order (highest `z_order` first),
/// so the topmost visible element under the cursor is returned.
///
/// Returns `None` when no element contains the point.
pub fn hit_test(canvas: &Canvas, pointer_x: f32, pointer_y: f32) -> Option<ElementId> {
    // Collect enabled elements, sorted by z_order descending.
    let mut candidates: Vec<&crate::types::UiElement> =
        canvas.elements.iter().filter(|e| e.enabled).collect();
    candidates.sort_by(|a, b| b.z_order.cmp(&a.z_order));

    for el in &candidates {
        if el.rect.contains(pointer_x, pointer_y) {
            return Some(el.id);
        }
    }

    None
}

/// Find the topmost enabled button element at the given pointer position.
fn hit_test_button(canvas: &Canvas, pointer_x: f32, pointer_y: f32) -> Option<ElementId> {
    let mut candidates: Vec<&crate::types::UiElement> =
        canvas.elements.iter().filter(|e| e.enabled).collect();
    candidates.sort_by(|a, b| b.z_order.cmp(&a.z_order));

    for el in &candidates {
        if el.rect.contains(pointer_x, pointer_y) {
            match &el.kind {
                UiElementKind::Button { .. }
                | UiElementKind::Toggle { .. }
                | UiElementKind::Checkbox { .. }
                | UiElementKind::Slider { .. } => return Some(el.id),
                _ => {}
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Input update
// ---------------------------------------------------------------------------

/// Update the input state for the current frame.
///
/// Call this once per frame with the current pointer / touch state.
///
/// # Parameters
/// * `state` — mutable input state (carries hover, press, click).
/// * `canvas` — the canvas to test against.
/// * `pointer_x`, `pointer_y` — current pointer position in canvas pixels.
/// * `pointer_down` — `true` when the pointer button was pressed this frame.
/// * `pointer_up` — `true` when the pointer button was released this frame.
/// * `callbacks` — optional callback registry for dispatching button clicks.
///
/// When a button is clicked (pressed + released on the same element), the
/// associated callback (if any) is dispatched through the [`CallbackRegistry`].
pub fn update_input(
    state: &mut UiInputState,
    canvas: &Canvas,
    pointer_x: f32,
    pointer_y: f32,
    pointer_down: bool,
    pointer_up: bool,
    callbacks: Option<&mut CallbackRegistry>,
) {
    // Capture ownership: when an element has captured input, route all
    // pointer events exclusively to it until release.
    if let Some(captured) = state.capture {
        if pointer_up {
            // Release capture on pointer-up and fire click/callback.
            state.clicked = Some(captured);
            if let Some(registry) = callbacks {
                fire_button_callback(registry, canvas, captured);
            }
            state.capture = None;
            state.pressed = None;
        }
        return; // All other events go to the capturing element
    }

    // Update hover (skip when capture is active)
    state.hovered = hit_test(canvas, pointer_x, pointer_y);

    // Press detection: interact with any interactive element
    if pointer_down {
        let pressed = hit_test_button(canvas, pointer_x, pointer_y);
        if let Some(id) = pressed {
            state.pressed = pressed;
            state.focused = Some(id); // Clicking gives focus
            debug!(element_id = ?id, "UI element pressed, focus set");
        } else {
            // Clicking outside clears focus
            state.focused = None;
        }
    }

    // Release detection
    if pointer_up {
        if let Some(pressed_id) = state.pressed {
            // Check if the pointer is still over the same element
            let released_on = hit_test(canvas, pointer_x, pointer_y);
            if released_on == Some(pressed_id) {
                // This is a click!
                state.clicked = Some(pressed_id);
                debug!(element_id = ?pressed_id, "UI element clicked");

                // Fire callback if registry is available
                if let Some(registry) = callbacks {
                    fire_button_callback(registry, canvas, pressed_id);
                }
            }
        }
        state.pressed = None;
    }
}

/// Look up the button element and fire its callback if a `callback_id` is set.
fn fire_button_callback(registry: &mut CallbackRegistry, canvas: &Canvas, element_id: ElementId) {
    if let Some(el) = canvas.get_element(element_id) {
        match &el.kind {
            UiElementKind::Button { callback_id, .. }
            | UiElementKind::Toggle { callback_id, .. }
            | UiElementKind::Checkbox { callback_id, .. }
            | UiElementKind::Slider { callback_id, .. } => {
                if let Some(cid) = callback_id {
                    if !cid.is_empty() {
                        registry.invoke(cid);
                        debug!(
                            element_id = ?element_id,
                            callback_id = %cid,
                            "Interactive element callback fired"
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::layout::Layout;
    use crate::types::{UiElement, UiElementKind};
    use crate::Canvas;
    use glam::Vec2;

    fn button_element(layout: Layout, z: i32, callback_id: &str) -> UiElement {
        UiElement::new(
            UiElementKind::Button {
                label: "Test".into(),
                normal_color: Color::WHITE,
                hover_color: Color::new(200, 200, 200, 255),
                pressed_color: Color::new(150, 150, 150, 255),
                callback_id: if callback_id.is_empty() {
                    None
                } else {
                    Some(callback_id.into())
                },
            },
            layout,
        )
        .with_z_order(z)
    }

    fn panel_element(layout: Layout, z: i32) -> UiElement {
        UiElement::new(
            UiElementKind::Panel {
                color: Color::WHITE,
            },
            layout,
        )
        .with_z_order(z)
    }

    fn setup_canvas() -> Canvas {
        let mut canvas = Canvas::new(800.0, 600.0);

        // Panel in the background
        let panel_layout = Layout::FILL;
        canvas.add_element(panel_element(panel_layout, 0));

        // Button at (100, 100, 200, 50)
        let btn_layout = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(100.0, 100.0),
            Vec2::new(300.0, 150.0),
        );
        canvas.add_element(button_element(btn_layout, 1, "btn_test"));

        canvas.layout_all();
        canvas
    }

    #[test]
    fn hit_test_finds_element() {
        let canvas = setup_canvas();
        // Inside the button
        let id = hit_test(&canvas, 150.0, 120.0);
        assert!(id.is_some());
    }

    #[test]
    fn hit_test_returns_none_in_empty_area() {
        let canvas = setup_canvas();
        let id = hit_test(&canvas, -10.0, -10.0);
        assert_eq!(id, None);
    }

    #[test]
    fn hit_test_returns_highest_z() {
        let mut canvas = setup_canvas();

        // Add another button on top
        let top_layout = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(100.0, 100.0),
            Vec2::new(200.0, 120.0),
        );
        let top_id = canvas.add_element(button_element(top_layout, 2, "btn_top"));
        canvas.layout_all();

        let id = hit_test(&canvas, 150.0, 110.0);
        assert_eq!(id, Some(top_id));
    }

    #[test]
    fn hit_test_skips_disabled() {
        let mut canvas = setup_canvas();
        let id = canvas.add_element(
            button_element(
                Layout::new(
                    Vec2::ZERO,
                    Vec2::ZERO,
                    Vec2::new(0.0, 0.0),
                    Vec2::new(800.0, 600.0),
                ),
                10,
                "btn_full",
            )
            .with_enabled(false),
        );
        canvas.layout_all();

        // The disabled full-screen button should be skipped; we should find
        // the panel (z=0) which is enabled.
        let result = hit_test(&canvas, 400.0, 300.0);
        assert!(result.is_some());
        assert_ne!(result, Some(id));
    }

    #[test]
    fn press_on_non_button_ignored() {
        let canvas = setup_canvas();
        let mut state = UiInputState::new();

        // Press on panel (not a button) — should not set pressed
        update_input(&mut state, &canvas, 50.0, 50.0, true, false, None);
        assert!(state.pressed.is_none());
    }

    #[test]
    fn press_and_release_triggers_click() {
        let canvas = setup_canvas();
        let mut state = UiInputState::new();

        // Press on button
        update_input(&mut state, &canvas, 150.0, 125.0, true, false, None);
        assert_eq!(state.pressed, hit_test(&canvas, 150.0, 125.0));

        // Release on same button
        update_input(&mut state, &canvas, 150.0, 125.0, false, true, None);
        assert!(state.pressed.is_none());
        assert_eq!(state.clicked, hit_test(&canvas, 150.0, 125.0));
    }

    #[test]
    fn press_and_release_elsewhere_no_click() {
        let canvas = setup_canvas();
        let mut state = UiInputState::new();

        // Press on button
        update_input(&mut state, &canvas, 150.0, 125.0, true, false, None);
        assert!(state.pressed.is_some());

        // Release away from button
        update_input(&mut state, &canvas, 400.0, 400.0, false, true, None);
        assert!(state.pressed.is_none());
        assert!(state.clicked.is_none());
    }

    #[test]
    fn consume_clicked_drains() {
        let canvas = setup_canvas();
        let mut state = UiInputState::new();

        update_input(&mut state, &canvas, 150.0, 125.0, true, false, None);
        update_input(&mut state, &canvas, 150.0, 125.0, false, true, None);

        assert_eq!(state.consume_clicked(), hit_test(&canvas, 150.0, 125.0));
        assert!(state.consume_clicked().is_none()); // already drained
    }

    #[test]
    fn callback_fires_on_click() {
        let canvas = setup_canvas();
        let mut reg = CallbackRegistry::new();
        let clicked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let clicked_cb = std::sync::Arc::clone(&clicked);
        reg.register(
            "btn_test",
            Box::new(move || {
                clicked_cb.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        let mut state = UiInputState::new();

        update_input(
            &mut state,
            &canvas,
            150.0,
            125.0,
            true,
            false,
            Some(&mut reg),
        );
        update_input(
            &mut state,
            &canvas,
            150.0,
            125.0,
            false,
            true,
            Some(&mut reg),
        );

        assert!(
            clicked.load(std::sync::atomic::Ordering::SeqCst),
            "callback should have been invoked"
        );
    }

    #[test]
    fn callback_not_fired_on_drag_off() {
        let canvas = setup_canvas();
        let mut reg = CallbackRegistry::new();
        let clicked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let clicked_cb = std::sync::Arc::clone(&clicked);
        reg.register(
            "btn_test",
            Box::new(move || {
                clicked_cb.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        let mut state = UiInputState::new();

        update_input(
            &mut state,
            &canvas,
            150.0,
            125.0,
            true,
            false,
            Some(&mut reg),
        );
        // Release elsewhere — no click
        update_input(
            &mut state,
            &canvas,
            400.0,
            400.0,
            false,
            true,
            Some(&mut reg),
        );

        assert!(
            !clicked.load(std::sync::atomic::Ordering::SeqCst),
            "callback should NOT have been invoked"
        );
    }

    #[test]
    fn callback_registry_basic() {
        let mut reg = CallbackRegistry::new();
        let val = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
        let val_cb = std::sync::Arc::clone(&val);
        reg.register(
            "inc",
            Box::new(move || {
                val_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        assert!(reg.contains("inc"));
        assert!(!reg.contains("nonexistent"));

        assert!(reg.invoke("inc"));
        assert_eq!(val.load(std::sync::atomic::Ordering::SeqCst), 1);

        assert!(reg.invoke("inc"));
        assert_eq!(val.load(std::sync::atomic::Ordering::SeqCst), 2);

        assert!(!reg.invoke("nonexistent"));
    }

    #[test]
    fn callback_registry_unregister() {
        let mut reg = CallbackRegistry::new();
        let val = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
        let val_cb = std::sync::Arc::clone(&val);
        reg.register(
            "a",
            Box::new(move || {
                val_cb.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        reg.invoke("a");
        assert_eq!(val.load(std::sync::atomic::Ordering::SeqCst), 1);

        reg.unregister("a");
        assert!(!reg.invoke("a"));
        assert_eq!(val.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn hover_updates() {
        let canvas = setup_canvas();
        let mut state = UiInputState::new();

        // Hover over button
        update_input(&mut state, &canvas, 150.0, 125.0, false, false, None);
        assert!(state.hovered.is_some());

        // Move away
        update_input(&mut state, &canvas, -10.0, -10.0, false, false, None);
        assert!(state.hovered.is_none());
    }

    #[test]
    fn reset_clears_state() {
        let mut state = UiInputState::new();
        state.hovered = Some(ElementId(1));
        state.pressed = Some(ElementId(1));
        state.clicked = Some(ElementId(1));

        state.reset();
        assert!(state.hovered.is_none());
        assert!(state.pressed.is_none());
        assert!(state.clicked.is_none());
    }
}
