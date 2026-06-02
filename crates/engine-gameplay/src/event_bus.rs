//! # Gameplay Event Bus (G18-F04)
//!
//! Lightweight pub/sub event system for gameplay events.
//!
//! # Architecture
//!
//! * **`GameplayEvent`** — a tagged union of all built-in gameplay event types
//!   (score, lives, health, ammo, state changes, dialogue, objectives, quests,
//!   and custom string-keyed events).
//! * **`EventBus`** — a subscription manager.  Callers subscribe with a
//!   string-typed event type and a boxed closure, receiving a `SubscriptionId`
//!   that can be used to unsubscribe.
//! * **`EventHistory`** — a ring buffer that records the last N events.
//!   New subscribers can call `replay` to catch up on history.
//!
//! # Example
//!
//! ```
//! use engine_gameplay::event_bus::{EventBus, GameplayEvent};
//!
//! let mut bus = EventBus::new(64);
//!
//! let id = bus.subscribe("ScoreChanged", |ev: &GameplayEvent| {
//!     if let GameplayEvent::ScoreChanged(pts) = ev {
//!         println!("Score changed by {}", pts);
//!     }
//! });
//!
//! bus.publish(GameplayEvent::ScoreChanged(100));
//! bus.publish(GameplayEvent::ScoreChanged(50));
//!
//! # // Unused — kept for drop ordering
//! # let _ = id;
//! ```

use crate::state::GameState;

// ---------------------------------------------------------------------------
// SubscriptionId
// ---------------------------------------------------------------------------

/// Opaque identifier for a subscription, returned by [`EventBus::subscribe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub(crate) u64);

impl SubscriptionId {
    /// The raw u64 identifier.
    pub fn to_u64(self) -> u64 {
        self.0
    }

    fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------
// GameplayEvent
// ---------------------------------------------------------------------------

/// Events that can flow through the gameplay event bus.
#[derive(Clone, Debug, PartialEq)]
pub enum GameplayEvent {
    /// The player's score changed by the given delta (positive or negative).
    ScoreChanged(i32),
    /// The player's remaining lives changed.
    LivesChanged(i32),
    /// The player's health changed.
    HealthChanged(f32),
    /// Ammo count changed.
    AmmoChanged(u32),
    /// The game state machine transitioned.
    GameStateChanged(GameState),
    /// A dialogue line was triggered (dialogue ID).
    DialogueTriggered(String),
    /// An objective was updated (objective ID).
    ObjectiveUpdated(String),
    /// A quest was completed (quest ID).
    QuestCompleted(String),
    /// A custom event with a string key and string payload.
    Custom(String, String),
}

impl GameplayEvent {
    /// Return the string event-type key for this event.
    ///
    /// This is the key used for subscription matching in [`EventBus`].
    pub fn event_type(&self) -> &str {
        match self {
            Self::ScoreChanged(_) => "ScoreChanged",
            Self::LivesChanged(_) => "LivesChanged",
            Self::HealthChanged(_) => "HealthChanged",
            Self::AmmoChanged(_) => "AmmoChanged",
            Self::GameStateChanged(_) => "GameStateChanged",
            Self::DialogueTriggered(_) => "DialogueTriggered",
            Self::ObjectiveUpdated(_) => "ObjectiveUpdated",
            Self::QuestCompleted(_) => "QuestCompleted",
            Self::Custom(key, _) => key.as_str(), // dynamic key
        }
    }
}

// ---------------------------------------------------------------------------
// EventHistory
// ---------------------------------------------------------------------------

/// A ring buffer that records the last N events for replay.
pub struct EventHistory {
    buffer: Vec<GameplayEvent>,
    capacity: usize,
    write_index: usize,
    count: usize,
}

impl EventHistory {
    /// Create a new history buffer with the given capacity.
    ///
    /// `capacity` is the maximum number of events retained.  When the
    /// buffer is full, the oldest event is overwritten.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
            capacity,
            write_index: 0,
            count: 0,
        }
    }

    /// Record an event into the ring buffer.
    pub fn record(&mut self, event: &GameplayEvent) {
        if self.capacity == 0 {
            return;
        }
        if self.buffer.len() < self.capacity {
            self.buffer.push(event.clone());
        } else {
            self.buffer[self.write_index] = event.clone();
        }
        self.write_index = (self.write_index + 1) % self.capacity;
        self.count = self.count.saturating_add(1);
    }

    /// Replay all recorded events to the given callback, in order.
    pub fn replay(&self, target: &mut dyn FnMut(&GameplayEvent)) {
        if self.count == 0 || self.buffer.is_empty() {
            return;
        }

        let cap = self.buffer.len();
        let start = if self.count > cap {
            self.write_index // oldest is at write_index when full
        } else {
            0
        };

        for i in 0..cap {
            let idx = (start + i) % cap;
            // Only emit events that have been written (when ring isn't full yet,
            // buffer may have uninitialised slots — but we only push, so all
            // slots are valid).
            target(&self.buffer[idx]);
        }
    }

    /// The number of events recorded so far.
    pub fn count(&self) -> usize {
        self.count
    }

    /// The maximum capacity of the history buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear all recorded events.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.write_index = 0;
        self.count = 0;
    }
}

