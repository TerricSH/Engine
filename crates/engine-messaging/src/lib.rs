//! Engine-wide message passing system.
//!
//! Provides a typed publish/subscribe message bus for decoupled communication
//! between engine systems (physics, audio, UI, gameplay, etc.).
//!
//! # Architecture
//!
//! - [`MessageBus`] — central hub. Systems subscribe to message types and
//!   publish messages.
//! - [`Message`] trait — implemented by each message type, providing a
//!   runtime type identifier.
//! - [`MessageId`] — type-erased identifier used for dispatch.
//! - [`HandlerId`] — opaque handle for unsubscribing.

#![forbid(unsafe_code)]

use std::any::{Any, TypeId};
use std::collections::BTreeMap;

use crossbeam_channel::{Receiver, Sender, unbounded};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned by the messaging system.
#[derive(Debug, Error)]
pub enum MessageError {
    /// Attempted to unsubscribe a handler that was already removed.
    #[error("handler not found")]
    HandlerNotFound,
    /// A channel send failed (receiver was dropped).
    #[error("channel send error: {0}")]
    SendFailed(String),
}

// ---------------------------------------------------------------------------
// Message trait
// ---------------------------------------------------------------------------

/// Any type that can be sent through the [`MessageBus`].
///
/// Implementations are automatically provided for any `Send + 'static` type.
pub trait Message: Send + 'static {
    /// Human-readable name for logging / debugging.
    fn message_name() -> &'static str;
}

/// Blanket implementation for any `Send + 'static`.
impl<T: Send + 'static> Message for T {
    fn message_name() -> &'static str {
        std::any::type_name::<T>()
    }
}

// ---------------------------------------------------------------------------
// HandlerId
// ---------------------------------------------------------------------------

/// Opaque handle returned when subscribing.  Use to unsubscribe later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandlerId(u64);

// ---------------------------------------------------------------------------
// Internal type-erased registration
// ---------------------------------------------------------------------------

type BoxedHandler = Box<dyn Fn(&dyn Any) + Send>;

struct HandlerEntry {
    id: HandlerId,
    handler: BoxedHandler,
}

// ---------------------------------------------------------------------------
// MessageBus
// ---------------------------------------------------------------------------

/// Central message bus for the engine.
///
/// # Example
///
/// ```ignore
/// use engine_messaging::MessageBus;
///
/// struct PlayerSpawned { id: u64 }
///
/// let mut bus = MessageBus::new();
/// let h = bus.subscribe::<PlayerSpawned>(|msg| {
///     tracing::info!("player {} spawned", msg.id);
/// });
///
/// bus.publish(PlayerSpawned { id: 42 });
/// bus.unsubscribe(h);
/// ```
pub struct MessageBus {
    handlers: BTreeMap<std::any::TypeId, Vec<HandlerEntry>>,
    next_id: u64,
}

impl MessageBus {
    /// Create a new empty message bus.
    pub fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Subscribe to a message type.
    ///
    /// `callback` is invoked on [`publish`](Self::publish) for every message
    /// of type `M` that has been sent since the last
    /// [`drain`](Self::drain) call.
    pub fn subscribe<M: Message>(&mut self, callback: impl Fn(&M) + Send + 'static) -> HandlerId {
        let id = HandlerId(self.next_id);
        self.next_id += 1;

        let entry = HandlerEntry {
            id,
            handler: Box::new(move |any| {
                if let Some(msg) = any.downcast_ref::<M>() {
                    callback(msg);
                }
            }),
        };

        self.handlers
            .entry(TypeId::of::<M>())
            .or_default()
            .push(entry);

        id
    }

    /// Remove a previously registered handler.
    pub fn unsubscribe(&mut self, handler_id: HandlerId) -> Result<(), MessageError> {
        for (_, entries) in &mut self.handlers {
            if let Some(pos) = entries.iter().position(|e| e.id == handler_id) {
                entries.swap_remove(pos);
                return Ok(());
            }
        }
        Err(MessageError::HandlerNotFound)
    }

    /// Publish a message to all subscribed handlers immediately.
    pub fn publish<M: Message>(&mut self, message: M) {
        let type_id = TypeId::of::<M>();
        if let Some(entries) = self.handlers.get(&type_id) {
            for entry in entries {
                (entry.handler)(&message as &dyn Any);
            }
        }
    }

    /// Remove all handlers for a message type.
    pub fn clear_handlers<M: Message>(&mut self) {
        self.handlers.remove(&TypeId::of::<M>());
    }

