//! Script command adapters owned by the runtime composition layer.
//!
//! `engine-script` defines the transport-safe command contract while the
//! subsystem crates own their runtime components.  The adapters in this
//! module are the only place that translates between those two layers.

pub(crate) mod animation;
#[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
pub(crate) mod ui;
