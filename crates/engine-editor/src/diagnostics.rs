//! Unified diagnostics panel for the editor.
//!
//! Collects diagnostics from various subsystems (build, script reload, asset
//! loading, …) and renders them in a scrollable, filterable list.
//!
//! Each entry carries a capture timestamp and can be expanded to show the
//! full diagnostic detail.

use std::time::Instant;

use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::commands::Command;
use crate::editor_ui::EditorUi;

// ---------------------------------------------------------------------------
// DiagnosticEntry
// ---------------------------------------------------------------------------

/// A single diagnostic with a capture timestamp.
#[derive(Clone, Debug)]
pub struct DiagnosticEntry {
    /// The underlying diagnostic data.
    pub diagnostic: Diagnostic,
    /// When this entry was recorded.
    pub timestamp: Instant,
}

impl DiagnosticEntry {
    /// Wrap a [`Diagnostic`] with the current timestamp.
    pub fn new(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostic,
            timestamp: Instant::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// DiagnosticsPanel
// ---------------------------------------------------------------------------

/// Editor panel that displays a scrollable, filterable list of diagnostics.
///
/// Each entry shows the diagnostic code, a severity icon (colour-coded), the
/// message text, the originating system, and contextual information such as
/// the associated file path, entity, or asset.
///
/// Diagnostics can be filtered by severity.  Click the "▼ Detail" button on
/// any entry to expand the full diagnostic record.
pub struct DiagnosticsPanel {
    visible: bool,
    name: String,
    /// The full list of diagnostic entries held by this panel.
    entries: Vec<DiagnosticEntry>,
    /// If set, only diagnostics matching this severity are shown.
    filter: Option<DiagnosticSeverity>,
    /// Index of the entry whose details are currently expanded.
    expanded: Option<usize>,
}

impl DiagnosticsPanel {
    /// Create a new diagnostics panel with the given display name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            entries: Vec::new(),
            filter: None,
            expanded: None,
        }
    }

    // ── Public query methods ───────────────────────────────────────────

    /// The display name of this panel.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the panel is visible.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Show or hide the panel.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// All entries (ignoring any active filter).
    pub fn all_entries(&self) -> &[DiagnosticEntry] {
        &self.entries
    }

    /// Current severity filter.  `None` means show all.
    pub fn filter(&self) -> Option<DiagnosticSeverity> {
        self.filter
    }

    /// Set the severity filter.  `None` disables filtering.
    pub fn set_filter(&mut self, filter: Option<DiagnosticSeverity>) {
        self.filter = filter;
    }

    // ── Mutation methods ───────────────────────────────────────────────

    /// Push a single diagnostic entry.
    pub fn push(&mut self, diag: Diagnostic) {
        self.entries.push(DiagnosticEntry::new(diag));
    }

    /// Push multiple diagnostic entries at once.
    pub fn push_many(&mut self, diags: Vec<Diagnostic>) {
        for diag in diags {
            self.entries.push(DiagnosticEntry::new(diag));
        }
    }

