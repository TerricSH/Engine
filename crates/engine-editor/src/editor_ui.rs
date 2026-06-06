use std::collections::HashMap;

use engine_ui::{Color, ElementId, Layout, UiElement, UiElementKind, UiRect};
use engine_ui::Canvas;

// -------------------------------------------------------------------
// Legacy event type for backward-compatible testing
// -------------------------------------------------------------------

/// Simulated input event that can be injected to drive editor UI widgets.
#[derive(Clone, Debug)]
pub enum UiEvent {
    /// Simulate a button click.
    ButtonClick(String),
    /// Set a text field value.
    TextFieldCommit(String, String),
    /// Set a slider value.
    SliderDrag(String, f32),
    /// Toggle a checkbox.
    CheckboxToggle(String, bool),
    /// Set a colour.
    ColorPick(String, [f32; 4]),
}

// -------------------------------------------------------------------
// EditorUi – real immediate-mode UI building engine-ui Canvas elements
// -------------------------------------------------------------------

/// Immediate-mode UI helper that builds real [`engine_ui::Canvas`] elements.
///
/// Each frame the host calls [`EditorUi::begin_frame`], runs all panel
/// `ui()` methods, then calls [`EditorUi::end_frame`] to finalize the
/// canvas.  Widget results (clicks, text edits, slider drags) are
/// returned immediately based on input state provided via
/// [`set_pointer`] / [`set_click`].
pub struct EditorUi {
    // -- Canvas being built this frame --
    canvas: Canvas,

    // -- Input state (set by host before begin_frame) --
    cursor_x: f32,
    cursor_y: f32,
    /// Has the left button been pressed since last frame?
    left_pressed: bool,
    /// Has the left button been released since last frame?
    left_released: bool,

    // -- Cross-frame click tracking --
    /// Label under cursor when the button was pressed.
    press_label: Option<String>,
    /// Label under cursor when the button was released.  If equal to
    /// `press_label`, the widget is considered "clicked" next frame.
    release_label: Option<String>,
    /// Set of labels that were clicked this frame (press == release).
    click_results: HashMap<String, bool>,

    // -- Injected events for backward-compat testing --
    injected_events: HashMap<String, UiEvent>,

    // -- Widget hit-testing --
    /// Ordered list of (label, rect) for all interactive widgets this frame.
    widget_hit_areas: Vec<(String, UiRect)>,

    // -- Layout state --
    panel_left: f32,
    panel_top: f32,
    panel_width: f32,
    layout_y: f32,
    widget_height: f32,
}

impl EditorUi {
    /// Create a fresh editor UI context.
    pub fn new() -> Self {
        Self {
            canvas: Canvas::new(1920.0, 1080.0),
            cursor_x: 0.0,
            cursor_y: 0.0,
            left_pressed: false,
            left_released: false,
            press_label: None,
            release_label: None,
            click_results: HashMap::new(),
            injected_events: HashMap::new(),
            widget_hit_areas: Vec::new(),
            panel_left: 0.0,
            panel_top: 0.0,
            panel_width: 250.0,
            layout_y: 0.0,
            widget_height: 22.0,
        }
    }

    // ── Host API ───────────────────────────────────────────────────────────

    /// Call at the **start** of every editor frame, before any panel `ui()`.
    ///
    /// Clears the canvas and resets layout state.  Processes the captured
    /// input events from the previous inter-frame period.
    pub fn begin_frame(&mut self) {
        self.canvas.clear();
        self.widget_hit_areas.clear();
        self.click_results.clear();
        self.panel_left = 0.0;
        self.panel_top = 0.0;
        self.panel_width = 250.0;
        self.layout_y = self.panel_top + 4.0;

        // Process any click that completed between frames.
        if let (Some(p), Some(r)) = (&self.press_label, &self.release_label) {
            if p == r {
                self.click_results.insert(p.clone(), true);
            }
        }
    }

    /// Call at the **end** of every editor frame, after all panel `ui()`.
    ///
    /// Finalises the canvas layout and captures press/release labels for
    /// next frame's click detection.  Returns the [`Canvas`] for batch
    /// extraction.
    pub fn end_frame(&mut self) -> &Canvas {
        // After all widgets are added, do a layout pass.
        self.canvas.layout_all();

        // Capture press/release labels for next frame.
        if self.left_pressed {
            self.press_label = self.hit_test_label();
        }

        if self.left_released {
            self.release_label = self.hit_test_label();
        }

        // If release happened without a matching press, clear it.
        if self.left_released && self.press_label.is_none() {
            self.release_label = None;
        }

        // Once a click pair is consumed, clear both so we don't double-fire.
        if self.press_label.is_some() && self.release_label.is_some() {
            if self.press_label == self.release_label {
                self.press_label = None;
                self.release_label = None;
            }
        }

        // Reset frame-level flags for next frame.
        self.left_pressed = false;
        self.left_released = false;

        &self.canvas
    }

