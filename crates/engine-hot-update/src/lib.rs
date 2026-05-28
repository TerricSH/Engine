#![forbid(unsafe_code)]

//! # engine-hot-update
//!
//! Gate 8 — Hot Update Package lifecycle management.
//!
//! Provides the [`PackageManager`] orchestrator that coordinates
//! download, verification, staging, atomic activation, rollback, and
//! runtime apply hooks for `MobileHotUpdate-v0` packages.
//!
//! ## Architecture
//!
//! ```text
//! PackageManager
//!   ├─ Verifier        — Signature, hash, compatibility, platform rules
//!   ├─ Downloader      — HTTP or local download of payload files
//!   ├─ PackageCache    — On-disk versioned package cache
//!   ├─ Installer       — Stage & atomic activation
//!   ├─ RollbackManager — Rollback to previous known-good
//!   └─ UpdateApplier   — Resource reload, logic asset copy, assembly apply
//! ```
//!
//! ## State Machine
//!
//! ```text
//! Discovered → Downloading → Downloaded → Verified → Staged → Active
//!                                                              ↓
//!                                                         RolledBack
//! ```
//!
//! Any phase can transition to `Rejected(reason)` on failure.

pub mod apply;
pub mod cache;
pub mod download;
pub mod error;
pub mod install;
pub mod manager;
pub mod package;
pub mod rollback;
pub mod verify;

// Re-export key public types.
pub use apply::UpdateApplier;
pub use cache::PackageCache;
pub use download::Downloader;
pub use error::UpdateError;
pub use install::Installer;
pub use manager::PackageManager;
pub use package::{Package, PackageState};
pub use rollback::RollbackManager;
pub use verify::Verifier;
