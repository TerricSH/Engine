//! # Engine Gameplay — Game state management and gameplay event system
//!
//! This crate provides foundational gameplay systems:
//!
//! * **Game State Manager** — Finite state machine for the game lifecycle
//!   (`Boot → Menu → Loading → Playing → Paused → GameOver`) with
//!   transition validation rules and C#-compatible callbacks.
//! * **Gameplay Event Bus** — Lightweight pub/sub event system for
//!   gameplay events such as score, health, ammo, dialogue, quests,
//!   and custom string-keyed events.
//!
//! Both systems are designed to be used from Rust gameplay code and
//! from the C# scripting layer via `engine-ffi`.

pub mod event_bus;
pub mod input;
pub mod platform;
pub mod state;
pub mod telemetry;

pub use event_bus::{EventBus, EventHistory, GameplayEvent, SubscriptionId};
pub use input::*;
pub use platform::{DesktopPlatform, MockPlatform, PlatformCapabilities, PlatformFacade};
pub use state::{GameState, GameStateManager, StateTransitionRule};
pub use telemetry::{clear, export_json, record, TelemetryCollector, TelemetryEvent};
