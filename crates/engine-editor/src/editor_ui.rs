use std::collections::HashMap;

use engine_ui::Canvas;
use engine_ui::{Color, Layout, UiElement, UiElementKind, UiRect};

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

/// Editing keys forwarded by the platform host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKey {
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
}

#[derive(Clone, Debug)]
struct TextEditState {
    id: String,
    original: String,
    buffer: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextInputEvent {
    Character(char),
    Key(UiKey),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PointerInputEvent {
    Press { position: [f32; 2], sequence: u64 },
    Release { position: [f32; 2], sequence: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum InputEvent {
    Pointer(PointerInputEvent),
    Text {
        event: TextInputEvent,
        sequence: u64,
    },
}

/// Position of a semantic widget result relative to the raw pointer event
/// carrying the same platform sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiInteractionPhase {
    /// Text blur is committed before a pointer press reaches viewport tools.
    BeforeRawPointer,
    /// Buttons and toggles fire after their physical pointer release.
    AfterRawPointer,
}

/// Shared platform order attached to a semantic panel edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiInteractionStamp {
    pub sequence: u64,
    pub phase: UiInteractionPhase,
}

#[derive(Clone, Debug)]
struct PendingTextCommit {
    value: String,
    stamp: UiInteractionStamp,
}

#[derive(Clone, Debug, PartialEq)]
enum FramePointerInputEvent {
    Press {
        position: [f32; 2],
        target: Option<String>,
        sequence: u64,
    },
    Release {
        position: [f32; 2],
        sequence: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ClickRecord {
    sequences: Vec<u64>,
}

// -------------------------------------------------------------------
// EditorUi —real immediate-mode UI building engine-ui Canvas elements
// -------------------------------------------------------------------

/// Immediate-mode UI helper that builds real [`engine_ui::Canvas`] elements.
///
/// Each frame the host calls [`EditorUi::begin_frame`], runs all panel
/// `ui()` methods, then calls [`EditorUi::end_frame`] to finalize the
/// canvas.  Widget results (clicks, text edits, slider drags) are
/// returned immediately based on input state provided via
/// [`set_pointer`](EditorUi::set_pointer) and the mouse transition methods.
pub struct EditorUi {
    // -- Canvas being built this frame --
    canvas: Canvas,

    // -- Input state (set by host before begin_frame) --
    cursor_x: f32,
    cursor_y: f32,
    pointer_sequence: Option<u64>,
    /// Has the left button been pressed since last frame?
    left_pressed: bool,
    /// Has the left button been released since last frame?
    left_released: bool,
    /// Is the left button currently held?
    left_down: bool,
    /// Ordered pointer, text and key transitions received since the previous
    /// UI frame. Keeping one queue is essential when a complete edit and a
    /// toolbar click both happen between redraws.
    input_events: Vec<InputEvent>,
    /// Pointer transitions resolved against the previous frame's topmost hit
    /// target and consumed by widgets in the current frame.
    frame_pointer_input_events: Vec<FramePointerInputEvent>,

    // -- Cross-frame click tracking --
    /// Label under cursor when the button was pressed.
    press_label: Option<String>,
    /// Label under cursor when the button was released. If equal to
    /// `press_label`, the widget is considered clicked in the current frame.
    release_label: Option<String>,
    /// Distinguishes a press on blank space (`None`) from no active press.
    press_capture_active: bool,
    /// Click count and last occurrence order for labels clicked this frame.
    click_results: HashMap<String, ClickRecord>,
    /// Monotonic order assigned to completed clicks. This lets mutually
    /// exclusive controls honour the user's final click when several complete
    /// between two redraws.
    next_click_order: u64,

    // -- Injected events for backward-compat testing --
    injected_events: HashMap<String, UiEvent>,

    // -- Widget hit-testing --
    /// Ordered list of (label, rect) for all interactive widgets this frame.
    widget_hit_areas: Vec<(String, UiRect)>,
    /// Per-frame occurrence count used to make repeated labels unambiguous.
    widget_occurrences: HashMap<String, usize>,

    // -- Persistent widget interaction state --
    active_text_edit: Option<TextEditState>,
    /// Final text values waiting for their immediate-mode fields to consume
    /// them. One pending value per field is sufficient because only the final
    /// value before the redraw is observable by the scene command layer.
    pending_text_commits: HashMap<String, PendingTextCommit>,
    /// Text-field IDs and source values from the previous frame. This allows
    /// ordered input replay to focus and edit a field before this frame draws.
    text_field_values: HashMap<String, String>,
    active_slider: Option<String>,
    open_color_edit: Option<String>,
    collapsing_states: HashMap<String, bool>,
    /// Trigger stamp for the most recent widget result. Panels consume this
    /// immediately when wrapping a returned editor command.
    last_interaction_stamp: Option<UiInteractionStamp>,

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
            pointer_sequence: None,
            left_pressed: false,
            left_released: false,
            left_down: false,
            input_events: Vec::new(),
            frame_pointer_input_events: Vec::new(),
            press_label: None,
            release_label: None,
            press_capture_active: false,
            click_results: HashMap::new(),
            next_click_order: 0,
            injected_events: HashMap::new(),
            widget_hit_areas: Vec::new(),
            widget_occurrences: HashMap::new(),
            active_text_edit: None,
            pending_text_commits: HashMap::new(),
            text_field_values: HashMap::new(),
            active_slider: None,
            open_color_edit: None,
            collapsing_states: HashMap::new(),
            last_interaction_stamp: None,
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
        self.click_results.clear();
        self.frame_pointer_input_events.clear();

        // Hit-test and edit against the previous frame before discarding its
        // widget map. One ordered queue preserves sequences such as
        // "click field, type, click Save" even if all of them happen between
        // two redraws.
        for event in std::mem::take(&mut self.input_events) {
            match event {
                InputEvent::Text { event, sequence } => {
                    self.apply_text_input(event, sequence);
                }
                InputEvent::Pointer(PointerInputEvent::Press {
                    position: [x, y],
                    sequence,
                }) => {
                    let pressed = self.hit_test_label_at(x, y);
                    let leaves_active_field = self
                        .active_text_edit
                        .as_ref()
                        .is_some_and(|active| pressed.as_ref() != Some(&active.id));
                    if leaves_active_field {
                        self.commit_active_text_edit(UiInteractionStamp {
                            sequence,
                            phase: UiInteractionPhase::BeforeRawPointer,
                        });
                    }
                    self.frame_pointer_input_events
                        .push(FramePointerInputEvent::Press {
                            position: [x, y],
                            target: pressed.clone(),
                            sequence,
                        });
                    self.press_label = pressed;
                    self.release_label = None;
                    self.press_capture_active = true;
                }
                InputEvent::Pointer(PointerInputEvent::Release {
                    position: [x, y],
                    sequence,
                }) => {
                    let released = self.hit_test_label_at(x, y);
                    self.frame_pointer_input_events
                        .push(FramePointerInputEvent::Release {
                            position: [x, y],
                            sequence,
                        });
                    self.release_label = released;
                    if self.press_capture_active
                        && self.press_label.as_ref() == self.release_label.as_ref()
                    {
                        if let Some(clicked) = self.press_label.clone() {
                            self.record_click_at(clicked.clone(), sequence);
                            if self
                                .active_text_edit
                                .as_ref()
                                .is_none_or(|active| active.id != clicked)
                            {
                                if let Some(previous_value) =
                                    self.text_field_values.get(&clicked).cloned()
                                {
                                    self.commit_active_text_edit(UiInteractionStamp {
                                        sequence,
                                        phase: UiInteractionPhase::AfterRawPointer,
                                    });
                                    let value = self
                                        .pending_text_commits
                                        .get(&clicked)
                                        .map(|commit| commit.value.clone())
                                        .unwrap_or(previous_value);
                                    self.active_text_edit = Some(TextEditState {
                                        id: clicked,
                                        original: value.clone(),
                                        buffer: value,
                                    });
                                }
                            }
                        }
                    }
                    self.press_label = None;
                    self.release_label = None;
                    self.press_capture_active = false;
                }
            }
        }

        self.canvas.clear();
        self.widget_hit_areas.clear();
        self.widget_occurrences.clear();
        self.text_field_values.clear();
        self.last_interaction_stamp = None;
        self.panel_left = 0.0;
        self.panel_top = 0.0;
        self.panel_width = 250.0;
        self.layout_y = self.panel_top + 4.0;
    }

    /// Call at the **end** of every editor frame, after all panel `ui()`.
    ///
    /// Finalises the canvas layout. Returns the [`Canvas`] for batch
    /// extraction.
    pub fn end_frame(&mut self) -> &Canvas {
        // After all widgets are added, do a layout pass.
        self.canvas.layout_all();

        // Reset frame-level flags for next frame.
        self.left_pressed = false;
        self.left_released = false;
        self.frame_pointer_input_events.clear();

        if !self.left_down {
            self.active_slider = None;
        }

        &self.canvas
    }

    /// Update pointer position.  Call before `begin_frame`.
    pub fn set_pointer(&mut self, x: f32, y: f32) {
        self.cursor_x = x;
        self.cursor_y = y;
    }

    /// Update pointer position with the host's shared platform-event order.
    pub fn set_pointer_with_sequence(&mut self, x: f32, y: f32, sequence: u64) {
        self.set_pointer(x, y);
        self.pointer_sequence = Some(sequence);
    }

    /// Current pointer position in window/canvas pixels.
    ///
    /// Hosts use this for viewport interactions that are not regular UI
    /// widgets, such as transform gizmos and scene picking.
    pub fn pointer_position(&self) -> [f32; 2] {
        [self.cursor_x, self.cursor_y]
    }

    /// Whether the primary pointer button is currently held.
    pub fn pointer_is_down(&self) -> bool {
        self.left_down
    }

    /// Whether the primary pointer button was pressed since the previous
    /// frame. The flag remains available until [`end_frame`](Self::end_frame).
    pub fn pointer_was_pressed(&self) -> bool {
        self.left_pressed
    }

    /// Whether the primary pointer button was released since the previous
    /// frame. The flag remains available until [`end_frame`](Self::end_frame).
    pub fn pointer_was_released(&self) -> bool {
        self.left_released
    }

    /// Whether a text field currently owns keyboard input focus.
    pub fn has_active_text_edit(&self) -> bool {
        self.active_text_edit.is_some()
    }

    /// Whether a focused or just-blurred text field contains a value that has
    /// not yet reached its owning editor panel.
    ///
    /// Hosts use this to keep Undo available while a field is still being
    /// edited. Otherwise an empty history would render Undo as non-interactive
    /// and the click that blurs the field could never undo the newly-created
    /// command in that same redraw.
    pub fn has_uncommitted_text_change(&self) -> bool {
        !self.pending_text_commits.is_empty()
            || self.active_text_edit.as_ref().is_some_and(|edit| {
                let baseline = self
                    .text_field_values
                    .get(&edit.id)
                    .map_or(edit.original.as_str(), String::as_str);
                edit.buffer != baseline
            })
    }

    /// Consume the platform stamp associated with the most recent widget
    /// result returned to a panel.
    pub fn take_last_interaction_stamp(&mut self) -> Option<UiInteractionStamp> {
        self.last_interaction_stamp.take()
    }

    /// Whether keyboard presses should be reserved for a focused or
    /// just-clicked text field.
    ///
    /// Platform hosts call this before the next redraw. It predicts focus by
    /// replaying queued pointer transitions against the previous frame's hit
    /// map, preventing a key typed immediately after clicking a field from
    /// leaking into gameplay input.
    pub fn captures_keyboard_input(&self) -> bool {
        let mut focused = self.active_text_edit.as_ref().map(|edit| edit.id.clone());
        let mut press_target = self.press_label.clone();
        let mut press_active = self.press_capture_active;

        for queued in &self.input_events {
            match queued {
                InputEvent::Pointer(PointerInputEvent::Press {
                    position: [x, y], ..
                }) => {
                    let target = self.hit_test_label_at(*x, *y);
                    if focused.as_ref() != target.as_ref() {
                        focused = None;
                    }
                    press_target = target;
                    press_active = true;
                }
                InputEvent::Pointer(PointerInputEvent::Release {
                    position: [x, y], ..
                }) => {
                    let target = self.hit_test_label_at(*x, *y);
                    if press_active && press_target.as_ref() == target.as_ref() {
                        focused = target.filter(|id| self.text_field_values.contains_key(id));
                    }
                    press_target = None;
                    press_active = false;
                }
                InputEvent::Text {
                    event: TextInputEvent::Key(UiKey::Enter | UiKey::Escape | UiKey::Tab),
                    ..
                } => focused = None,
                InputEvent::Text {
                    event:
                        TextInputEvent::Character(_)
                        | TextInputEvent::Key(UiKey::Backspace | UiKey::Delete),
                    ..
                } => {}
            }
        }
        focused.is_some()
    }

    /// Whether the pointer currently overlaps an interactive widget declared
    /// during this frame.
    ///
    /// Viewport tools should check this after panels have built their widgets
    /// so a click on an inspector or toolbar control cannot also start a scene
    /// manipulation.
    pub fn pointer_over_widget(&self) -> bool {
        self.pointer_over_widget_at(self.cursor_x, self.cursor_y)
    }

    /// Whether an arbitrary canvas position overlaps an interactive widget
    /// declared during this frame.
    ///
    /// Raw viewport input queues retain the position of each press event, so
    /// they use this method instead of testing only the latest cursor sample.
    pub fn pointer_over_widget_at(&self, x: f32, y: f32) -> bool {
        self.widget_hit_areas
            .iter()
            .rev()
            .any(|(_, rect)| rect.contains(x, y))
    }

    /// Reserve a non-interactive editor region against viewport tools.
    ///
    /// Panels often contain blank space between concrete widgets. Registering
    /// their full rectangle prevents a transform gizmo rendered underneath
    /// that space from starting a drag there. Widgets declared afterwards
    /// still win normal UI hit testing because hit areas are searched in
    /// reverse declaration order.
    pub fn block_pointer_rect(&mut self, left: f32, top: f32, width: f32, height: f32) {
        if !left.is_finite()
            || !top.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            return;
        }
        let rect = UiRect::new(left, top, width, height);
        let id = format!(
            "__pointer_block:{:08x}:{:08x}:{:08x}:{:08x}",
            left.to_bits(),
            top.to_bits(),
            width.to_bits(),
            height.to_bits()
        );
        self.widget_hit_areas.push((id, rect));
    }

    /// Keep editor coordinates and UI projection aligned with the host window.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.canvas.resize(width.max(1.0), height.max(1.0));
    }

    /// Record a mouse press event.  Call before `begin_frame`.
    pub fn set_mouse_pressed(&mut self) {
        let sequence = self.allocate_click_order();
        self.set_mouse_pressed_with_sequence(sequence);
    }

    /// Record a mouse press carrying the host's shared platform-event order.
    ///
    /// Viewport tools should receive the same `sequence`, allowing their raw
    /// events and semantic UI clicks to be replayed in one strict order.
    pub fn set_mouse_pressed_with_sequence(&mut self, sequence: u64) {
        self.left_pressed = true;
        self.left_down = true;
        self.input_events
            .push(InputEvent::Pointer(PointerInputEvent::Press {
                position: [self.cursor_x, self.cursor_y],
                sequence,
            }));
    }

    /// Record a mouse release event.  Call before `begin_frame`.
    pub fn set_mouse_released(&mut self) {
        let sequence = self.allocate_click_order();
        self.set_mouse_released_with_sequence(sequence);
    }

    /// Record a mouse release carrying the host's shared platform-event order.
    pub fn set_mouse_released_with_sequence(&mut self, sequence: u64) {
        self.left_released = true;
        self.left_down = false;
        self.input_events
            .push(InputEvent::Pointer(PointerInputEvent::Release {
                position: [self.cursor_x, self.cursor_y],
                sequence,
            }));
    }

    /// Cancel pointer capture without synthesizing a click.
    ///
    /// Use this on focus loss or when the host cancels a viewport gesture. A
    /// fake release would otherwise activate whichever button happens to be
    /// under the last known cursor position.
    pub fn cancel_pointer_interaction(&mut self) {
        self.left_pressed = false;
        self.left_released = false;
        self.left_down = false;
        self.input_events
            .retain(|event| matches!(event, InputEvent::Text { .. }));
        self.frame_pointer_input_events.clear();
        self.press_label = None;
        self.release_label = None;
        self.press_capture_active = false;
        self.click_results.clear();
        self.active_slider = None;
    }

    /// Discard any in-progress or just-blurred text edit.
    ///
    /// Hosts use this when the semantic target was deleted before its field
    /// could be revisited. Ordinary selection changes should instead render
    /// the old target once so the blurred value can be committed correctly.
    pub fn cancel_text_edit(&mut self) {
        self.active_text_edit = None;
        self.pending_text_commits.clear();
        self.input_events
            .retain(|event| matches!(event, InputEvent::Pointer(_)));
    }

    /// Queue a layout-aware character for the focused text field.
    pub fn type_character(&mut self, character: char) {
        let sequence = self.allocate_click_order();
        self.type_character_with_sequence(character, sequence);
    }

    /// Queue a character carrying the host's shared platform-event order.
    pub fn type_character_with_sequence(&mut self, character: char, sequence: u64) {
        if !character.is_control() {
            self.input_events.push(InputEvent::Text {
                event: TextInputEvent::Character(character),
                sequence,
            });
        }
    }

    /// Queue a text-editing key for the focused text field.
    pub fn press_key(&mut self, key: UiKey) {
        let sequence = self.allocate_click_order();
        self.press_key_with_sequence(key, sequence);
    }

    /// Queue a text-editing key carrying the shared platform-event order.
    pub fn press_key_with_sequence(&mut self, key: UiKey, sequence: u64) {
        self.input_events.push(InputEvent::Text {
            event: TextInputEvent::Key(key),
            sequence,
        });
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

    fn hit_test_label_at(&self, x: f32, y: f32) -> Option<String> {
        for (label, rect) in self.widget_hit_areas.iter().rev() {
            if rect.contains(x, y) {
                return Some(label.clone());
            }
        }
        None
    }

    fn allocate_click_order(&mut self) -> u64 {
        let order = self.next_click_order;
        self.next_click_order = self.next_click_order.wrapping_add(1);
        order
    }

    fn record_click_at(&mut self, id: String, order: u64) {
        let record = self.click_results.entry(id).or_default();
        record.sequences.push(order);
    }

    fn commit_active_text_edit(&mut self, stamp: UiInteractionStamp) {
        let Some(edit) = self.active_text_edit.take() else {
            return;
        };
        let baseline = self
            .text_field_values
            .get(&edit.id)
            .map_or(edit.original.as_str(), String::as_str);
        if edit.buffer == baseline {
            self.pending_text_commits.remove(&edit.id);
        } else {
            self.pending_text_commits.insert(
                edit.id,
                PendingTextCommit {
                    value: edit.buffer,
                    stamp,
                },
            );
        }
    }

    fn apply_text_input(&mut self, event: TextInputEvent, sequence: u64) {
        let mut finish = false;
        let mut cancel = false;
        if let Some(edit) = self.active_text_edit.as_mut() {
            match event {
                TextInputEvent::Character(character) => edit.buffer.push(character),
                TextInputEvent::Key(UiKey::Backspace) => {
                    edit.buffer.pop();
                }
                // The v0 editor keeps a single caret at the end, so Delete
                // has no character to remove.
                TextInputEvent::Key(UiKey::Delete) => {}
                TextInputEvent::Key(UiKey::Enter | UiKey::Tab) => finish = true,
                TextInputEvent::Key(UiKey::Escape) => cancel = true,
            }
        }
        if cancel {
            self.active_text_edit = None;
        } else if finish {
            self.commit_active_text_edit(UiInteractionStamp {
                sequence,
                phase: UiInteractionPhase::BeforeRawPointer,
            });
        }
    }

    fn click_count(&self, id: &str) -> u32 {
        self.click_results
            .get(id)
            .map_or(0, |record| record.sequences.len() as u32)
    }

    fn widget_id(&mut self, label: &str) -> String {
        let base = format!(
            "{:08x}:{:08x}:{label}",
            self.panel_left.to_bits(),
            self.panel_top.to_bits()
        );
        let occurrence = self.widget_occurrences.entry(base.clone()).or_default();
        let id = format!("{base}#{occurrence}");
        *occurrence += 1;
        id
    }

    fn push_widget(&mut self, id: &str, rect: UiRect, kind: UiElementKind) {
        self.widget_hit_areas.push((id.to_string(), rect));
        self.canvas
            .add_element(UiElement::new(kind, Self::absolute_layout(rect)).with_z_order(10));
    }

    fn add_text(&mut self, rect: UiRect, text: &str, font_size: f32, color: Color) {
        self.canvas.add_element(
            UiElement::new(
                UiElementKind::Text {
                    content: text.to_string(),
                    font_size,
                    color,
                },
                Self::absolute_layout(rect),
            )
            .with_z_order(11),
        );
    }

    fn absolute_layout(rect: UiRect) -> Layout {
        Layout::new(
            glam::Vec2::ZERO,
            glam::Vec2::ZERO,
            glam::Vec2::new(rect.x, rect.y),
            glam::Vec2::new(rect.x + rect.width, rect.y + rect.height),
        )
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

    /// Draw a non-interactive label/value row.
    pub fn label_value(&mut self, label: &str, value: &str) {
        let rect = self.widget_rect();
        self.canvas.add_element(
            UiElement::new(
                UiElementKind::Panel {
                    color: Color::new(25, 28, 36, 255),
                },
                Self::absolute_layout(rect),
            )
            .with_z_order(10),
        );
        let lr = self.label_rect();
        self.add_text(
            lr,
            &format!("{label}: {value}"),
            12.0,
            Color::new(180, 180, 180, 255),
        );
        self.advance();
    }

    /// A push button. Returns `true` when it was clicked at least once since
    /// the previous redraw.
    pub fn button(&mut self, label: &str) -> bool {
        !self.draw_button(label, true).sequences.is_empty()
    }

    /// Draw a button with a stable hit area while conditionally accepting its
    /// click.
    ///
    /// Disabled controls still participate in the previous-frame hit map.
    /// This is important for state that can become enabled because of events
    /// earlier in the same input queue, such as typing into a field and then
    /// clicking Undo before the next redraw.
    pub fn button_enabled(&mut self, label: &str, enabled: bool) -> bool {
        let clicked = !self.draw_button(label, enabled).sequences.is_empty();
        enabled && clicked
    }

    /// A push button that returns the order of its final click.
    ///
    /// Use this for mutually exclusive controls: if several buttons are
    /// clicked between redraws, the caller can select the greatest order and
    /// therefore honour the user's last choice.
    pub fn ordered_button(&mut self, label: &str) -> Option<u64> {
        let click = self.draw_button(label, true);
        click.sequences.last().copied()
    }

    /// Return every completed click in shared platform-event order.
    ///
    /// Unlike [`button`](Self::button), this does not collapse multiple clicks
    /// received between redraws. Hosts can merge these tokens with viewport
    /// input and execute Undo, Save, mode changes, and similar actions at the
    /// exact point at which the user released the button. `enabled` controls
    /// presentation only: the host must validate each token during replay,
    /// because an earlier ordered action can make a previously-disabled Redo
    /// valid before its release is reached.
    pub fn ordered_button_clicks(&mut self, label: &str, enabled: bool) -> Vec<u64> {
        let clicks = self.draw_button(label, enabled);
        clicks.sequences
    }

    /// A toggle-style button. Returns `true` only when the number of clicks
    /// since the previous redraw is odd.
    pub fn toggle_button(&mut self, label: &str) -> bool {
        self.draw_button(label, true).sequences.len() % 2 == 1
    }

    fn draw_button(&mut self, label: &str, enabled: bool) -> ClickRecord {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        // Check injected events (testing path)
        let injected_click = matches!(
            self.injected_events.remove(label),
            Some(UiEvent::ButtonClick(_))
        );

        let is_hovered = rect.contains(self.cursor_x, self.cursor_y);
        let color = if !enabled {
            Color::new(38, 42, 50, 255)
        } else if is_hovered && self.left_pressed {
            Color::new(100, 140, 200, 255)
        } else if is_hovered {
            Color::new(80, 120, 180, 255)
        } else {
            Color::new(50, 70, 100, 255)
        };

        let kind = UiElementKind::Button {
            label: label.to_string(),
            normal_color: color,
            hover_color: if enabled {
                Color::new(80, 120, 180, 255)
            } else {
                color
            },
            pressed_color: if enabled {
                Color::new(100, 140, 200, 255)
            } else {
                color
            },
            callback_id: None,
        };
        self.push_widget(&id, rect, kind);

        let lr = self.label_rect();
        self.add_text(
            lr,
            label,
            14.0,
            if enabled {
                Color::new(220, 220, 220, 255)
            } else {
                Color::new(110, 115, 125, 255)
            },
        );

        let mut click = self.click_results.get(&id).cloned().unwrap_or_default();
        if injected_click {
            let sequence = self.allocate_click_order();
            click.sequences.push(sequence);
        }
        self.last_interaction_stamp =
            click
                .sequences
                .last()
                .copied()
                .map(|sequence| UiInteractionStamp {
                    sequence,
                    phase: UiInteractionPhase::AfterRawPointer,
                });
        self.advance();
        click
    }

    /// Draw a texture-backed image in the current panel.
    ///
    /// The texture ID is forwarded into the normal `engine-ui` batch path, so
    /// the renderer can resolve an editor-owned offscreen preview exactly like
    /// any other UI texture.
    pub fn image(&mut self, texture_id: &str, height: f32) {
        let height = height.max(self.widget_height);
        let rect = UiRect::new(
            self.panel_left + 4.0,
            self.layout_y,
            self.panel_width - 8.0,
            height,
        );
        self.canvas.add_element(
            UiElement::new(
                UiElementKind::Image {
                    texture_id: texture_id.to_string(),
                    color: Color::WHITE,
                },
                Self::absolute_layout(rect),
            )
            .with_z_order(10),
        );
        self.layout_y += height + 4.0;
    }

    /// A single-line text field.  Returns `Some(edited_value)` when the
    /// user commits a change, or `None` if unchanged.
    pub fn text_field(&mut self, label: &str, value: &str) -> Option<String> {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        // Check injected events (testing path)
        let injected_commit = match self.injected_events.remove(label) {
            Some(UiEvent::TextFieldCommit(_, new_value)) => Some(PendingTextCommit {
                value: new_value,
                stamp: UiInteractionStamp {
                    sequence: self.allocate_click_order(),
                    phase: UiInteractionPhase::BeforeRawPointer,
                },
            }),
            _ => None,
        };
        let committed = injected_commit.or_else(|| self.pending_text_commits.remove(&id));
        self.last_interaction_stamp = committed.as_ref().map(|commit| commit.stamp);

        let active = self.active_text_edit.as_ref().filter(|edit| edit.id == id);
        let is_active = active.is_some();
        let shown_value = active.map_or_else(|| value.to_string(), |edit| edit.buffer.clone());

        // Background panel
        let kind = UiElementKind::Panel {
            color: if is_active {
                Color::new(45, 55, 75, 255)
            } else {
                Color::new(30, 30, 40, 255)
            },
        };
        self.push_widget(&id, rect, kind);

        let lr = self.label_rect();
        let cursor = if is_active { "|" } else { "" };
        self.add_text(
            lr,
            &format!("{label}: {shown_value}{cursor}"),
            12.0,
            Color::new(180, 180, 180, 255),
        );

        self.text_field_values.insert(
            id,
            committed
                .as_ref()
                .map_or_else(|| value.to_string(), |commit| commit.value.clone()),
        );
        self.advance();
        committed.map(|commit| commit.value)
    }

    /// A horizontal slider for `f32` values.
    pub fn slider_f32(&mut self, label: &str, value: f32, min: f32, max: f32) -> Option<f32> {
        self.ordered_slider_f32(label, value, min, max)
            .last()
            .map(|(_, value)| *value)
            .filter(|dragged| (*dragged - value).abs() > f32::EPSILON)
    }

    /// A horizontal slider that preserves every sampled value and its shared
    /// platform-event stamp.
    ///
    /// The ordinary [`slider_f32`](Self::slider_f32) API exposes only the
    /// final value for compatibility. Hosts that merge panel state with raw
    /// viewport input use this form so two completed slider gestures separated
    /// by a gizmo gesture in the same redraw remain independently replayable.
    pub fn ordered_slider_f32(
        &mut self,
        label: &str,
        value: f32,
        min: f32,
        max: f32,
    ) -> Vec<(UiInteractionStamp, f32)> {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        // Check injected events (testing path)
        if let Some(UiEvent::SliderDrag(_, new_val)) = self.injected_events.remove(label) {
            let stamp = UiInteractionStamp {
                sequence: self.allocate_click_order(),
                phase: UiInteractionPhase::AfterRawPointer,
            };
            let new_val = new_val.clamp(min, max);
            self.last_interaction_stamp = Some(stamp);
            let kind = UiElementKind::Slider {
                label: label.to_string(),
                value: new_val,
                min,
                max,
                callback_id: None,
            };
            self.push_widget(&id, rect, kind);
            self.advance();
            return vec![(stamp, new_val)];
        }

        let mut captured = self
            .active_slider
            .as_ref()
            .is_some_and(|active| active == &id);
        let mut samples = Vec::new();
        for event in self.frame_pointer_input_events.clone() {
            match event {
                FramePointerInputEvent::Press {
                    position: [x, _],
                    target,
                    sequence,
                } if !captured && target.as_deref() == Some(id.as_str()) => {
                    captured = true;
                    samples.push((x, sequence));
                    self.active_slider = Some(id.clone());
                }
                FramePointerInputEvent::Release {
                    position: [x, _],
                    sequence,
                } if captured => {
                    samples.push((x, sequence));
                    captured = false;
                    if self.active_slider.as_deref() == Some(id.as_str()) {
                        self.active_slider = None;
                    }
                }
                FramePointerInputEvent::Press { .. } | FramePointerInputEvent::Release { .. } => {}
            }
        }
        if captured && self.left_down {
            let sample = (
                self.cursor_x,
                self.pointer_sequence.unwrap_or(self.next_click_order),
            );
            if samples.last().copied() != Some(sample) {
                samples.push(sample);
            }
            self.active_slider = Some(id.clone());
        }

        let mut previous = value;
        let mut dragged_values = Vec::new();
        if min.is_finite() && max.is_finite() && max > min {
            for (sampled_x, sequence) in samples {
                let ratio = ((sampled_x - rect.x) / rect.width.max(1.0)).clamp(0.0, 1.0);
                let sampled_value = min + (max - min) * ratio;
                if (sampled_value - previous).abs() > f32::EPSILON {
                    dragged_values.push((
                        UiInteractionStamp {
                            sequence,
                            phase: UiInteractionPhase::AfterRawPointer,
                        },
                        sampled_value,
                    ));
                    previous = sampled_value;
                }
            }
        }
        if let Some((stamp, _)) = dragged_values.last() {
            self.last_interaction_stamp = Some(*stamp);
        }
        let displayed_value = dragged_values.last().map_or(value, |(_, value)| *value);

        let kind = UiElementKind::Slider {
            label: label.to_string(),
            value: displayed_value,
            min,
            max,
            callback_id: None,
        };
        self.push_widget(&id, rect, kind);

        let lr = self.label_rect();
        self.add_text(
            lr,
            &format!("{label}: {displayed_value:.3}"),
            12.0,
            Color::new(180, 180, 180, 255),
        );

        self.advance();
        dragged_values
    }

    /// A checkbox.  Returns the *new* checked state.
    pub fn checkbox(&mut self, label: &str, checked: bool) -> bool {
        self.ordered_checkbox_changes(label, checked)
            .last()
            .map_or(checked, |(_, checked)| *checked)
    }

    /// A checkbox that returns every toggled state in shared platform order.
    pub fn ordered_checkbox_changes(
        &mut self,
        label: &str,
        checked: bool,
    ) -> Vec<(UiInteractionStamp, bool)> {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        // Check injected events (testing path)
        if let Some(UiEvent::CheckboxToggle(_, new_state)) = self.injected_events.remove(label) {
            let stamp = UiInteractionStamp {
                sequence: self.allocate_click_order(),
                phase: UiInteractionPhase::AfterRawPointer,
            };
            self.last_interaction_stamp = Some(stamp);
            let kind = UiElementKind::Checkbox {
                label: label.to_string(),
                checked: new_state,
                color: Color::new(100, 180, 255, 255),
                callback_id: None,
            };
            self.push_widget(&id, rect, kind);
            self.advance();
            return vec![(stamp, new_state)];
        }

        let mut current = checked;
        let changes = self
            .click_results
            .get(&id)
            .map(|click| {
                click
                    .sequences
                    .iter()
                    .copied()
                    .map(|sequence| {
                        current = !current;
                        (
                            UiInteractionStamp {
                                sequence,
                                phase: UiInteractionPhase::AfterRawPointer,
                            },
                            current,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some((stamp, _)) = changes.last() {
            self.last_interaction_stamp = Some(*stamp);
        }

        let kind = UiElementKind::Checkbox {
            label: label.to_string(),
            checked: current,
            color: Color::new(100, 180, 255, 255),
            callback_id: None,
        };
        self.push_widget(&id, rect, kind);

        let lr = self.label_rect();
        self.add_text(
            UiRect::new(lr.x + 20.0, lr.y, lr.width - 20.0, lr.height),
            label,
            12.0,
            Color::new(180, 180, 180, 255),
        );

        self.advance();
        changes
    }

    /// A simple color picker.  Returns `Some(new_color)` when the user
    /// adjusts the colour, or `None` if unchanged.
    pub fn color_edit(&mut self, label: &str, color: [f32; 4]) -> Option<[f32; 4]> {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        // Check injected events (testing path)
        if let Some(UiEvent::ColorPick(_, new_color)) = self.injected_events.remove(label) {
            self.last_interaction_stamp = Some(UiInteractionStamp {
                sequence: self.allocate_click_order(),
                phase: UiInteractionPhase::AfterRawPointer,
            });
            self.advance();
            return Some(new_color);
        }

        if self.click_count(&id) % 2 == 1 {
            if self.open_color_edit.as_deref() == Some(id.as_str()) {
                self.open_color_edit = None;
            } else {
                self.open_color_edit = Some(id.clone());
            }
        }

        let c = Color::new(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        );

        let kind = UiElementKind::Panel { color: c };
        self.push_widget(&id, rect, kind);

        let lr = self.label_rect();
        self.add_text(
            lr,
            &format!("{label} [{},{},{},{}]", c.r, c.g, c.b, c.a),
            11.0,
            Color::new(255, 255, 255, 255),
        );

        self.advance();

        if self.open_color_edit.as_deref() != Some(id.as_str()) {
            return None;
        }

        let mut edited = color;
        let mut changed = false;
        for (channel, suffix) in ["R", "G", "B", "A"].into_iter().enumerate() {
            if let Some(value) =
                self.slider_f32(&format!("{label}.{suffix}"), edited[channel], 0.0, 1.0)
            {
                edited[channel] = value;
                changed = true;
            }
        }
        changed.then_some(edited)
    }

    /// A horizontal separator line.
    pub fn separator(&mut self) {
        let rect = UiRect::new(
            self.panel_left + 4.0,
            self.layout_y,
            self.panel_width - 8.0,
            1.0,
        );
        self.canvas.add_element(
            UiElement::new(
                UiElementKind::Panel {
                    color: Color::new(60, 60, 70, 255),
                },
                Self::absolute_layout(rect),
            )
            .with_z_order(5),
        );
        self.layout_y += 6.0;
    }

    /// A collapsible header section.  Returns `true` when expanded.
    pub fn collapsing_header(&mut self, label: &str, default_open: bool) -> bool {
        let rect = self.widget_rect();
        let id = self.widget_id(label);

        let is_hovered = rect.contains(self.cursor_x, self.cursor_y);
        let bg_color = if is_hovered {
            Color::new(50, 60, 80, 255)
        } else {
            Color::new(35, 40, 55, 255)
        };

        let kind = UiElementKind::Panel { color: bg_color };
        self.push_widget(&id, rect, kind);

        let mut open = *self
            .collapsing_states
            .entry(id.clone())
            .or_insert(default_open);

        // Check injected events (testing path)
        if let Some(UiEvent::ButtonClick(_)) = self.injected_events.remove(label) {
            open = !open;
            self.collapsing_states.insert(id, open);
            self.advance();
            return open;
        }

        let lr = self.label_rect();
        self.add_text(lr, label, 13.0, Color::new(200, 200, 210, 255));

        // Track toggle state
        let was_clicked = self.click_count(&id) % 2 == 1;
        if was_clicked {
            open = !open;
            self.collapsing_states.insert(id, open);
        }
        self.advance();
        open
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_press_release_sequence_emits_one_click() {
        let mut ui = EditorUi::new();
        ui.resize(800.0, 600.0);
        ui.set_pointer(10.0, 10.0);

        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();

        ui.set_mouse_pressed();
        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();

        ui.set_mouse_released();
        ui.begin_frame();
        assert!(ui.button("Play"));
        ui.end_frame();

        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();
    }

    #[test]
    fn complete_click_between_redraws_uses_recorded_positions() {
        let mut ui = EditorUi::new();
        ui.resize(800.0, 600.0);
        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();

        ui.begin_frame();
        assert!(ui.button("Play"));
        ui.end_frame();
    }

    #[test]
    fn viewport_drag_released_over_button_does_not_click_button() {
        let mut ui = EditorUi::new();
        ui.resize(800.0, 600.0);
        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();

        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed();
        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_released();

        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();

        ui.begin_frame();
        assert!(!ui.button("Play"));
        ui.end_frame();
    }

    #[test]
    fn focus_loss_cancels_pressed_widget_without_clicking_it() {
        let mut ui = EditorUi::new();
        ui.set_pointer(10.0, 10.0);
        ui.begin_frame();
        assert!(!ui.button("Delete"));
        ui.end_frame();

        ui.set_mouse_pressed();
        ui.begin_frame();
        assert!(!ui.button("Delete"));
        ui.end_frame();

        ui.cancel_pointer_interaction();
        ui.begin_frame();
        assert!(!ui.button("Delete"));
        ui.end_frame();
        assert!(!ui.pointer_is_down());
    }

    #[test]
    fn end_frame_preserves_absolute_widget_layout() {
        let mut ui = EditorUi::new();
        ui.resize(800.0, 600.0);
        ui.begin_frame();
        ui.set_panel_rect(100.0, 20.0, 200.0);
        ui.button("Save");

        let canvas = ui.end_frame();
        let button = canvas
            .elements
            .iter()
            .find(|element| matches!(element.kind, UiElementKind::Button { .. }))
            .expect("button element");
        assert_eq!(button.rect, UiRect::new(104.0, 24.0, 192.0, 22.0));

        let batches = canvas.build_batches();
        assert!(!batches.is_empty());
        assert!(batches
            .iter()
            .all(|batch| batch.clip_rect.max == [800.0, 600.0]));
    }

    #[test]
    fn image_widget_emits_a_texture_backed_ui_batch() {
        let mut ui = EditorUi::new();
        ui.resize(320.0, 240.0);
        ui.begin_frame();
        ui.image("editor/material-preview/7", 96.0);
        let canvas = ui.end_frame();

        let image = canvas
            .elements
            .iter()
            .find(|element| matches!(element.kind, UiElementKind::Image { .. }))
            .expect("image element");
        assert_eq!(image.rect.height, 96.0);
        assert!(canvas.build_batches().iter().any(|batch| {
            batch
                .texture
                .as_ref()
                .is_some_and(|texture| texture.id == "editor/material-preview/7")
        }));
    }

    fn draw_text_field(ui: &mut EditorUi, value: &str) -> Option<String> {
        ui.begin_frame();
        let result = ui.text_field("Name", value);
        ui.end_frame();
        result
    }

    fn focus_default_text_field(ui: &mut EditorUi, value: &str) {
        ui.set_pointer(10.0, 10.0);
        assert_eq!(draw_text_field(ui, value), None);
        ui.set_mouse_pressed();
        assert_eq!(draw_text_field(ui, value), None);
        ui.set_mouse_released();
        assert_eq!(draw_text_field(ui, value), None);
    }

    #[test]
    fn text_field_accepts_ordered_unicode_input_and_commits_once() {
        let mut ui = EditorUi::new();
        focus_default_text_field(&mut ui, "Player");

        ui.type_character('中');
        ui.type_character('a');
        ui.press_key(UiKey::Backspace);
        ui.press_key(UiKey::Enter);
        assert_eq!(draw_text_field(&mut ui, "Player"), Some("Player中".into()));
        assert_eq!(draw_text_field(&mut ui, "Player中"), None);
    }

    #[test]
    fn text_field_escape_cancels_and_blank_click_commits() {
        let mut ui = EditorUi::new();
        focus_default_text_field(&mut ui, "Player");
        ui.type_character('X');
        ui.press_key(UiKey::Escape);
        assert_eq!(draw_text_field(&mut ui, "Player"), None);

        focus_default_text_field(&mut ui, "Player");
        ui.type_character('Y');
        assert_eq!(draw_text_field(&mut ui, "Player"), None);
        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed();
        assert_eq!(draw_text_field(&mut ui, "Player"), Some("PlayerY".into()));
    }

    #[test]
    fn cancelled_text_edit_cannot_spill_into_reused_widget_id() {
        let mut ui = EditorUi::new();
        focus_default_text_field(&mut ui, "Material A");
        ui.type_character('X');
        assert_eq!(draw_text_field(&mut ui, "Material A"), None);
        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed();
        ui.begin_frame();
        ui.cancel_text_edit();
        assert_eq!(ui.text_field("Name", "Material B"), None);
        ui.end_frame();
    }

    #[test]
    fn same_label_in_different_panels_keeps_focus_isolated() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Left"), None);
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Right"), None);
        ui.end_frame();

        ui.set_pointer(310.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        ui.type_character('!');
        ui.press_key(UiKey::Enter);
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Left"), None);
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Right"), Some("Right!".into()));
        ui.end_frame();
    }

    #[test]
    fn slider_captures_pointer_until_release() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        assert_eq!(ui.slider_f32("Speed", 0.0, 0.0, 1.0), None);
        ui.end_frame();

        ui.set_pointer(125.0, 10.0);
        ui.set_mouse_pressed();
        ui.begin_frame();
        let middle = ui.slider_f32("Speed", 0.0, 0.0, 1.0).unwrap();
        ui.end_frame();
        assert!((0.45..0.55).contains(&middle));

        ui.set_pointer(500.0, 500.0);
        ui.begin_frame();
        assert_eq!(ui.slider_f32("Speed", middle, 0.0, 1.0), Some(1.0));
        ui.end_frame();

        ui.set_mouse_released();
        ui.begin_frame();
        assert_eq!(ui.slider_f32("Speed", 1.0, 0.0, 1.0), None);
        ui.end_frame();
    }

    #[test]
    fn ordered_slider_preserves_multiple_gestures_between_redraws() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert!(ui.ordered_slider_f32("Speed", 0.0, 0.0, 1.0).is_empty());
        ui.end_frame();

        ui.set_pointer(60.0, 10.0);
        ui.set_mouse_pressed_with_sequence(10);
        ui.set_pointer(80.0, 10.0);
        ui.set_mouse_released_with_sequence(11);
        ui.set_pointer(160.0, 10.0);
        ui.set_mouse_pressed_with_sequence(20);
        ui.set_pointer(180.0, 10.0);
        ui.set_mouse_released_with_sequence(21);

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        let changes = ui.ordered_slider_f32("Speed", 0.0, 0.0, 1.0);
        ui.end_frame();
        assert_eq!(
            changes
                .iter()
                .map(|(stamp, _)| (stamp.sequence, stamp.phase))
                .collect::<Vec<_>>(),
            [
                (10, UiInteractionPhase::AfterRawPointer),
                (11, UiInteractionPhase::AfterRawPointer),
                (20, UiInteractionPhase::AfterRawPointer),
                (21, UiInteractionPhase::AfterRawPointer),
            ]
        );
        assert!(changes.windows(2).all(|pair| pair[0].1 < pair[1].1));
    }

    #[test]
    fn ordered_replay_commits_text_before_a_later_save_click() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Player"), None);
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(!ui.button("Save"));
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        ui.type_character('X');
        ui.set_pointer(310.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Player"), Some("PlayerX".into()));
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(ui.button("Save"));
        ui.end_frame();
    }

    #[test]
    fn disabled_undo_hit_area_can_activate_after_an_interframe_text_edit() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(!ui.button_enabled("Undo", false));
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Player"), None);
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        ui.type_character('X');
        ui.set_pointer(310.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();

        ui.begin_frame();
        assert!(ui.has_uncommitted_text_change());
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(ui.button_enabled("Undo", true));
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.text_field("Name", "Player"), Some("PlayerX".into()));
        ui.end_frame();
    }

    #[test]
    fn typed_text_before_blank_click_commits_in_the_same_redraw() {
        let mut ui = EditorUi::new();
        focus_default_text_field(&mut ui, "Player");

        ui.type_character('Z');
        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();

        assert_eq!(draw_text_field(&mut ui, "Player"), Some("PlayerZ".into()));
    }

    #[test]
    fn pending_text_field_click_captures_keyboard_before_redraw() {
        let mut ui = EditorUi::new();
        assert_eq!(draw_text_field(&mut ui, "Player"), None);
        assert!(!ui.captures_keyboard_input());

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        assert!(ui.captures_keyboard_input());

        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        assert!(!ui.captures_keyboard_input());
    }

    #[test]
    fn double_checkbox_click_preserves_original_state() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        assert!(!ui.checkbox("Enabled", false));
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        for _ in 0..2 {
            ui.set_mouse_pressed();
            ui.set_mouse_released();
        }

        ui.begin_frame();
        assert!(!ui.checkbox("Enabled", false));
        ui.end_frame();
    }

    #[test]
    fn ordered_checkbox_preserves_each_toggle_between_redraws() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        assert!(ui.ordered_checkbox_changes("Enabled", false).is_empty());
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed_with_sequence(30);
        ui.set_mouse_released_with_sequence(31);
        ui.set_mouse_pressed_with_sequence(40);
        ui.set_mouse_released_with_sequence(41);

        ui.begin_frame();
        let changes = ui.ordered_checkbox_changes("Enabled", false);
        ui.end_frame();
        assert_eq!(
            changes,
            [
                (
                    UiInteractionStamp {
                        sequence: 31,
                        phase: UiInteractionPhase::AfterRawPointer,
                    },
                    true,
                ),
                (
                    UiInteractionStamp {
                        sequence: 41,
                        phase: UiInteractionPhase::AfterRawPointer,
                    },
                    false,
                ),
            ]
        );
    }

    #[test]
    fn ordered_buttons_report_the_users_final_choice() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.ordered_button("Rotate"), None);
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert_eq!(ui.ordered_button("Scale"), None);
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        ui.set_pointer(310.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        let rotate = ui.ordered_button("Rotate").expect("Rotate click");
        ui.set_panel_rect(300.0, 0.0, 250.0);
        let scale = ui.ordered_button("Scale").expect("Scale click");
        ui.end_frame();
        assert!(scale > rotate);
    }

    #[test]
    fn ordered_button_clicks_preserve_every_host_release_sequence() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert!(ui.ordered_button_clicks("Undo", true).is_empty());
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed_with_sequence(10);
        ui.set_mouse_released_with_sequence(11);
        ui.set_mouse_pressed_with_sequence(20);
        ui.set_mouse_released_with_sequence(21);

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.ordered_button_clicks("Undo", true), vec![11, 21]);
        ui.end_frame();
    }

    #[test]
    fn ordered_disabled_button_retains_token_for_replay_time_validation() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert!(ui.ordered_button_clicks("Redo", false).is_empty());
        ui.end_frame();

        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed_with_sequence(30);
        ui.set_mouse_released_with_sequence(31);

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.ordered_button_clicks("Redo", false), vec![31]);
        ui.end_frame();
    }

    #[test]
    fn text_blur_stamp_precedes_raw_pointer_at_the_same_sequence() {
        let mut ui = EditorUi::new();
        focus_default_text_field(&mut ui, "Player");
        ui.type_character_with_sequence('X', 40);
        ui.set_pointer(500.0, 500.0);
        ui.set_mouse_pressed_with_sequence(50);

        ui.begin_frame();
        assert_eq!(ui.text_field("Name", "Player"), Some("PlayerX".into()));
        assert_eq!(
            ui.take_last_interaction_stamp(),
            Some(UiInteractionStamp {
                sequence: 50,
                phase: UiInteractionPhase::BeforeRawPointer,
            })
        );
        ui.end_frame();
    }

    #[test]
    fn button_stamp_follows_raw_release_at_the_same_sequence() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        assert!(!ui.button("Apply"));
        ui.end_frame();
        ui.set_pointer(10.0, 10.0);
        ui.set_mouse_pressed_with_sequence(60);
        ui.set_mouse_released_with_sequence(61);

        ui.begin_frame();
        assert!(ui.button("Apply"));
        assert_eq!(
            ui.take_last_interaction_stamp(),
            Some(UiInteractionStamp {
                sequence: 61,
                phase: UiInteractionPhase::AfterRawPointer,
            })
        );
        ui.end_frame();
    }

    #[test]
    fn completed_slider_drag_uses_its_release_before_a_later_button_click() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        assert_eq!(ui.slider_f32("Speed", 0.0, 0.0, 1.0), None);
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(!ui.button("Apply"));
        ui.end_frame();

        ui.set_pointer(125.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_pointer(64.5, 10.0);
        ui.set_mouse_released();
        ui.set_pointer(310.0, 10.0);
        ui.set_mouse_pressed();
        ui.set_mouse_released();
        ui.set_pointer(700.0, 500.0);

        ui.begin_frame();
        ui.set_panel_rect(0.0, 0.0, 250.0);
        let speed = ui.slider_f32("Speed", 0.0, 0.0, 1.0).expect("slider drag");
        ui.set_panel_rect(300.0, 0.0, 250.0);
        assert!(ui.button("Apply"));
        ui.end_frame();
        assert!((speed - 0.25).abs() < 0.01, "speed was {speed}");
    }

    #[test]
    fn collapsing_header_state_persists_across_frames() {
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.inject_event(UiEvent::ButtonClick("Transform".into()));
        assert!(!ui.collapsing_header("Transform", true));
        ui.end_frame();

        ui.begin_frame();
        assert!(!ui.collapsing_header("Transform", true));
        ui.end_frame();
    }
}
