use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// A single telemetry data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub timestamp: f64,
    pub category: String,
    pub name: String,
    pub value: f64,
    pub metadata: String,
}

/// Collects and manages telemetry events with a fixed-capacity ring buffer.
pub struct TelemetryCollector {
    events: Vec<TelemetryEvent>,
    max_events: usize,
    session_id: String,
}

impl TelemetryCollector {
    /// Create a new collector with the given capacity and session identifier.
    pub fn new(max_events: usize, session_id: impl Into<String>) -> Self {
        Self {
            events: Vec::with_capacity(max_events),
            max_events,
            session_id: session_id.into(),
        }
    }

    /// Returns a reference to all currently stored events.
    pub fn events(&self) -> &[TelemetryEvent] {
        &self.events
    }

    /// Returns the session identifier.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the maximum number of events this collector can hold.
    pub fn max_events(&self) -> usize {
        self.max_events
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Returns a Unix timestamp (seconds since epoch) as `f64`.
fn now_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Record a new telemetry event, trimming oldest entries if the capacity is
/// exceeded.
pub fn record(
    collector: &mut TelemetryCollector,
    category: &str,
    name: &str,
    value: f64,
    metadata: &str,
) {
    let event = TelemetryEvent {
        timestamp: now_timestamp(),
        category: category.to_string(),
        name: name.to_string(),
        value,
        metadata: metadata.to_string(),
    };
    collector.events.push(event);

    // Trim oldest entries when the buffer overflows.
    if collector.events.len() > collector.max_events {
        let excess = collector.events.len() - collector.max_events;
        collector.events.drain(0..excess);
    }
}

/// Export all recorded events as a JSON array string.
pub fn export_json(collector: &TelemetryCollector) -> String {
    serde_json::to_string(&collector.events).unwrap_or_else(|_| "[]".into())
}

/// Clear all recorded events from the collector.
pub fn clear(collector: &mut TelemetryCollector) {
    collector.events.clear();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_adds_event() {
        let mut col = TelemetryCollector::new(100, "test-session");
        assert_eq!(col.events().len(), 0);

        record(&mut col, "input", "click", 1.0, "button=fire");
        assert_eq!(col.events().len(), 1);

        let ev = &col.events()[0];
        assert_eq!(ev.category, "input");
        assert_eq!(ev.name, "click");
        assert_eq!(ev.value, 1.0);
        assert_eq!(ev.metadata, "button=fire");
        assert!(ev.timestamp > 0.0);
    }

    #[test]
    fn export_produces_valid_json() {
        let mut col = TelemetryCollector::new(100, "json-test");
        record(&mut col, "test", "event_a", 42.0, "a=1");
        record(&mut col, "test", "event_b", 99.0, "b=2");

        let json = export_json(&col);
        let parsed: Vec<TelemetryEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "event_a");
        assert_eq!(parsed[1].name, "event_b");
    }

    #[test]
    fn max_events_trim_works() {
        let mut col = TelemetryCollector::new(3, "trim-test");

        record(&mut col, "cat", "e1", 1.0, "");
        record(&mut col, "cat", "e2", 2.0, "");
        record(&mut col, "cat", "e3", 3.0, "");
        assert_eq!(col.events().len(), 3);

        // This should push e1 out.
        record(&mut col, "cat", "e4", 4.0, "");
        assert_eq!(col.events().len(), 3);
        assert_eq!(col.events()[0].name, "e2");
        assert_eq!(col.events()[1].name, "e3");
        assert_eq!(col.events()[2].name, "e4");
    }

    #[test]
    fn clear_empties_events() {
        let mut col = TelemetryCollector::new(100, "clear-test");
        record(&mut col, "cat", "e1", 1.0, "");
        record(&mut col, "cat", "e2", 2.0, "");
        assert!(!col.events().is_empty());

        clear(&mut col);
        assert!(col.events().is_empty());
    }

    #[test]
    fn empty_export_is_valid_json() {
        let col = TelemetryCollector::new(10, "empty");
        let json = export_json(&col);
        assert_eq!(json, "[]");
    }

    #[test]
    fn session_id_is_stored() {
        let col = TelemetryCollector::new(10, "my-session-42");
        assert_eq!(col.session_id(), "my-session-42");
    }
}