    /// Update pointer position.  Call before `begin_frame`.
    pub fn set_pointer(&mut self, x: f32, y: f32) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    /// Record a mouse press event.  Call before `begin_frame`.
    pub fn set_mouse_pressed(&mut self) {
        self.left_pressed = true;
    }

    /// Record a mouse release event.  Call before `begin_frame`.
    pub fn set_mouse_released(&mut self) {
        self.left_released = true;
    }

    /// Set the current panel region.
    pub fn set_panel_rect(&mut self, left: f32, top: f32, width: f32) {
        self.panel_left = left;
        self.panel_top = top;
        self.panel_width = width;
        self.layout_y = top + 4.0;
    }

    /// Access the underlying Canvas (for batch extraction).
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Consume the canvas and return it.
    pub fn take_canvas(&mut self) -> Canvas {
        let mut c = Canvas::new(self.canvas.width, self.canvas.height);
        std::mem::swap(&mut c, &mut self.canvas);
        c
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn hit_test_label(&self) -> Option<String> {
        for (label, rect) in self.widget_hit_areas.iter().rev() {
            if rect.contains(self.cursor_x, self.cursor_y) {
                return Some(label.clone());
            }
        }
        None
    }

    fn push_widget(&mut self, label: &str, rect: UiRect, kind: UiElementKind) {
        self.widget_hit_areas.push((label.to_string(), rect));
        let eid = self.canvas.add_element(
            UiElement::new(kind, Layout::FILL).with_z_order(10),
        );
        if let Some(el) = self.canvas.get_element_mut(eid) {
            el.rect = rect;
        }
    }

    fn add_text(&mut self, x: f32, y: f32, w: f32, h: f32, text: &str, font_size: f32, color: Color) {
        let eid = self.canvas.add_element(
            UiElement::new(
                UiElementKind::Text {
                    content: text.to_string(),
                    font_size,
                    color,
                },
                Layout::FILL,
            )
            .with_z_order(11),
        );
        if let Some(el) = self.canvas.get_element_mut(eid) {
            el.rect = UiRect::new(x, y, w, h);
        }
    }

    fn advance(&mut self) {
        self.layout_y += self.widget_height + 4.0;
    }

    fn label_rect(&self) -> UiRect {
        UiRect::new(
            self.panel_left + 8.0,
            self.layout_y + 3.0,
            self.panel_width - 16.0,
            self.widget_height - 3.0,
        )
    }

    fn widget_rect(&self) -> UiRect {
        UiRect::new(
            self.panel_left + 4.0,
            self.layout_y,
            self.panel_width - 8.0,
            self.widget_height,
        )
    }

    // ── Widget API ────────────────────────────────────────────────────────

    /// A push button.  Returns `true` ONCE when clicked.
    pub fn button(&mut self, label: &str) -> bool {
        let rect = self.widget_rect();

        // Check injected events (testing path)
        if let Some(UiEvent::ButtonClick(_)) = self.injected_events.remove(label) {
            self.advance();
            return true;
        }

        let is_hovered = rect.contains(self.cursor_x, self.cursor_y);
        let color = if is_hovered && self.left_pressed {
            Color::new(100, 140, 200, 255)
        } else if is_hovered {
            Color::new(80, 120, 180, 255)
        } else {
            Color::new(50, 70, 100, 255)
        };

        let kind = UiElementKind::Button {
            label: label.to_string(),
            normal_color: color,
            hover_color: Color::new(80, 120, 180, 255),
            pressed_color: Color::new(100, 140, 200, 255),
            callback_id: None,
        };
        self.push_widget(label, rect, kind);

        let lr = self.label_rect();
        self.add_text(lr.x, lr.y, lr.width, lr.height, label, 14.0, Color::new(220, 220, 220, 255));

        let clicked = self.click_results.contains_key(label);
        self.advance();
        clicked
    }

    /// A single-line text field.  Returns `Some(edited_value)` when the
    /// user commits a change, or `None` if unchanged.
    pub fn text_field(&mut self, label: &str, value: &str) -> Option<String> {
        let rect = self.widget_rect();

        // Check injected events (testing path)
        if let Some(UiEvent::TextFieldCommit(_, new_val)) = self.injected_events.remove(label) {
            self.advance();
            return Some(new_val);
        }

        // Background panel
        let kind = UiElementKind::Panel {
            color: Color::new(30, 30, 40, 255),
        };
        self.push_widget(label, rect, kind);

        let lr = self.label_rect();
        self.add_text(lr.x, lr.y, lr.width, lr.height, &format!("{label}: {value}"), 12.0, Color::new(180, 180, 180, 255));

        self.advance();
        None
    }

    /// A horizontal slider for `f32` values.
    pub fn slider_f32(&mut self, label: &str, value: f32, min: f32, max: f32) -> Option<f32> {
        let rect = self.widget_rect();

        // Check injected events (testing path)
        if let Some(UiEvent::SliderDrag(_, new_val)) = self.injected_events.remove(label) {
            self.advance();
            return Some(new_val.clamp(min, max));
        }

        let kind = UiElementKind::Slider {
            label: label.to_string(),
            value,
            min,
            max,
            callback_id: None,
        };
        self.push_widget(label, rect, kind);

        let lr = self.label_rect();
        self.add_text(lr.x, lr.y, lr.width, lr.height, &format!("{label}: {value:.2}"), 12.0, Color::new(180, 180, 180, 255));

        self.advance();
        None
    }

    /// A checkbox.  Returns the *new* checked state.
    pub fn checkbox(&mut self, label: &str, checked: bool) -> bool {
        let rect = self.widget_rect();

        // Check injected events (testing path)
        if let Some(UiEvent::CheckboxToggle(_, new_state)) = self.injected_events.remove(label) {
            self.advance();
            return new_state;
        }

        let kind = UiElementKind::Checkbox {
            label: label.to_string(),
            checked,
            color: Color::new(100, 180, 255, 255),
            callback_id: None,
        };
        self.push_widget(label, rect, kind);

        let lr = self.label_rect();
        self.add_text(lr.x + 20.0, lr.y, lr.width - 20.0, lr.height, label, 12.0, Color::new(180, 180, 180, 255));

        let was_clicked = self.click_results.contains_key(label);
        self.advance();
        if was_clicked { !checked } else { checked }
    }

    /// A simple color picker.  Returns `Some(new_color)` when the user
    /// adjusts the colour, or `None` if unchanged.
    pub fn color_edit(&mut self, label: &str, color: [f32; 4]) -> Option<[f32; 4]> {
        let rect = self.widget_rect();

        // Check injected events (testing path)
        if let Some(UiEvent::ColorPick(_, new_color)) = self.injected_events.remove(label) {
            self.advance();
            return Some(new_color);
        }

        let c = Color::new(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        );

        let kind = UiElementKind::Panel { color: c };
        self.push_widget(label, rect, kind);

        let lr = self.label_rect();
        self.add_text(lr.x, lr.y, lr.width, lr.height, &format!("{label} [{},{},{},{}]", c.r, c.g, c.b, c.a), 11.0, Color::new(255, 255, 255, 255));

        self.advance();
        None
    }

    /// A horizontal separator line.
    pub fn separator(&mut self) {
        let rect = UiRect::new(self.panel_left + 4.0, self.layout_y, self.panel_width - 8.0, 1.0);
        let eid = self.canvas.add_element(
            UiElement::new(
                UiElementKind::Panel { color: Color::new(60, 60, 70, 255) },
                Layout::FILL,
            )
            .with_z_order(5),
        );
        if let Some(el) = self.canvas.get_element_mut(eid) {
            el.rect = rect;
        }
        self.layout_y += 6.0;
    }

    /// A collapsible header section.  Returns `true` when expanded.
    pub fn collapsing_header(&mut self, label: &str, default_open: bool) -> bool {
        let rect = self.widget_rect();

        let is_hovered = rect.contains(self.cursor_x, self.cursor_y);
        let bg_color = if is_hovered {
            Color::new(50, 60, 80, 255)
        } else {
            Color::new(35, 40, 55, 255)
        };

        let kind = UiElementKind::Panel { color: bg_color };
        self.push_widget(label, rect, kind);

        // Check injected events (testing path)
        if let Some(UiEvent::ButtonClick(_)) = self.injected_events.remove(label) {
            self.advance();
            return !default_open;
        }

        let lr = self.label_rect();
        self.add_text(lr.x, lr.y, lr.width, lr.height, label, 13.0, Color::new(200, 200, 210, 255));

        // Track toggle state
        let was_clicked = self.click_results.contains_key(label);
        self.advance();
        if was_clicked { !default_open } else { default_open }
    }

    /// Reset layout state for a new panel/frame.
    pub fn reset(&mut self) {
        self.layout_y = self.panel_top + 4.0;
    }

    /// Inject a UI event for the next frame.
    pub fn inject_event(&mut self, event: UiEvent) {
        let key = match &event {
            UiEvent::ButtonClick(l) => l.clone(),
            UiEvent::TextFieldCommit(l, _) => l.clone(),
            UiEvent::SliderDrag(l, _) => l.clone(),
            UiEvent::CheckboxToggle(l, _) => l.clone(),
            UiEvent::ColorPick(l, _) => l.clone(),
        };
        self.injected_events.insert(key, event);
    }

    // ── Layout control ──────────────────────────────────────────────────

    /// Set the current panel position.
    pub fn set_panel_position(&mut self, left: f32, top: f32) {
        self.panel_left = left;
        self.panel_top = top;
        self.layout_y = top + 4.0;
    }

    /// Set the current panel content width.
    pub fn set_panel_content_width(&mut self, width: f32) {
        self.panel_width = width;
    }
}

impl Default for EditorUi {
    fn default() -> Self {
        Self::new()
    }
}
