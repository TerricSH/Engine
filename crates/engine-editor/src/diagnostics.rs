//! Diagnostics data model shared by Console and specialized editor views.

use std::time::Instant;

use engine_serialize::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Debug)]
pub struct DiagnosticEntry {
    pub diagnostic: Diagnostic,
    pub timestamp: Instant,
}

impl DiagnosticEntry {
    pub fn new(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostic,
            timestamp: Instant::now(),
        }
    }
}

pub struct DiagnosticsPanel {
    entries: Vec<DiagnosticEntry>,
    filter: Option<DiagnosticSeverity>,
}

impl DiagnosticsPanel {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            filter: None,
        }
    }

    pub fn all_entries(&self) -> &[DiagnosticEntry] {
        &self.entries
    }

    pub fn visible_entries(&self) -> impl Iterator<Item = &DiagnosticEntry> {
        self.entries.iter().filter(|entry| {
            self.filter
                .is_none_or(|severity| entry.diagnostic.severity == severity)
        })
    }

    pub fn filter(&self) -> Option<DiagnosticSeverity> {
        self.filter
    }

    pub fn set_filter(&mut self, filter: Option<DiagnosticSeverity>) {
        self.filter = filter;
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.entries.push(DiagnosticEntry::new(diagnostic));
    }

    pub fn push_many(&mut self, diagnostics: Vec<Diagnostic>) {
        self.entries
            .extend(diagnostics.into_iter().map(DiagnosticEntry::new));
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for DiagnosticsPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_applied_without_discarding_entries() {
        let mut panel = DiagnosticsPanel::default();
        panel.push(Diagnostic::new(
            "INFO",
            DiagnosticSeverity::Info,
            "test",
            "ready",
        ));
        panel.push(Diagnostic::new(
            "ERROR",
            DiagnosticSeverity::Error,
            "test",
            "failed",
        ));
        panel.set_filter(Some(DiagnosticSeverity::Error));
        assert_eq!(panel.visible_entries().count(), 1);
        assert_eq!(panel.all_entries().len(), 2);
    }

    #[test]
    fn clear_removes_every_entry() {
        let mut panel = DiagnosticsPanel::default();
        panel.push(Diagnostic::new(
            "INFO",
            DiagnosticSeverity::Info,
            "test",
            "ready",
        ));
        panel.clear();
        assert!(panel.all_entries().is_empty());
    }
}
