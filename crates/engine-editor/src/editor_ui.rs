// -------------------------------------------------------------------
// EditorUi – immediate-mode UI helper
// -------------------------------------------------------------------

/// Simple immediate-mode UI helper passed to [`EditorPanel::ui`].
///
/// Panels declare their widgets through this object.  In a real engine
/// integration a backend would translate these calls into rendered UI;
/// here they serve as scaffolding that returns default / no-op values.
pub struct EditorUi {
    // Layout tracking for auto-advancing the cursor.
    cursor_y: f32,
    // Widget ID generation (ensures unique IDs per frame).
    next_id: u64,
}

impl EditorUi {
    /// Create a fresh UI context.
    pub fn new() -> Self {
        Self {
            cursor_y: 0.0,
            next_id: 1,
        }
    }

    /// Reset layout state for a new panel/frame.
    pub fn reset(&mut self) {
        self.cursor_y = 0.0;
        self.next_id = 1;
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A single-line text field.
    ///
    /// Returns `Some(edited_value)` when the user commits a change, or
    /// `None` if the value was not modified.
    pub fn text_field(&mut self, label: &str, value: &str) -> Option<String> {
        let _ = (label, value, self.alloc_id());
        // Scaffolding: real input sampling would go here.
        None
    }

    /// A push button.
    ///
    /// Returns `true` for the frame in which the button was clicked.
    pub fn button(&mut self, label: &str) -> bool {
        let _ = (label, self.alloc_id());
        false
    }

    /// A horizontal slider for `f32` values.
    ///
    /// Returns `Some(new_value)` while the slider is being dragged, or
    /// `None` when released / unchanged.
    pub fn slider_f32(&mut self, label: &str, value: f32, min: f32, max: f32) -> Option<f32> {
        let _ = (label, value, min, max, self.alloc_id());
        None
    }

    /// A checkbox.
    ///
    /// Returns the *new* checked state.  If the user did not interact
    /// with the widget the return value equals `checked`.
    pub fn checkbox(&mut self, label: &str, checked: bool) -> bool {
        let _ = (label, self.alloc_id());
        checked
    }

    /// A simple color picker.
    ///
    /// Returns `Some(new_color)` when the user adjusts the colour, or
    /// `None` if unchanged.
    pub fn color_edit(&mut self, label: &str, color: [f32; 4]) -> Option<[f32; 4]> {
        let _ = (label, color, self.alloc_id());
        None
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
