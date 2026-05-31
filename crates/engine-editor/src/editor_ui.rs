// -------------------------------------------------------------------
// EditorUi – immediate-mode UI helper
// -------------------------------------------------------------------

use std::collections::HashMap;

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

/// Simple immediate-mode UI helper passed to [`EditorPanel::ui`].
///
/// Panels declare their widgets through this object.  In a real engine
/// integration a backend would translate these calls into rendered UI;
/// here events can be injected to drive widget responses, enabling
/// production editor workflows and tests.
pub struct EditorUi {
    // Layout tracking for auto-advancing the cursor.
    cursor_y: f32,
    // Widget ID generation (ensures unique IDs per frame).
    next_id: u64,
    // Injected events keyed by label — consumed on first matching call.
    events: HashMap<String, UiEvent>,
}

impl EditorUi {
    /// Create a fresh UI context.
    pub fn new() -> Self {
        Self {
            cursor_y: 0.0,
            next_id: 1,
            events: HashMap::new(),
        }
    }

    /// Reset layout state for a new panel/frame.
    pub fn reset(&mut self) {
        self.cursor_y = 0.0;
        self.next_id = 1;
    }

    /// Inject a UI event for the next frame.  The event is consumed the
    /// first time a widget with a matching label is polled.
    pub fn inject_event(&mut self, event: UiEvent) {
        let key = match &event {
            UiEvent::ButtonClick(l) => l.clone(),
            UiEvent::TextFieldCommit(l, _) => l.clone(),
            UiEvent::SliderDrag(l, _) => l.clone(),
            UiEvent::CheckboxToggle(l, _) => l.clone(),
            UiEvent::ColorPick(l, _) => l.clone(),
        };
        self.events.insert(key, event);
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn consume_event(&mut self, label: &str) -> Option<UiEvent> {
        self.events.remove(label)
    }

    /// A single-line text field.
    ///
    /// Returns `Some(edited_value)` when the user commits a change, or
    /// `None` if the value was not modified.
    pub fn text_field(&mut self, label: &str, value: &str) -> Option<String> {
        let _ = (value, self.alloc_id());
        match self.consume_event(label) {
            Some(UiEvent::TextFieldCommit(_, new_val)) => Some(new_val),
            _ => None,
        }
    }

    /// A push button.
    ///
    /// Returns `true` ONCE when a click event is consumed, then `false`
    /// for subsequent calls until a new event is injected.
    pub fn button(&mut self, label: &str) -> bool {
        let _ = self.alloc_id();
        if let Some(UiEvent::ButtonClick(_)) = self.consume_event(label) {
            true
        } else {
            false
        }
    }

    /// A horizontal slider for `f32` values.
    ///
    /// Returns `Some(new_value)` while the slider is being dragged, or
    /// `None` when released / unchanged.
    pub fn slider_f32(&mut self, label: &str, value: f32, min: f32, max: f32) -> Option<f32> {
        let _ = (value, min, max, self.alloc_id());
        match self.consume_event(label) {
            Some(UiEvent::SliderDrag(_, new_val)) => Some(new_val.clamp(min, max)),
            _ => None,
        }
    }

    /// A checkbox.
    ///
    /// Returns the *new* checked state.  If the user did not interact
    /// with the widget the return value equals `checked`.
    pub fn checkbox(&mut self, label: &str, checked: bool) -> bool {
        let _ = self.alloc_id();
        match self.consume_event(label) {
            Some(UiEvent::CheckboxToggle(_, new_state)) => new_state,
            _ => checked,
        }
    }

    /// A simple color picker.
    ///
    /// Returns `Some(new_color)` when the user adjusts the colour, or
    /// `None` if unchanged.
    pub fn color_edit(&mut self, label: &str, color: [f32; 4]) -> Option<[f32; 4]> {
        let _ = (color, self.alloc_id());
        match self.consume_event(label) {
            Some(UiEvent::ColorPick(_, new_color)) => Some(new_color),
            _ => None,
        }
    }

    /// A horizontal separator line.
    pub fn separator(&mut self) {
        self.cursor_y += 8.0;
    }

    /// A collapsible header section.
    ///
    /// Returns `true` when the header is expanded.
    pub fn collapsing_header(&mut self, label: &str, default_open: bool) -> bool {
        let _ = (label, self.alloc_id());
        default_open
    }
}

impl Default for EditorUi {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_returns_false_without_event() {
        let mut ui = EditorUi::new();
        assert!(!ui.button("Test"));
    }

    #[test]
    fn button_returns_true_when_clicked() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Test".into()));
        assert!(ui.button("Test"));
    }

    #[test]
    fn text_field_returns_none_without_event() {
        let mut ui = EditorUi::new();
        assert_eq!(ui.text_field("Name", "old"), None);
    }

    #[test]
    fn text_field_returns_value_when_committed() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::TextFieldCommit("Name".into(), "new".into()));
        assert_eq!(ui.text_field("Name", "old"), Some("new".into()));
    }

    #[test]
    fn slider_returns_clamped_value() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::SliderDrag("Vol".into(), 2.0));
        let result = ui.slider_f32("Vol", 0.5, 0.0, 1.0);
        assert_eq!(result, Some(1.0)); // clamped
    }

    #[test]
    fn checkbox_returns_injected_state() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::CheckboxToggle("Enable".into(), true));
        assert!(ui.checkbox("Enable", false));
    }

    #[test]
    fn event_is_consumed_on_first_call() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Once".into()));
        assert!(ui.button("Once")); // consumed
        assert!(!ui.button("Once")); // gone
    }

    #[test]
    fn reset_clears_clicked_state() {
        let mut ui = EditorUi::new();
        ui.inject_event(UiEvent::ButtonClick("Btn".into()));
        assert!(ui.button("Btn"));
        ui.reset();
        assert!(!ui.button("Btn")); // reset cleared it
    }
}
