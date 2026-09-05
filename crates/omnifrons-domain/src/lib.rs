//! Framework-independent domain core for Omnifrons.
//!
//! This crate depends on `std` and `thiserror` only (see
//! `docs/repository-layout.md` § Crate map). It never depends on Tauri,
//! Tokio, or any adapter, so that domain, scope, and process-terminal-state
//! contracts run and test without a desktop shell or an async runtime.

pub mod scope;
