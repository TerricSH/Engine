//! XR runtime abstraction shared by render backends and gameplay input.
//!
//! The default build contains portable stereo/tracking/action contracts.
//! `openxr-runtime` adds dynamic OpenXR loader discovery without imposing an
//! XR loader dependency on ordinary desktop games.

#![deny(unsafe_op_in_unsafe_fn)]

mod action;
mod frame;
#[cfg(feature = "openxr-runtime")]
mod openxr_runtime;
mod runtime;

pub use action::*;
pub use frame::*;
#[cfg(feature = "openxr-runtime")]
pub use openxr_runtime::*;
pub use runtime::*;
