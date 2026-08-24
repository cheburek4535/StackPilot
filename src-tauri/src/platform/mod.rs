//! Cross-platform command execution foundation.
//!
//! This module provides typed abstractions for host OS detection, shell
//! resolution, command construction, environment overlays, and path handling.
//! Business modules should use these types instead of hard-coding `cmd`,
//! `sh`, `taskkill`, `where`/`which`, PATH separator behavior, or
//! executable suffix logic.
//!
//! # Module overview
//!
//! - [`host`] — `HostOs` / `HostArch` enums and current-host detection.
//! - [`shell`] — `ShellKind` enum, legacy string parsing, default resolution.
//! - [`command`] — `tokio::process::Command` / `std::process::Command`
//!   builders for direct and shell execution modes.
//! - [`environment`] — `EnvironmentOverlay` for PATH prepend, env set/remove,
//!   applied to both sync and async commands.
//! - [`paths`] — safe path comparison, executable resolution, suffix handling.

pub mod command;
pub mod environment;
pub mod host;
pub mod paths;
pub mod shell;