    /// Remove all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.expanded = None;
    }

    /// Replace all entries (legacy compatibility).
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.entries = diagnostics.into_iter().map(DiagnosticEntry::new).collect();
        self.expanded = None;
    }

    /// Append diagnostics (legacy compatibility).
    pub fn add_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        for diag in diagnostics {
            self.entries.push(DiagnosticEntry::new(diag));
        }
    }

    // ── UI rendering ───────────────────────────────────────────────────

    /// Render the diagnostics panel.
    ///
    /// Returns a list of commands (currently empty — the panel is read-only,
    /// but the return type matches the pattern used by other editor panels
    /// for future extensibility).
    ///
    /// Draws a severity filter bar followed by a scrollable list of
    /// diagnostics.  Each entry can be expanded via the "▼ Detail" button.
    pub fn ui(&mut self, ui: &mut EditorUi) -> Vec<Box<dyn Command>> {
        let open = ui.collapsing_header(&format!("{} [{}]", self.name, self.entries.len()), true);
        if !open {
            return Vec::new();
        }

        // ── Filter bar ──────────────────────────────────────────────
        ui.separator();
        let all_clicked = ui.button("All");
        let info_clicked = ui.button("Info");
        let warn_clicked = ui.button("Warning");
        let err_clicked = ui.button("Error");
        let fatal_clicked = ui.button("Fatal");

        if all_clicked {
            self.filter = None;
        } else if info_clicked {
            self.filter = Some(DiagnosticSeverity::Info);
        } else if warn_clicked {
            self.filter = Some(DiagnosticSeverity::Warning);
        } else if err_clicked {
            self.filter = Some(DiagnosticSeverity::Error);
        } else if fatal_clicked {
            self.filter = Some(DiagnosticSeverity::Fatal);
        }

        // Reflect current filter state
        let active_filter_name = match self.filter {
            None => "All",
            Some(DiagnosticSeverity::Info) => "Info",
            Some(DiagnosticSeverity::Warning) => "Warning",
            Some(DiagnosticSeverity::Error) => "Error",
            Some(DiagnosticSeverity::Fatal) => "Fatal",
        };
        ui.text_field("Filter", active_filter_name);

        ui.separator();

        // ── Entry list ──────────────────────────────────────────────
        let filtered_indices = self.filtered_indices();

        if filtered_indices.is_empty() {
            ui.text_field("Info", "No diagnostics to display.");
        } else {
            ui.text_field("Count", &format!("{} entries", filtered_indices.len()));

            // Collect owned entries to avoid borrow conflicts with render_entry
            let owned: Vec<DiagnosticEntry> = filtered_indices
                .iter()
                .filter_map(|&idx| self.entries.get(idx).cloned())
                .collect();

            for (i, entry) in owned.iter().enumerate() {
                let orig_idx = filtered_indices[i];
                self.render_entry(ui, entry, orig_idx);
            }
        }

        // ── Clear button ────────────────────────────────────────────
        ui.separator();
        if ui.button("Clear All") {
            self.clear();
        }

        tracing::debug!(
            panel = %self.name,
            total = self.entries.len(),
            filtered = filtered_indices.len(),
            "DiagnosticsPanel.ui"
        );

        Vec::new()
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Return indices of entries that pass the current filter.
    fn filtered_indices(&self) -> Vec<usize> {
        match self.filter {
            Some(sev) => self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.diagnostic.severity == sev)
                .map(|(i, _)| i)
                .collect(),
            None => (0..self.entries.len()).collect(),
        }
    }

    /// Render a single diagnostic entry.
    fn render_entry(&mut self, ui: &mut EditorUi, entry: &DiagnosticEntry, orig_idx: usize) {
        let diag = &entry.diagnostic;
        let icon = severity_icon(diag.severity);

        // Collapsed one-line view
        let elapsed = entry.timestamp.elapsed();
        let time_str = if elapsed.as_secs() < 120 {
            format!("{}s ago", elapsed.as_secs())
        } else {
            format!("{}m ago", elapsed.as_secs() / 60)
        };

        let one_line = format!(
            "{} [{}] {}",
            icon,
            diag.code,
            if diag.message.len() > 100 {
                format!("{}…", &diag.message[..97])
            } else {
                diag.message.clone()
            },
        );

        ui.text_field(&time_str, &one_line);

        // Expand / collapse detail
        let is_expanded = self.expanded == Some(orig_idx);
        let btn_label = if is_expanded { "▲ Hide" } else { "▼ Detail" };
        if ui.button(btn_label) {
            self.expanded = if is_expanded { None } else { Some(orig_idx) };
        }

        if is_expanded {
            ui.separator();
            // Full detail
            ui.text_field("Code", &diag.code);
            ui.text_field("Severity", &format!("{:?}", diag.severity));
            ui.text_field("System", &diag.system);
            ui.text_field("Message", &diag.message);
            if let Some(ref path) = diag.path {
                ui.text_field("Path", path);
            }
            if let Some(ref entity) = diag.entity {
                ui.text_field("Entity", entity);
            }
            if let Some(ref asset) = diag.asset {
                let asset_str = if let Some(ref logical) = asset.logical_path {
                    format!("{} ({})", asset.id, logical)
                } else {
                    asset.id.clone()
                };
                ui.text_field("Asset", &asset_str);
            }
            if let Some(ref sug) = diag.suggested_action {
                ui.text_field("Suggested", sug);
            }

            // Related diagnostics
            if !diag.related.is_empty() {
                let related_label = format!("Related ({})", diag.related.len());
                let related_open = ui.collapsing_header(&related_label, false);
                if related_open {
                    for related in &diag.related {
                        let related_entry = DiagnosticEntry::new(related.clone());
                        self.render_entry(ui, &related_entry, orig_idx);
                    }
                }
            }
            ui.separator();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return a single-character icon for a severity level.
fn severity_icon(sev: DiagnosticSeverity) -> &'static str {
    match sev {
        DiagnosticSeverity::Info => "[i]",
        DiagnosticSeverity::Warning => "[!]",
        DiagnosticSeverity::Error => "[E]",
        DiagnosticSeverity::Fatal => "[X]",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_diagnostics() -> Vec<Diagnostic> {
        vec![
            Diagnostic::new("CS1001", DiagnosticSeverity::Error, "build", "Identifier expected")
                .path("src/Program.cs"),
            Diagnostic::new(
                "RELOAD_OK",
                DiagnosticSeverity::Info,
                "script",
                "Assembly reloaded successfully",
            ),
            Diagnostic::new(
                "ASSET_MISSING",
                DiagnosticSeverity::Warning,
                "asset",
                "Texture not found",
            )
            .path("assets/textures/floor.asset"),
            Diagnostic::new(
                "SCRIPT_CRASH",
                DiagnosticSeverity::Fatal,
                "script",
                "NullReferenceException in OnUpdate",
            )
            .path("scripts/PlayerController.csx"),
        ]
    }

    #[test]
    fn diagnostics_panel_new() {
        let panel = DiagnosticsPanel::new("Diagnostics");
        assert_eq!(panel.name(), "Diagnostics");
        assert!(panel.visible());
        assert!(panel.all_entries().is_empty());
        assert!(panel.filter().is_none());
    }

    #[test]
    fn diagnostics_panel_push() {
        let mut panel = DiagnosticsPanel::new("D");
        panel.push(Diagnostic::new("E1", DiagnosticSeverity::Error, "test", "msg"));
        assert_eq!(panel.all_entries().len(), 1);
    }

    #[test]
    fn diagnostics_panel_push_many() {
        let mut panel = DiagnosticsPanel::new("D");
        panel.push_many(sample_diagnostics());
        assert_eq!(panel.all_entries().len(), 4);
    }

    #[test]
    fn diagnostics_panel_add_and_clear() {
        let mut panel = DiagnosticsPanel::new("Diag");
        panel.add_diagnostics(sample_diagnostics());
        assert_eq!(panel.all_entries().len(), 4);

        panel.clear();
        assert!(panel.all_entries().is_empty());
    }

    #[test]
    fn diagnostics_panel_set_diagnostics() {
        let mut panel = DiagnosticsPanel::new("Diag");
        let diags = sample_diagnostics();
        panel.set_diagnostics(diags.clone());
        assert_eq!(panel.all_entries().len(), 4);

        panel.set_diagnostics(vec![]);
        assert!(panel.all_entries().is_empty());
    }

    #[test]
    fn diagnostics_panel_filter_none() {
        let mut panel = DiagnosticsPanel::new("Diag");
        panel.add_diagnostics(sample_diagnostics());
        let filtered = panel.filtered_indices();
        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn diagnostics_panel_filter_error() {
        let mut panel = DiagnosticsPanel::new("Diag");
        panel.add_diagnostics(sample_diagnostics());
        panel.set_filter(Some(DiagnosticSeverity::Error));
        let filtered = panel.filtered_indices();
        assert_eq!(filtered.len(), 1);
        assert!(panel.entries[filtered[0]].diagnostic.code == "CS1001");
    }

    #[test]
    fn diagnostics_panel_filter_fatal() {
        let mut panel = DiagnosticsPanel::new("Diag");
        panel.add_diagnostics(sample_diagnostics());
        panel.set_filter(Some(DiagnosticSeverity::Fatal));
        let filtered = panel.filtered_indices();
        assert_eq!(filtered.len(), 1);
        assert_eq!(panel.entries[filtered[0]].diagnostic.code, "SCRIPT_CRASH");
    }

    #[test]
    fn diagnostics_panel_ui_empty() {
        let mut panel = DiagnosticsPanel::new("Diag");
        let mut ui = EditorUi::new();
        let cmds = panel.ui(&mut ui);
        assert!(cmds.is_empty());
    }

    #[test]
    fn diagnostics_panel_ui_with_entries() {
        let mut panel = DiagnosticsPanel::new("Diag");
        panel.add_diagnostics(sample_diagnostics());
        let mut ui = EditorUi::new();
        let cmds = panel.ui(&mut ui);
        assert!(cmds.is_empty());
    }

    #[test]
    fn diagnostics_panel_visibility() {
        let mut panel = DiagnosticsPanel::new("Diag");
        assert!(panel.visible());
        panel.set_visible(false);
        assert!(!panel.visible());
        panel.set_visible(true);
        assert!(panel.visible());
    }

    #[test]
    fn diagnostics_panel_filter_set_get() {
        let mut panel = DiagnosticsPanel::new("Diag");
        assert!(panel.filter().is_none());
        panel.set_filter(Some(DiagnosticSeverity::Warning));
        assert_eq!(panel.filter(), Some(DiagnosticSeverity::Warning));
        panel.set_filter(None);
        assert!(panel.filter().is_none());
    }

    #[test]
    fn severity_icon_values() {
        assert_eq!(severity_icon(DiagnosticSeverity::Info), "[i]");
        assert_eq!(severity_icon(DiagnosticSeverity::Warning), "[!]");
        assert_eq!(severity_icon(DiagnosticSeverity::Error), "[E]");
        assert_eq!(severity_icon(DiagnosticSeverity::Fatal), "[X]");
    }

    #[test]
    fn diagnostic_entry_new() {
        let d = Diagnostic::new("C1", DiagnosticSeverity::Info, "sys", "msg");
        let entry = DiagnosticEntry::new(d);
        assert_eq!(entry.diagnostic.code, "C1");
        assert_eq!(entry.diagnostic.severity, DiagnosticSeverity::Info);
    }
}
