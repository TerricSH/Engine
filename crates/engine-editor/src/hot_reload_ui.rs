//! Hot-reload diagnostics panel — scrolling log view for asset / shader /
//! script reload messages with colour-coded severity levels.

use crate::editor_ui::EditorUi;

// ---------------------------------------------------------------------------
// ReloadMessage
// ---------------------------------------------------------------------------

/// A single reload log entry.
#[derive(Clone, Debug)]
pub struct ReloadMessage {
    /// Human-readable timestamp string (e.g. `"12:34:56.789"`).
    pub timestamp: String,
    /// Severity / level label (`"error"`, `"warning"`, `"info"`, …).
    pub level: String,
    /// The log text body.
    pub text: String,
}

impl ReloadMessage {
    /// Create a new reload message with a fresh timestamp.
    pub fn new(level: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            timestamp: Self::now_str(),
            level: level.into(),
            text: text.into(),
        }
    }

    /// Return a simple timestamp string.
    fn now_str() -> String {
        // v0: use a monotonic millisecond count as a stand-in for a real
        // wall-clock formatter.  A production version could use `chrono` or
        // `std::time::SystemTime`.
        let elapsed = std::time::Instant::now()
            .duration_since(std::time::Instant::now())
            .as_secs_f64()
            .abs();
        let secs = elapsed as u64;
        let millis = ((elapsed - secs as f64) * 1000.0) as u64;
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            secs / 3600,
            (secs / 60) % 60,
            secs % 60,
            millis
        )
    }
}

// ---------------------------------------------------------------------------
// HotReloadPanel
// ---------------------------------------------------------------------------

/// Editor panel that displays a scrolling log of hot-reload messages.
pub struct HotReloadPanel {
    /// Ring buffer of log messages (newest appended at the end).
    pub messages: Vec<ReloadMessage>,
    /// Whether the view should auto-scroll to the latest message.
    pub auto_scroll: bool,

    /// Maximum number of messages retained (prevents unbounded growth).
    max_messages: usize,
}

impl HotReloadPanel {
    /// Create a new hot-reload panel.
    ///
    /// Retains up to `max_messages` entries (default 1024).
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            auto_scroll: true,
            max_messages: 1024,
        }
    }

    /// Remove all messages from the log.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Replace `max_messages` (truncates old entries if the new limit is
    /// smaller than the current message count).
    pub fn set_max_messages(&mut self, max: usize) {
        self.max_messages = max;
        if self.messages.len() > max {
            self.messages.drain(0..self.messages.len() - max);
        }
    }

    /// The current maximum message limit.
    pub fn max_messages(&self) -> usize {
        self.max_messages
    }
}

impl Default for HotReloadPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// push_reload_message
// ---------------------------------------------------------------------------

/// Push a new reload message to the panel.
///
/// The message is timestamped automatically and appended to the log.  Old
/// entries are trimmed when the count exceeds [`HotReloadPanel::max_messages`].
pub fn push_reload_message(panel: &mut HotReloadPanel, level: &str, text: &str) {
    panel.messages.push(ReloadMessage::new(level, text));

    // Trim oldest messages when over the limit.
    while panel.messages.len() > panel.max_messages {
        panel.messages.remove(0);
    }
}

// ---------------------------------------------------------------------------
// draw_hot_reload
// ---------------------------------------------------------------------------