// ---------------------------------------------------------------------------
// EventSubscription type alias
// ---------------------------------------------------------------------------

type EventSubscription = (
    String,
    SubscriptionId,
    Box<dyn FnMut(&GameplayEvent) + Send>,
);

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

/// A pub/sub event bus for gameplay events.
///
/// Subscribers register with a string event-type key and receive a
/// [`SubscriptionId`] that can be used to unsubscribe.
pub struct EventBus {
    subscriptions: Vec<EventSubscription>,
    history: EventHistory,
}

impl EventBus {
    /// Create a new event bus with the given history capacity.
    ///
    /// The history records the last `history_capacity` events for replay
    /// to new subscribers.  Pass `0` to disable history.
    pub fn new(history_capacity: usize) -> Self {
        Self {
            subscriptions: Vec::new(),
            history: EventHistory::new(history_capacity),
        }
    }

    /// Subscribe to an event type.
    ///
    /// `event_type` is matched against [`GameplayEvent::event_type()`].
    /// For custom events this is the string key used in the `Custom` variant.
    ///
    /// Returns a [`SubscriptionId`] that can be used to unsubscribe.
    pub fn subscribe<F>(&mut self, event_type: &str, callback: F) -> SubscriptionId
    where
        F: FnMut(&GameplayEvent) + Send + 'static,
    {
        let id = SubscriptionId::next();
        self.subscriptions
            .push((event_type.to_string(), id, Box::new(callback)));
        id
    }

    /// Unsubscribe a previously registered subscription.
    ///
    /// Returns `true` if the subscription was found and removed.
    pub fn unsubscribe(&mut self, id: SubscriptionId) -> bool {
        let len_before = self.subscriptions.len();
        self.subscriptions.retain(|(_, sid, _)| *sid != id);
        self.subscriptions.len() < len_before
    }

    /// Publish an event to all matching subscribers.
    ///
    /// The event is also recorded in the history buffer.
    pub fn publish(&mut self, event: GameplayEvent) {
        let event_type = event.event_type().to_string();

        // Record to history first.
        self.history.record(&event);

        // Fire matching subscriptions.
        for (et, _, cb) in &mut self.subscriptions {
            if *et == event_type || *et == "*" {
                cb(&event);
            }
        }
    }

    /// Subscribe to an event type AND immediately receive all past events
    /// from the history via the callback.
    ///
    /// Returns the [`SubscriptionId`].
    pub fn subscribe_with_replay<F>(&mut self, event_type: &str, mut callback: F) -> SubscriptionId
    where
        F: FnMut(&GameplayEvent) + Send + 'static,
    {
        // Replay history for this subscriber.
        self.history.replay(&mut |ev| {
            if ev.event_type() == event_type || event_type == "*" {
                callback(ev);
            }
        });

        // Then subscribe for future events.
        self.subscribe(event_type, callback)
    }

