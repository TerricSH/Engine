//! UI input handling — hit testing, hover detection, and click dispatch.
//!
//! The preferred entry point is [`UiInputState::process_event`]. The legacy
//! [`update_input`] adapter remains available for callback-based callers.

use tracing::debug;

use crate::types::{ElementId, UiElementKind};
use crate::Canvas;

// ---------------------------------------------------------------------------
// Platform-independent pointer and click events
// ---------------------------------------------------------------------------

/// A primary-pointer event expressed in canvas pixel coordinates.
///
/// Platform integrations translate mouse, pen, or single-touch input into
/// this type. [`UiInputState::process_event`] performs layout before using the
/// coordinates, so callers do not need to keep computed rectangles current.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiPointerEvent {
    /// The pointer moved without changing its pressed state.
    Move { x: f32, y: f32 },
    /// The primary pointer button was pressed.
    Press { x: f32, y: f32 },
    /// The primary pointer button was released.
    Release { x: f32, y: f32 },
    /// The current pointer interaction was interrupted.
    Cancel,
}

/// A completed click on an interactive retained-mode element.
///
/// `callback_id` mirrors the element's optional callback identifier. The
/// event is still emitted when that identifier is absent so native gameplay
/// code can route the click by [`ElementId`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiClickEvent {
    pub element_id: ElementId,
    pub callback_id: Option<String>,
}

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
        self.pressed = None;
        self.clicked = None;
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
        self.capture = None;
        self.touch_slots.clear();
    }

    /// Process one platform-independent pointer event.
    ///
    /// Layout is recomputed before every event. A press on an interactive
    /// element captures the pointer until release or cancellation. A click is
    /// produced only when release occurs inside the captured element while it
    /// remains enabled and interactive.
    ///
    /// Buttons, toggles, checkboxes, and sliders all produce the same click
    /// event. This method deliberately does not mutate toggle, checkbox, or
    /// slider values; gameplay or script code owns those state changes.
    pub fn process_event(
        &mut self,
        canvas: &mut Canvas,
        event: UiPointerEvent,
    ) -> Option<UiClickEvent> {
        canvas.layout_all();
        self.process_laid_out_event(canvas, event)
    }

    fn process_laid_out_event(
        &mut self,
        canvas: &Canvas,
        event: UiPointerEvent,
    ) -> Option<UiClickEvent> {
        // `clicked` describes only the event just processed. Clearing it here
        // prevents a later release/cancel from exposing a stale click.
        self.clicked = None;

        match event {
            UiPointerEvent::Move { x, y } => {
                self.hovered = match self.capture {
                    Some(captured) => {
                        interactive_element_at(canvas, captured, x, y).then_some(captured)
                    }
                    None => hit_test_interactive(canvas, x, y),
                };
                None
            }
            UiPointerEvent::Press { x, y } => {
                let target = hit_test_interactive(canvas, x, y);
                self.hovered = target;
                self.pressed = target;
                self.capture = target;
                self.focused = target;
                if let Some(element_id) = target {
                    debug!(?element_id, "UI element pressed and captured");
                }
                None
            }
            UiPointerEvent::Release { x, y } => {
                let captured = self.capture.take();
                let pressed = self.pressed.take();
                self.hovered = hit_test_interactive(canvas, x, y);

                let clicked = captured.filter(|element_id| {
                    pressed == Some(*element_id)
                        && interactive_element_at(canvas, *element_id, x, y)
                });
                let event = clicked.and_then(|element_id| click_event(canvas, element_id));
                if let Some(event) = &event {
                    self.clicked = Some(event.element_id);
                    debug!(element_id = ?event.element_id, "UI element clicked");
                }
                event
            }
            UiPointerEvent::Cancel => {
                self.hovered = None;
                self.pressed = None;
                self.capture = None;
                self.touch_slots.clear();
                None
            }
        }
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
/// Draw order is deterministic: higher `z_order` wins, and for equal
/// `z_order` the element inserted later in [`Canvas::elements`] wins because
/// it is drawn later.
///
/// Returns `None` when no element contains the point.
pub fn hit_test(canvas: &Canvas, pointer_x: f32, pointer_y: f32) -> Option<ElementId> {
    topmost_element(canvas, pointer_x, pointer_y, |_| true)
}

/// Find the topmost enabled interactive element at a canvas position.
///
/// Interactive elements are buttons, toggles, checkboxes, and sliders. The
/// ordering exactly matches [`hit_test`]: higher z first, then later insertion
/// for equal z. Computed rectangles must already be current; use
/// [`UiInputState::process_event`] when handling an event because it performs
/// layout automatically.
pub fn hit_test_interactive(canvas: &Canvas, pointer_x: f32, pointer_y: f32) -> Option<ElementId> {
    topmost_element(canvas, pointer_x, pointer_y, is_interactive)
}

fn topmost_element(
    canvas: &Canvas,
    pointer_x: f32,
    pointer_y: f32,
    accepts: impl Fn(&UiElementKind) -> bool,
) -> Option<ElementId> {
    canvas
        .elements
        .iter()
        .enumerate()
        .filter(|(_, element)| {
            element.enabled && accepts(&element.kind) && element.rect.contains(pointer_x, pointer_y)
        })
        .max_by_key(|(draw_index, element)| (element.z_order, *draw_index))
        .map(|(_, element)| element.id)
}

fn is_interactive(kind: &UiElementKind) -> bool {
    matches!(
        kind,
        UiElementKind::Button { .. }
            | UiElementKind::Toggle { .. }
            | UiElementKind::Checkbox { .. }
            | UiElementKind::Slider { .. }
    )
}

fn interactive_element_at(
    canvas: &Canvas,
    element_id: ElementId,
    pointer_x: f32,
    pointer_y: f32,
) -> bool {
    canvas.get_element(element_id).is_some_and(|element| {
        element.enabled
            && is_interactive(&element.kind)
            && element.rect.contains(pointer_x, pointer_y)
    })
}

fn click_event(canvas: &Canvas, element_id: ElementId) -> Option<UiClickEvent> {
    let element = canvas.get_element(element_id)?;
    let callback_id = match &element.kind {
        UiElementKind::Button { callback_id, .. }
        | UiElementKind::Toggle { callback_id, .. }
        | UiElementKind::Checkbox { callback_id, .. }
        | UiElementKind::Slider { callback_id, .. } => callback_id.clone(),
        _ => return None,
    };
    Some(UiClickEvent {
        element_id,
        callback_id,
    })
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
/// When an interactive element is clicked (pressed + released on the same
/// element), its callback (if any) is dispatched through the
/// [`CallbackRegistry`].
/// Unlike [`UiInputState::process_event`], this compatibility adapter cannot
/// recompute layout because it accepts an immutable canvas; callers must run
/// [`Canvas::layout_all`] after changing layout data.
pub fn update_input(
    state: &mut UiInputState,
    canvas: &Canvas,
    pointer_x: f32,
    pointer_y: f32,
    pointer_down: bool,
    pointer_up: bool,
    callbacks: Option<&mut CallbackRegistry>,
) {
    let press = UiPointerEvent::Press {
        x: pointer_x,
        y: pointer_y,
    };
    let release = UiPointerEvent::Release {
        x: pointer_x,
        y: pointer_y,
    };
    let click = match (pointer_down, pointer_up) {
        (true, true) => {
            state.process_laid_out_event(canvas, press);
            state.process_laid_out_event(canvas, release)
        }
        (true, false) => state.process_laid_out_event(canvas, press),
        (false, true) => state.process_laid_out_event(canvas, release),
        (false, false) => state.process_laid_out_event(
            canvas,
            UiPointerEvent::Move {
                x: pointer_x,
                y: pointer_y,
            },
        ),
    };

    if let (Some(click), Some(registry)) = (click, callbacks) {
        if let Some(callback_id) = click.callback_id.filter(|id| !id.is_empty()) {
            registry.invoke(&callback_id);
            debug!(
                element_id = ?click.element_id,
                %callback_id,
                "Interactive element callback fired"
            );
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

    fn interactive_kinds(callback_id: Option<&str>) -> Vec<UiElementKind> {
        let callback_id = || callback_id.map(str::to_owned);
        vec![
            UiElementKind::Button {
                label: "Button".into(),
                normal_color: Color::WHITE,
                hover_color: Color::WHITE,
                pressed_color: Color::WHITE,
                callback_id: callback_id(),
            },
            UiElementKind::Toggle {
                label: "Toggle".into(),
                is_on: false,
                color_on: Color::WHITE,
                color_off: Color::BLACK,
                callback_id: callback_id(),
            },
            UiElementKind::Checkbox {
                label: "Checkbox".into(),
                checked: false,
                color: Color::WHITE,
                callback_id: callback_id(),
            },
            UiElementKind::Slider {
                label: "Slider".into(),
                value: 0.25,
                min: 0.0,
                max: 1.0,
                callback_id: callback_id(),
            },
        ]
    }

    fn click(state: &mut UiInputState, canvas: &mut Canvas, x: f32, y: f32) -> UiClickEvent {
        assert_eq!(
            state.process_event(canvas, UiPointerEvent::Press { x, y }),
            None
        );
        state
            .process_event(canvas, UiPointerEvent::Release { x, y })
            .expect("press and release on the same interactive element must click")
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
    fn overlapping_elements_use_z_then_later_draw_order() {
        let mut canvas = Canvas::new(100.0, 100.0);
        let layout = Layout::FILL;
        let high_z = canvas.add_element(button_element(layout, 2, "high_z"));
        canvas.add_element(button_element(layout, 1, "later_but_lower"));
        canvas.layout_all();
        assert_eq!(hit_test_interactive(&canvas, 50.0, 50.0), Some(high_z));

        let same_z_later = canvas.add_element(button_element(layout, 2, "same_z_later"));
        canvas.layout_all();
        assert_eq!(
            hit_test_interactive(&canvas, 50.0, 50.0),
            Some(same_z_later)
        );

        let mut state = UiInputState::new();
        assert_eq!(
            click(&mut state, &mut canvas, 50.0, 50.0),
            UiClickEvent {
                element_id: same_z_later,
                callback_id: Some("same_z_later".into()),
            }
        );
    }

    #[test]
    fn interactive_hit_test_ignores_visual_only_elements() {
        let mut canvas = Canvas::new(100.0, 100.0);
        let button = canvas.add_element(button_element(Layout::FILL, 0, "button"));
        let panel = canvas.add_element(panel_element(Layout::FILL, 10));
        canvas.layout_all();

        assert_eq!(hit_test(&canvas, 50.0, 50.0), Some(panel));
        assert_eq!(hit_test_interactive(&canvas, 50.0, 50.0), Some(button));
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
    fn process_event_layouts_before_every_pointer_event() {
        let mut canvas = Canvas::new(200.0, 100.0);
        let id = canvas.add_element(button_element(
            Layout::new(
                Vec2::ZERO,
                Vec2::ZERO,
                Vec2::new(10.0, 10.0),
                Vec2::new(60.0, 40.0),
            ),
            0,
            "button",
        ));
        assert_eq!(canvas.get_element(id).unwrap().rect, crate::UiRect::ZERO);

        let mut state = UiInputState::new();
        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Press { x: 20.0, y: 20.0 }),
            None
        );
        assert_eq!(state.capture, Some(id));
        assert_ne!(canvas.get_element(id).unwrap().rect, crate::UiRect::ZERO);

        canvas.get_element_mut(id).unwrap().layout = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(100.0, 10.0),
            Vec2::new(150.0, 40.0),
        );
        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 20.0, y: 20.0 }),
            None,
            "release must use the newly laid-out rectangle"
        );
        assert_eq!(state.capture, None);
        assert_eq!(state.pressed, None);
    }

    #[test]
    fn every_interactive_kind_emits_callback_and_element_id_without_mutating_value() {
        for (index, kind) in interactive_kinds(Some("gameplay_callback"))
            .into_iter()
            .enumerate()
        {
            let expected_kind = kind.clone();
            let mut canvas = Canvas::new(100.0, 100.0);
            let id = canvas.add_element(UiElement::new(kind, Layout::FILL));
            let mut state = UiInputState::new();

            assert_eq!(
                click(&mut state, &mut canvas, 50.0, 50.0),
                UiClickEvent {
                    element_id: id,
                    callback_id: Some("gameplay_callback".into()),
                },
                "interactive kind {index} must emit a routed click"
            );
            assert_eq!(canvas.get_element(id).unwrap().kind, expected_kind);
        }
    }

    #[test]
    fn every_interactive_kind_without_callback_still_emits_element_id() {
        for (index, kind) in interactive_kinds(None).into_iter().enumerate() {
            let mut canvas = Canvas::new(100.0, 100.0);
            let id = canvas.add_element(UiElement::new(kind, Layout::FILL));
            let mut state = UiInputState::new();

            assert_eq!(
                click(&mut state, &mut canvas, 50.0, 50.0),
                UiClickEvent {
                    element_id: id,
                    callback_id: None,
                },
                "interactive kind {index} must retain its element ID"
            );
        }
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
    fn press_capture_does_not_retarget_or_click_on_release_outside() {
        let mut canvas = Canvas::new(200.0, 100.0);
        let left = canvas.add_element(button_element(
            Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(80.0, 100.0)),
            0,
            "left",
        ));
        let right = canvas.add_element(button_element(
            Layout::new(
                Vec2::ZERO,
                Vec2::ZERO,
                Vec2::new(120.0, 0.0),
                Vec2::new(200.0, 100.0),
            ),
            0,
            "right",
        ));
        let mut state = UiInputState::new();

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Press { x: 40.0, y: 50.0 }),
            None
        );
        assert_eq!(state.pressed, Some(left));
        assert_eq!(state.capture, Some(left));

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Move { x: 160.0, y: 50.0 }),
            None
        );
        assert_eq!(state.capture, Some(left));
        assert_eq!(state.hovered, None, "captured movement cannot retarget");
        assert_eq!(hit_test_interactive(&canvas, 160.0, 50.0), Some(right));

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 160.0, y: 50.0 }),
            None
        );
        assert_eq!(state.capture, None);
        assert_eq!(state.pressed, None);
        assert_eq!(state.clicked, None);
        assert_eq!(state.hovered, Some(right));
    }

    #[test]
    fn press_outside_then_release_inside_does_not_click() {
        let mut canvas = setup_canvas();
        let mut state = UiInputState::new();

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Press { x: -1.0, y: -1.0 }),
            None
        );
        assert_eq!(state.capture, None);
        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 150.0, y: 125.0 }),
            None
        );
        assert_eq!(state.clicked, None);
    }

    #[test]
    fn cancel_clears_capture_and_prevents_a_later_ghost_click() {
        let mut canvas = setup_canvas();
        let mut state = UiInputState::new();

        state.process_event(&mut canvas, UiPointerEvent::Press { x: 150.0, y: 125.0 });
        assert!(state.capture.is_some());
        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Cancel),
            None
        );
        assert_eq!(state.capture, None);
        assert_eq!(state.pressed, None);
        assert_eq!(state.clicked, None);

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 150.0, y: 125.0 }),
            None
        );
        assert_eq!(state.clicked, None);
    }

    #[test]
    fn stray_release_clears_the_previous_transient_click() {
        let mut canvas = setup_canvas();
        let mut state = UiInputState::new();

        let first = click(&mut state, &mut canvas, 150.0, 125.0);
        assert_eq!(state.clicked, Some(first.element_id));
        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 150.0, y: 125.0 }),
            None
        );
        assert_eq!(state.clicked, None);
    }

    #[test]
    fn disabling_captured_element_before_release_prevents_click() {
        let mut canvas = setup_canvas();
        let mut state = UiInputState::new();
        state.process_event(&mut canvas, UiPointerEvent::Press { x: 150.0, y: 125.0 });
        let captured = state.capture.expect("button must capture the press");
        canvas.get_element_mut(captured).unwrap().enabled = false;

        assert_eq!(
            state.process_event(&mut canvas, UiPointerEvent::Release { x: 150.0, y: 125.0 }),
            None
        );
        assert_eq!(state.capture, None);
        assert_eq!(state.clicked, None);
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
        state.capture = Some(ElementId(1));
        state.touch_slots.insert(7, ElementId(1));

        state.reset();
        assert!(state.hovered.is_none());
        assert!(state.pressed.is_none());
        assert!(state.clicked.is_none());
        assert!(state.capture.is_none());
        assert!(state.touch_slots.is_empty());
    }

    #[test]
    fn releasing_capture_cannot_leave_a_pending_click() {
        let mut state = UiInputState::new();
        state.pressed = Some(ElementId(1));
        state.capture = Some(ElementId(1));
        state.clicked = Some(ElementId(1));

        state.release_capture();

        assert_eq!(state.pressed, None);
        assert_eq!(state.capture, None);
        assert_eq!(state.clicked, None);
    }
}