    /// Remove all handlers for all message types.
    pub fn clear_all(&mut self) {
        self.handlers.clear();
    }

    /// Number of registered handler entries (across all types).
    pub fn handler_count(&self) -> usize {
        self.handlers.values().map(|v| v.len()).sum()
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Channel-based MessageBus (async / multi-threaded variant)
// ---------------------------------------------------------------------------

/// A channel-backed message bus for cross-thread communication.
///
/// Unlike [`MessageBus`], which dispatches synchronously on `publish`,
/// `ChannelBus` buffers messages and the receiver drains them at its own
/// pace via [`try_recv`](Self::try_recv) / [`recv`](Self::recv).
pub struct ChannelBus<M: Message> {
    tx: Sender<M>,
    rx: Receiver<M>,
}

impl<M: Message> ChannelBus<M> {
    /// Create a new channel bus.
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self { tx, rx }
    }

    /// Send a message (non-blocking).
    pub fn send(&self, msg: M) -> Result<(), MessageError> {
        self.tx.send(msg).map_err(|e| MessageError::SendFailed(e.to_string()))
    }

    /// Try to receive a message (non-blocking).
    pub fn try_recv(&self) -> Option<M> {
        self.rx.try_recv().ok()
    }

    /// Block until a message arrives.
    pub fn recv(&self) -> Result<M, MessageError> {
        self.rx.recv().map_err(|e| MessageError::SendFailed(e.to_string()))
    }

    /// Drain all pending messages.
    pub fn drain(&self) -> Vec<M> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    /// Get a sender (can be cloned for use in other threads).
    pub fn sender(&self) -> Sender<M> {
        self.tx.clone()
    }
}

impl<M: Message> Default for ChannelBus<M> {
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

    struct TestMsg { value: i32 }
    struct OtherMsg { label: String }

    #[test]
    fn subscribe_and_publish() {
        let mut bus = MessageBus::new();
        let received = std::cell::Cell::new(0);
        let _h = bus.subscribe::<TestMsg>(move |msg| {
            received.set(msg.value);
        });
        bus.publish(TestMsg { value: 42 });
        assert_eq!(received.get(), 42);
    }

    #[test]
    fn unsubscribe() {
        let mut bus = MessageBus::new();
        let count = std::cell::Cell::new(0);
        let h = bus.subscribe::<TestMsg>(move |_| {
            count.set(count.get() + 1);
        });
        bus.publish(TestMsg { value: 1 });
        assert_eq!(count.get(), 1);
        bus.unsubscribe(h).unwrap();
        bus.publish(TestMsg { value: 2 });
        assert_eq!(count.get(), 1, "should not receive after unsubscribe");
    }

    #[test]
    fn different_types_dont_interfere() {
        let mut bus = MessageBus::new();
        let test_vals = std::cell::Cell::new(0);
        let other_vals = std::cell::Cell::new(String::new());

        let _h1 = bus.subscribe::<TestMsg>(|msg| { test_vals.set(msg.value); });
        let _h2 = bus.subscribe::<OtherMsg>(|msg| { other_vals.set(msg.label.clone()); });

        bus.publish(OtherMsg { label: "hello".into() });
        assert_eq!(test_vals.get(), 0);
        assert_eq!(other_vals.into_inner(), "hello");
    }

    #[test]
    fn clear_handlers() {
        let mut bus = MessageBus::new();
        let count = std::cell::Cell::new(0);
        let _h = bus.subscribe::<TestMsg>(|_| { count.set(count.get() + 1); });
        bus.clear_handlers::<TestMsg>();
        bus.publish(TestMsg { value: 0 });
        assert_eq!(count.get(), 0);
    }

    #[test]
    fn handler_id_unique() {
        let mut bus = MessageBus::new();
        let h1 = bus.subscribe::<TestMsg>(|_| {});
        let h2 = bus.subscribe::<TestMsg>(|_| {});
        assert_ne!(h1, h2);
    }

    #[test]
    fn channel_bus_send_recv() {
        let bus = ChannelBus::<TestMsg>::new();
        bus.send(TestMsg { value: 7 }).unwrap();
        let msg = bus.try_recv().unwrap();
        assert_eq!(msg.value, 7);
    }

    #[test]
    fn channel_bus_drain() {
        let bus = ChannelBus::<TestMsg>::new();
        bus.send(TestMsg { value: 1 }).unwrap();
        bus.send(TestMsg { value: 2 }).unwrap();
        let msgs = bus.drain();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].value, 1);
        assert_eq!(msgs[1].value, 2);
    }
}