    /// Access the event history.
    pub fn history(&self) -> &EventHistory {
        &self.history
    }

    /// Access the event history mutably (for clear, etc.).
    pub fn history_mut(&mut self) -> &mut EventHistory {
        &mut self.history
    }

    /// The number of active subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // Helper: quick check helpers. Used via Arc<Mutex<...>> to satisfy Send.

    // -- Publish / Subscribe ------------------------------------------------

    #[test]
    fn publish_triggers_subscriber() {
        let mut bus = EventBus::new(64);
        let last_value = Arc::new(Mutex::new(None));
        let lv = Arc::clone(&last_value);

        bus.subscribe("ScoreChanged", move |ev: &GameplayEvent| {
            if let GameplayEvent::ScoreChanged(v) = ev {
                *lv.lock().unwrap() = Some(*v);
            }
        });

        bus.publish(GameplayEvent::ScoreChanged(42));
        assert_eq!(*last_value.lock().unwrap(), Some(42));
    }

    #[test]
    fn publish_does_not_trigger_unrelated() {
        let mut bus = EventBus::new(64);
        let fired = Arc::new(Mutex::new(false));
        let f = Arc::clone(&fired);

        bus.subscribe("LivesChanged", move |_: &GameplayEvent| {
            *f.lock().unwrap() = true;
        });

        bus.publish(GameplayEvent::ScoreChanged(100));
        assert!(!*fired.lock().unwrap());
    }

    #[test]
    fn multiple_subscribers_all_fire() {
        let mut bus = EventBus::new(64);
        let count = Arc::new(Mutex::new(0u32));

        let c1 = Arc::clone(&count);
        bus.subscribe("ScoreChanged", move |_: &GameplayEvent| {
            *c1.lock().unwrap() += 1;
        });

        let c2 = Arc::clone(&count);
        bus.subscribe("ScoreChanged", move |_: &GameplayEvent| {
            *c2.lock().unwrap() += 1;
        });

        bus.publish(GameplayEvent::ScoreChanged(10));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn wildcard_subscriber_receives_all() {
        let mut bus = EventBus::new(64);
        let events = Arc::new(Mutex::new(Vec::new()));
        let evt = Arc::clone(&events);

        bus.subscribe("*", move |ev: &GameplayEvent| {
            evt.lock().unwrap().push(ev.event_type().to_string());
        });

        bus.publish(GameplayEvent::ScoreChanged(1));
        bus.publish(GameplayEvent::LivesChanged(2));
        bus.publish(GameplayEvent::HealthChanged(3.0));

        assert_eq!(events.lock().unwrap().len(), 3);
    }

    // -- Unsubscribe --------------------------------------------------------

    #[test]
    fn unsubscribe_removes_subscriber() {
        let mut bus = EventBus::new(64);
        let count = Arc::new(Mutex::new(0u32));
        let c = Arc::clone(&count);

        let id = bus.subscribe("ScoreChanged", move |_: &GameplayEvent| {
            *c.lock().unwrap() += 1;
        });

        bus.publish(GameplayEvent::ScoreChanged(10));
        assert_eq!(*count.lock().unwrap(), 1);

        assert!(bus.unsubscribe(id));
        bus.publish(GameplayEvent::ScoreChanged(20));
        // Should not fire again.
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn unsubscribe_invalid_id_returns_false() {
        let mut bus = EventBus::new(64);
        assert!(!bus.unsubscribe(SubscriptionId(999)));
    }

    // -- History ------------------------------------------------------------

    #[test]
    fn history_records_events() {
        let mut bus = EventBus::new(64);
        bus.publish(GameplayEvent::ScoreChanged(1));
        bus.publish(GameplayEvent::ScoreChanged(2));
        assert_eq!(bus.history().count(), 2);
    }

    #[test]
    fn history_replay_delivers_all_events() {
        let mut bus = EventBus::new(64);
        bus.publish(GameplayEvent::ScoreChanged(10));
        bus.publish(GameplayEvent::LivesChanged(3));

        let replayed = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&replayed);
        bus.history_mut()
            .replay(&mut |ev| r.lock().unwrap().push(ev.clone()));

        assert_eq!(replayed.lock().unwrap().len(), 2);
        assert_eq!(replayed.lock().unwrap()[0], GameplayEvent::ScoreChanged(10));
        assert_eq!(replayed.lock().unwrap()[1], GameplayEvent::LivesChanged(3));
    }

    #[test]
    fn history_capacity_limit() {
        let mut bus = EventBus::new(2);
        bus.publish(GameplayEvent::ScoreChanged(1));
        bus.publish(GameplayEvent::ScoreChanged(2));
        bus.publish(GameplayEvent::ScoreChanged(3)); // overwrites index 0

        let replayed = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&replayed);
        bus.history_mut()
            .replay(&mut |ev| r.lock().unwrap().push(ev.clone()));

        // Only 2 events retained (capacity = 2).
        assert_eq!(replayed.lock().unwrap().len(), 2);
    }

