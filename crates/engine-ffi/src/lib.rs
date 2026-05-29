//! # Engine FFI — Rust ↔ C# bridge layer
//!
//! This crate defines the C ABI between the Rust engine and C# scripting.
//! All public functions use `#[no_mangle] extern "C"` and all shared types
//! use `#[repr(C)]` so they can be called directly from P/Invoke or
//! ILRuntime CLR bindings without serialization.
//!
//! # Module overview
//!
//! * [`types`] — FFI-safe type definitions (`FfiEntityId`, `FfiComponentTypeId`, …)
//! * [`component`] — Component type registry + read/write FFI
//! * [`entity`] — Entity lifecycle FFI
//! * [`coroutine`] — Coroutine system FFI bridge
//! * [`r#async`] — Async I/O + callback dispatch
//! * [`engine`] — Engine service calls (spawn, play_sound, etc.)

pub mod r#async;
pub mod component;
pub mod coroutine;
pub mod engine;
pub mod entity;
pub mod types;
