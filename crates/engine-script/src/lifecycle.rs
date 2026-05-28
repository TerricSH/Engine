//! Standard lifecycle callback names for script components.
//!
//! These constants define the canonical method names that the script host
//! invokes on script instances. All hosts **should** use these names so that
//! the engine can dispatch lifecycle events uniformly regardless of the
//! scripting language or runtime.

/// Names of the standard lifecycle methods.
pub mod lifecycle {
    /// Called when the script instance is first created (after fields are set).
    pub const ON_CREATE: &str = "OnCreate";

    /// Called just before the first update tick.
    pub const ON_START: &str = "OnStart";

    /// Called every frame with the delta time (in seconds).
    pub const ON_UPDATE: &str = "OnUpdate";

    /// Called when the script instance is being destroyed.
    pub const ON_DESTROY: &str = "OnDestroy";
}
