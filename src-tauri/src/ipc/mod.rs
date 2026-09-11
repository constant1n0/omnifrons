//! Typed IPC surface for the Tauri shell: wire data shapes (`dto`) and the
//! commands that produce and consume them (`commands`).

pub mod catalog_repair;
pub mod commands;
pub mod dto;
pub mod guidance;
pub mod publication;
pub mod recovery;
pub mod wrong_root;
