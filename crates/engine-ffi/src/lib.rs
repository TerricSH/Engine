//! # Engine FFI — Rust ↔ C# bridge layer
//!
//! This crate defines the C ABI between the Rust engine and C# scripting.
//! All public functions use `#[no_mangle] extern "C"` and all shared types
//! use `#[repr(C)]` so they can be called directly from P/Invoke or
//! ILRuntime CLR bindings without serialization.
//!
//! # Safety policy
//!
//! This crate is **excepted** from `#![forbid(unsafe_code)]` because FFI
//! bridging inherently requires `unsafe` for:
//!
//! * Dereferencing raw pointers passed from C# (`CStr::from_ptr`, …)
//! * `static mut` globals for the callback registry (singleton pattern)
//! * `unsafe extern "C"` functions that accept raw pointer parameters
//!
//! Every `unsafe` block must have a `// SAFETY:` comment explaining its
//! invariants.  The callback registry ([`registry`]) provides a safe
//! dispatch layer that most FFI functions should use instead of directly
//! calling through raw pointers.
//!
//! # Module overview
//!
//! * [`types`] — FFI-safe type definitions (`FfiEntityId`, `FfiComponentTypeId`, …)
//! * [`component`] — Component type registry + read/write FFI
//! * [`entity`] — Entity lifecycle FFI
//! * [`coroutine`] — Coroutine system FFI bridge
//! * [`r#async`] — Async I/O + callback dispatch
//! * [`engine`] — Engine service calls (spawn, play_sound, etc.)
//! * [`registry`] — Runtime callback table populated by the engine
//!
//! This crate is excepted from `#![forbid(unsafe_code)]` because it provides the FFI bridge between Rust and C#.

pub mod animation;
pub mod r#async;
pub mod character;
pub mod component;
pub mod coroutine;
pub mod engine;
pub mod entity;
pub mod ik;
pub mod registry;
pub mod types;
pub mod world_bridge;
