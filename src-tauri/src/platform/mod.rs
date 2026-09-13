//! Cross-platform command execution foundation.
//!
//! This module provides typed abstractions for host OS detection, shell
//! resolution, command construction, environment overlays, path handling,
//! application launching, Docker preflight, and readiness checking.
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
//! - [`command_resolver`] — Authoritative command resolution: structured
//!   `ResolvedCommand`, platform-aware shim detection, shell-aware
//!   construction. All modules must delegate here for command resolution.
//! - [`shell_service`] — Shell validation, availability checking, fallback
//!   detection, and structured diagnostics.
//! - [`environment`] — `EnvironmentOverlay` for PATH prepend, env set/remove,
//!   applied to both sync and async commands. Includes secret redaction.
//! - [`paths`] — safe path comparison, executable resolution, suffix handling.
//! - [`ide`] — IDE / CLI discovery (PATH, registry, app bundles, flatpak).
//! - [`app_launcher`] — Native application resolution and structured
//!   launcher construction. Handles flatpak as structured commands.
//! - [`terminal`] — Terminal backend abstraction and launch plan resolution.
//! - [`docker_service`] — Docker CLI/daemon preflight, readiness checks,
//!   service status, and structured diagnostics.
//! - [`wsl`] — Windows Subsystem for Linux update automation so an outdated
//!   WSL2 never blocks Docker Desktop with an interactive prompt.
//! - [`readiness`] — TCP port and HTTP/HTTPS URL readiness checking with
//!   proper URL parsing, cancellation, and failure diagnostics.

pub mod app_launcher;
pub mod command;
pub mod command_resolver;
pub mod docker_service;
pub mod environment;
pub mod host;
pub mod ide;
pub mod paths;
pub mod readiness;
pub mod shell;
pub mod shell_service;
pub mod terminal;
pub mod wsl;

/// `CREATE_NO_WINDOW` (0x08000000): the child must not create a new
/// console window.
#[cfg(target_os = "windows")]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Hide the console of a background child process on Windows.
///
/// Release builds run as GUI applications without an attached console
/// (`windows_subsystem = "windows"` in `main.rs`), so every console-mode
/// child (cmd, powershell, reg, taskkill, npm...) would otherwise open a
/// visible console window that flashes and closes. Debug builds inherit
/// the developer console instead, which is why this problem only shows
/// up in release binaries (.exe/.msi) while `tauri dev` looks fine.
///
/// Works with both `std::process::Command` and `tokio::process::Command` —
/// both implement `std::os::windows::process::CommandExt`.
#[cfg(target_os = "windows")]
pub fn suppress_child_console<C: std::os::windows::process::CommandExt>(cmd: &mut C) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Same as [`suppress_child_console`], but for `tokio::process::Command`
/// (tokio exposes `creation_flags` natively instead of the std trait).
#[cfg(target_os = "windows")]
pub fn suppress_child_console_async(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}