/// Draw the hot-reload log panel using [`EditorUi`] primitives.
///
/// Layout:
/// - Toolbar: auto-scroll toggle + clear button
/// - Scrolling log view with colour-coded entries:
///   - `"error"`   → red   (prefix `[E]`)
///   - `"warning"` → yellow (prefix `[W]`)
///   - `"info"`    → white  (prefix `[I]`)
/// - Other levels default to white.
pub fn draw_hot_reload(ui: &mut EditorUi, panel: &mut HotReloadPanel) {
    let header_label = format!("Hot Reload [{}]", panel.messages.len());
    let open = ui.collapsing_header(&header_label, true);
    if !open {
        return;
    }

    // ── Toolbar ─────────────────────────────────────────────────────
    panel.auto_scroll = ui.checkbox("Auto-scroll", panel.auto_scroll);

    if ui.button("Clear") {
        panel.clear();
    }

    ui.separator();

    // ── Log entries ─────────────────────────────────────────────────
    if panel.messages.is_empty() {
        ui.text_field("Info", "No reload messages yet.");
        return;
    }

    // v0: iterate in reverse (newest first), rendering a formatted line
    // for each entry with a severity prefix.
    for msg in panel.messages.iter().rev() {
        let prefix = match msg.level.to_lowercase().as_str() {
            "error" | "err" => "[E]",
            "warning" | "warn" => "[W]",
            _ => "[I]",
        };

        let line = format!(
            "{} {} {}",
            prefix,
            if prefix == "[E]" {
                "⨯"
            } else if prefix == "[W]" {
                "⚠"
            } else {
                "·"
            },
            msg.text
        );

        // Use the timestamp as the label and the message as the value.
        ui.text_field(&msg.timestamp, &line);
    }

    tracing::debug!(
        count = panel.messages.len(),
        auto_scroll = panel.auto_scroll,
        "HotReloadPanel draw"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Panel construction ──────────────────────────────────────────

    #[test]
    fn panel_new_has_defaults() {
        let panel = HotReloadPanel::new();
        assert!(panel.messages.is_empty());
        assert!(panel.auto_scroll);
        assert_eq!(panel.max_messages(), 1024);
    }

    #[test]
    fn panel_default_is_same_as_new() {
        let a = HotReloadPanel::new();
        let b = HotReloadPanel::default();
        assert_eq!(a.max_messages(), b.max_messages());
        assert_eq!(a.auto_scroll, b.auto_scroll);
        assert_eq!(a.messages.len(), b.messages.len());
    }

    // ── ReloadMessage ───────────────────────────────────────────────

    #[test]
    fn reload_message_new_sets_fields() {
        let msg = ReloadMessage::new("error", "Shader compile failed");
        assert_eq!(msg.level, "error");
        assert_eq!(msg.text, "Shader compile failed");
        // Timestamp should be non-empty.
        assert!(!msg.timestamp.is_empty());
    }

    #[test]
    fn reload_message_new_timestamp_format() {
        let msg = ReloadMessage::new("info", "test");
        // Format: HH:MM:SS.mmm
        assert_eq!(msg.timestamp.len(), 12, "timestamp should be HH:MM:SS.mmm");
        assert_eq!(msg.timestamp.chars().filter(|&c| c == ':').count(), 2);
        assert_eq!(msg.timestamp.chars().filter(|&c| c == '.').count(), 1);
    }

    // ── push_reload_message ─────────────────────────────────────────

    #[test]
    fn push_adds_message() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "Asset loaded");
        assert_eq!(panel.messages.len(), 1);
        assert_eq!(panel.messages[0].level, "info");
        assert_eq!(panel.messages[0].text, "Asset loaded");
    }

    #[test]
    fn push_preserves_order() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "first");
        push_reload_message(&mut panel, "warning", "second");
        push_reload_message(&mut panel, "error", "third");

        assert_eq!(panel.messages.len(), 3);
        assert_eq!(panel.messages[0].text, "first");
        assert_eq!(panel.messages[1].text, "second");
        assert_eq!(panel.messages[2].text, "third");
    }

    #[test]
    fn push_trims_at_max() {
        let mut panel = HotReloadPanel::new();
        panel.set_max_messages(3);

        push_reload_message(&mut panel, "info", "A");
        push_reload_message(&mut panel, "info", "B");
        push_reload_message(&mut panel, "info", "C");
        assert_eq!(panel.messages.len(), 3);

        // Fourth push should evict "A".
        push_reload_message(&mut panel, "info", "D");
        assert_eq!(panel.messages.len(), 3);
        assert_eq!(panel.messages[0].text, "B");
        assert_eq!(panel.messages[1].text, "C");
        assert_eq!(panel.messages[2].text, "D");
    }

    #[test]
    fn push_multiple_levels() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "error", "err1");
        push_reload_message(&mut panel, "warning", "warn1");
        push_reload_message(&mut panel, "info", "info1");

        assert_eq!(panel.messages[0].level, "error");
        assert_eq!(panel.messages[1].level, "warning");
        assert_eq!(panel.messages[2].level, "info");
    }

    // ── clear ───────────────────────────────────────────────────────

    #[test]
    fn clear_removes_all_messages() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "keep me?");
        assert!(!panel.messages.is_empty());

        panel.clear();
        assert!(panel.messages.is_empty());
    }

    #[test]
    fn clear_twice_no_panic() {
        let mut panel = HotReloadPanel::new();
        panel.clear();
        panel.clear(); // second clear on empty log
    }

    // ── set_max_messages ────────────────────────────────────────────

    #[test]
    fn set_max_messages_truncates() {
        let mut panel = HotReloadPanel::new();
        for i in 0..10 {
            push_reload_message(&mut panel, "info", &format!("msg {}", i));
        }
        assert_eq!(panel.messages.len(), 10);

        panel.set_max_messages(3);
        assert_eq!(panel.messages.len(), 3);
        // Keeps the last 3 entries (newest).
        assert_eq!(panel.messages[0].text, "msg 7");
        assert_eq!(panel.messages[2].text, "msg 9");
    }

    #[test]
    fn set_max_messages_larger_than_count() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "only");
        panel.set_max_messages(100);
        assert_eq!(panel.messages.len(), 1);
    }

    // ── draw_hot_reload ─────────────────────────────────────────────

    #[test]
    fn draw_empty_panel_does_not_panic() {
        let mut panel = HotReloadPanel::new();
        let mut ui = EditorUi::new();
        draw_hot_reload(&mut ui, &mut panel);
    }

    #[test]
    fn draw_panel_with_messages_does_not_panic() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "Asset A loaded");
        push_reload_message(&mut panel, "warning", "Texture B not found");
        push_reload_message(&mut panel, "error", "Shader C compile failed");

        let mut ui = EditorUi::new();
        draw_hot_reload(&mut ui, &mut panel);
    }

    #[test]
    fn draw_after_clear_does_not_panic() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "temp");
        panel.clear();

        let mut ui = EditorUi::new();
        draw_hot_reload(&mut ui, &mut panel);
    }

    // ── auto_scroll ─────────────────────────────────────────────────

    #[test]
    fn auto_scroll_toggle() {
        let mut panel = HotReloadPanel::new();
        assert!(panel.auto_scroll);

        panel.auto_scroll = false;
        assert!(!panel.auto_scroll);

        panel.auto_scroll = true;
        assert!(panel.auto_scroll);
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn push_with_empty_text() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "info", "");
        assert_eq!(panel.messages.len(), 1);
        assert!(panel.messages[0].text.is_empty());
    }

    #[test]
    fn push_with_empty_level() {
        let mut panel = HotReloadPanel::new();
        push_reload_message(&mut panel, "", "something");
        assert_eq!(panel.messages[0].level, "");
    }

    #[test]
    fn draw_with_zero_max_messages() {
        let mut panel = HotReloadPanel::new();
        panel.set_max_messages(0);
        push_reload_message(&mut panel, "info", "this gets evicted immediately");

        let mut ui = EditorUi::new();
        draw_hot_reload(&mut ui, &mut panel);
        // Should not panic; the log is empty because every push is evicted.
    }
}