    #[test]
    fn subscribe_with_replay() {
        let mut bus = EventBus::new(64);
        bus.publish(GameplayEvent::ScoreChanged(100));
        bus.publish(GameplayEvent::LivesChanged(5));

        let replayed = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&replayed);
        bus.subscribe_with_replay("ScoreChanged", move |ev: &GameplayEvent| {
            if let GameplayEvent::ScoreChanged(v) = ev {
                r.lock().unwrap().push(*v);
            }
        });

        // Should have received the historical ScoreChanged(100).
        assert_eq!(replayed.lock().unwrap().len(), 1);
        assert_eq!(replayed.lock().unwrap()[0], 100);

        // Future events also fire.
        bus.publish(GameplayEvent::ScoreChanged(200));
        assert_eq!(replayed.lock().unwrap().len(), 2);
        assert_eq!(replayed.lock().unwrap()[1], 200);
    }

    #[test]
    fn history_clear() {
        let mut bus = EventBus::new(64);
        bus.publish(GameplayEvent::ScoreChanged(10));
        assert_eq!(bus.history().count(), 1);
        bus.history_mut().clear();
        assert_eq!(bus.history().count(), 0);
    }

    #[test]
    fn history_zero_capacity() {
        let mut bus = EventBus::new(0);
        bus.publish(GameplayEvent::ScoreChanged(10));
        assert_eq!(bus.history().count(), 0);
    }

    // -- Custom events ------------------------------------------------------

    #[test]
    fn custom_event_routing() {
        let mut bus = EventBus::new(64);
        let fired = Arc::new(Mutex::new(false));
        let f = Arc::clone(&fired);

        bus.subscribe("MyCustomEvent", move |ev: &GameplayEvent| {
            if let GameplayEvent::Custom(key, val) = ev {
                assert_eq!(key, "MyCustomEvent");
                assert_eq!(val, "hello");
                *f.lock().unwrap() = true;
            }
        });

        bus.publish(GameplayEvent::Custom(
            "MyCustomEvent".to_string(),
            "hello".to_string(),
        ));
        assert!(*fired.lock().unwrap());
    }

    // -- Subscription count -------------------------------------------------

    #[test]
    fn subscription_count() {
        let mut bus = EventBus::new(64);
        assert_eq!(bus.subscription_count(), 0);

        let id1 = bus.subscribe("a", |_: &GameplayEvent| {});
        let id2 = bus.subscribe("b", |_: &GameplayEvent| {});
        assert_eq!(bus.subscription_count(), 2);

        bus.unsubscribe(id1);
        assert_eq!(bus.subscription_count(), 1);

        bus.unsubscribe(id2);
        assert_eq!(bus.subscription_count(), 0);
    }
}
