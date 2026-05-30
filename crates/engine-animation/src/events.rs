use serde::{Deserialize, Serialize};

/// An animation event marker on a clip timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimEventDef {
    /// Normalized time (0..1) within the clip.
    pub time_normalized: f32,
    /// Event name/identifier.
    pub name: String,
    /// Optional string payload.
    pub payload: Option<String>,
}

/// Fired animation event at runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimEvent {
    pub name: String,
    pub clip_asset: String,
    pub payload: Option<String>,
    /// Current clip time when fired.
    pub clip_time: f32,
}

/// Collects fired animation events during a frame.
#[derive(Clone, Debug, Default)]
pub struct AnimEventCollector {
    pub events: Vec<AnimEvent>,
}

impl AnimEventCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Record a fired event.
    pub fn push(&mut self, event: AnimEvent) {
        self.events.push(event);
    }

    /// Drain all recorded events.
    pub fn drain(&mut self) -> Vec<AnimEvent> {
        std::mem::take(&mut self.events)
    }
}

/// Check if an event should fire given the previous and current clip times.
/// Fires when the playhead crosses the event time (forward playback).
pub fn check_event_trigger(
    event_time: f32,
    prev_time: f32,
    current_time: f32,
    duration: f32,
) -> bool {
    if duration <= 0.0 {
        return false;
    }
    let normalized = event_time * duration;

    if current_time >= prev_time {
        // Forward playback
        prev_time <= normalized && normalized < current_time
    } else {
        // Looped around — fire if event is in [prev, duration) or [0, current)
        (prev_time <= normalized && normalized <= duration)
            || (0.0 <= normalized && normalized < current_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_fires_at_exact_time() {
        // Event at normalized 0.25 (=0.5s absolute), prev=0.4s, current=0.6s
        assert!(check_event_trigger(0.25, 0.4, 0.6, 2.0));
    }

    #[test]
    fn event_does_not_fire_before() {
        // Event at normalized 0.25 (=0.5s absolute), prev=0.0s, current=0.4s — not yet reached
        assert!(!check_event_trigger(0.25, 0.0, 0.4, 2.0));
    }

    #[test]
    fn event_fires_on_loop_wrap() {
        // Event at normalized 0.9 (=1.8s absolute), clip loops from 1.7 to 0.1
        // 1.7s ← 1.8s ← 2.0s (wrap) → 0.0s → 0.1s — fires in [prev, duration)
        assert!(check_event_trigger(0.9, 1.7, 0.1, 2.0));
    }

    #[test]
    fn no_event_when_duration_zero() {
        assert!(!check_event_trigger(0.5, 0.0, 1.0, 0.0));
    }

    #[test]
    fn event_collector_push_drain() {
        let mut col = AnimEventCollector::new();
        col.push(AnimEvent {
            name: "test".into(),
            clip_asset: "clip".into(),
            payload: None,
            clip_time: 1.0,
        });
        assert!(!col.is_empty());
        assert_eq!(col.drain().len(), 1);
        assert!(col.is_empty());
    }

    #[test]
    fn event_def_serde_roundtrip() {
        let def = AnimEventDef {
            time_normalized: 0.5,
            name: "footstep".into(),
            payload: Some("left".into()),
        };
        let bytes = bincode::serialize(&def).unwrap();
        let back: AnimEventDef = bincode::deserialize(&bytes).unwrap();
        assert_eq!(def, back);
    }
}
