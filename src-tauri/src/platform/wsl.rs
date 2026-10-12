//! Windows Subsystem for Linux (WSL) readiness automation for Docker.
//!
//! Docker Desktop on Windows runs its Linux containers on the WSL2 backend.
//! When the installed WSL is outdated, the very first `wsl.exe` invocation
//! (which Docker Desktop performs at startup) drops into an interactive
//! console gate instead of launching the distro:
//!
//! ```text
//! Windows Subsystem for Linux needs a newer version to continue.
//! To update, run "wsl.exe --update".
//! Press any key to install Windows Subsystem for Linux...
//! The operation will time out in 60 seconds.
//! ```
//!
//! With nobody to press a key, the launcher exits after the 60-second
//! window and Docker Desktop records a startup failure, so the daemon never
//! becomes ready inside the step timeout. This module:
//!
//! - detects WSL presence (cached per session, so the UI doesn't spawn
//!   `wsl.exe` on every render);
//! - runs `wsl.exe --update` *before* Docker Desktop starts a Linux VM —
//!   the update is idempotent, and its result is cached for the session;
//! - installs WSL from scratch (`wsl --install --no-distribution`, requires
//!   a reboot) with stage progress for the UI;
//! - pins `wsl --set-default-version 2` after a fresh install.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// How long a probe of "is WSL installed?" stays cached. The UI polls this
/// on page open; spawning `wsl.exe` every time would be wasteful.
const PRESENCE_TTL: Duration = Duration::from_secs(5 * 60);
/// How long the result of `wsl --update` stays cached. An update result is
/// stable for the whole session; re-running the no-op on every docker launch
/// just wastes a couple hundred milliseconds. 30 minutes is a safe middle.
const ENSURE_TTL: Duration = Duration::from_secs(30 * 60);
/// Net install timeout: `wsl --install` downloads the whole WSL package.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// `wsl --set-default-version` only reconfigures an existing install.
const DEFAULT_VERSION_TIMEOUT: Duration = Duration::from_secs(60);
/// Probe timeout for `wsl --version` / `wsl --status` (never starts a VM).
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll cadence while waiting for a child to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

// ---------------------------------------------------------------------------
// Facing the frontend/orchestrator
// ---------------------------------------------------------------------------

/// Read-only WSL state for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WslStateView {
    /// `wsl.exe` resolves and responds — WSL is available on this machine.
    pub present: bool,
    /// The default WSL version is 2 (Docker requires WSL2). Informational:
    /// the launch flow still runs `wsl --update` regardless.
    pub wsl2_default: bool,
    /// Human-readable summary (probe artifacts, e.g. version errors).
    pub message: String,
}

/// Outcome of the pre-Docker WSL readiness routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WslEnsureOutcome {
    /// WSL is not applicable on this platform (non-Windows hosts). Nothing
    /// to automate — Docker uses its native backend there.
    NotApplicable,
    /// `wsl.exe` could not be executed at all. Installing WSL is an
    /// interactive decision (size, reboot) — the launch is blocked and the
    /// UI offers an install dialog instead.
    Missing,
    /// WSL is installed and already up to date; no work was required.
    AlreadyCurrent,
    /// WSL was updated by us; Docker Desktop can now start cleanly.
    Updated,
    /// An update was attempted (including the `--web-download` fallback)
    /// but could not complete. Docker may still show its own prompt.
    Failed(String),
}

impl WslEnsureOutcome {
    pub fn label(&self) -> &'static str {
        match self {
            WslEnsureOutcome::NotApplicable => "not applicable",
            WslEnsureOutcome::Missing => "wsl.exe missing",
            WslEnsureOutcome::AlreadyCurrent => "already current",
            WslEnsureOutcome::Updated => "updated",
            WslEnsureOutcome::Failed(_) => "failed",
        }
    }
}

/// Session-scoped presence cache: `(probed_at, present, wsl2_default)`.
static PRESENCE: std::sync::Mutex<Option<(Instant, bool, bool)>> = std::sync::Mutex::new(None);
/// Session-scoped ensure cache: `(ensured_at, outcome)`.
static ENSURE: std::sync::Mutex<Option<(Instant, WslEnsureOutcome)>> = std::sync::Mutex::new(None);

/// Current cached WSL state (re-probes once per `PRESENCE_TTL`).
///
/// Cheap: after the first call in a session this returns from memory. Never
/// runs `wsl --update` — that stays a launch-time action.
pub fn health() -> WslStateView {
    if !cfg!(target_os = "windows") {
        // WSL is a Windows-only subsystem: report it as absent (with an
        // explicit "not applicable" message) so no UI can mistake the
        // non-Windows host for a machine with WSL installed.
        return WslStateView {
            present: false,
            wsl2_default: false,
            message: "WSL is not applicable on this platform".to_string(),
        };
    }
    let mut cache = PRESENCE.lock().expect("wsl presence cache poisoned");
    let fresh = cache
        .map(|(at, _, _)| at.elapsed() < PRESENCE_TTL)
        .unwrap_or(false);
    let (present, wsl2_default) = if fresh {
        cache.map(|(_, p, d)| (p, d)).unwrap_or((false, false))
    } else {
        let probed = probe_presence();
        *cache = Some((Instant::now(), probed.0, probed.1));
        probed
    };
    WslStateView {
        present,
        wsl2_default,
        message: presence_message(present, wsl2_default),
    }
}

/// Drop both session caches. Called after an install (or install failure) so
/// the UI and orchestrator re-probe reality instead of stale cache entries.
pub fn invalidate_cache() {
    if let Ok(mut c) = PRESENCE.lock() {
        *c = None;
    }
    if let Ok(mut c) = ENSURE.lock() {
        *c = None;
    }
}

/// Ensure WSL is up to date before Docker Desktop is launched.
///
/// * Windows with `wsl.exe` on PATH → runs `wsl.exe --update` (idempotent),
///   falling back to `wsl.exe --update --web-download` when the primary path
///   fails; result cached for `ENSURE_TTL`.
/// * Windows without `wsl.exe` → [`WslEnsureOutcome::Missing`].
/// * Other platforms → [`WslEnsureOutcome::NotApplicable`].
///
/// This is a blocking call: a real update can spend minutes downloading the
/// WSL package. Call it from `spawn_blocking` in async contexts.
pub fn ensure_ready_before_docker() -> WslEnsureOutcome {
    if !cfg!(target_os = "windows") {
        return WslEnsureOutcome::NotApplicable;
    }
    // Fast path: the last ensure result is still fresh. This is what makes
    // repeated docker launches cheap — the `wsl --update` no-op (a couple
    // hundred ms per spawn) is not re-run every time.
    if let Ok(cache) = ENSURE.lock() {
        if let Some((at, outcome)) = cache.as_ref() {
            if at.elapsed() < ENSURE_TTL {
                return outcome.clone();
            }
        }
    }
    let outcome = ensure_windows();
    if let Ok(mut cache) = ENSURE.lock() {
        *cache = Some((Instant::now(), outcome.clone()));
    }
    outcome
}

/// Install WSL from scratch (`wsl --install --no-distribution`), then pin the
/// default version to 2. Long-running; `progress` receives human-readable
/// stages ("скачиваем WSL…", "настраиваем WSL 2…"). Returns a success message
/// for the "reboot your PC" dialog. A reboot is required after success.
pub fn install(mut progress: impl FnMut(&str)) -> Result<String, String> {
    invalidate_cache();
    if !cfg!(target_os = "windows") {
        return Err("WSL is only applicable on Windows hosts".to_string());
    }
    if !wsl_available_now() {
        progress("Запускаем установку WSL (около 400 МБ)…");
        let (code, output) = run_command("wsl.exe", &["--install", "--no-distribution"], INSTALL_TIMEOUT)
            .map_err(|e| e)?;
        match code {
            Some(0) => {
                progress("WSL установлен — настраиваем версию по умолчанию (WSL 2)…");
                let _ = set_default_version_two();
                progress("Готово — перезагрузите ПК, чтобы изменения вступили в силу");
                Ok("WSL установлен. Перезагрузите ПК, чтобы продолжить.".to_string())
            }
            Some(code) => Err(format!(
                "Установка WSL завершилась с кодом {code}: {output}"
            )),
            None => Err("Установка WSL была прервана системой".to_string()),
        }
    } else {
        // WSL is already present (stale cache race): nothing to install, but
        // make sure the default version is pinned so Docker has WSL2.
        progress("WSL уже установлен — проверяем версию по умолчанию…");
        set_default_version_two().map_err(|e| {
            format!("Не удалось включить WSL 2 по умолчанию: {e}")
        })?;
        progress("Готово — перезагрузите ПК, чтобы изменения вступили в силу");
        Ok("WSL настроен. Перезагрузите ПК, чтобы продолжить.".to_string())
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

/// Full ensure pass on Windows: probe presence, update when present.
fn ensure_windows() -> WslEnsureOutcome {
    let Some((present, _wsl2_default)) = cached_presence_or_probe() else {
        return WslEnsureOutcome::Missing;
    };
    if !present {
        return WslEnsureOutcome::Missing;
    }
    match run_update(&[]) {
        Ok(output) => {
            if output_is_already_current(&output) {
                WslEnsureOutcome::AlreadyCurrent
            } else {
                WslEnsureOutcome::Updated
            }
        }
        Err(primary) => match run_update(&["--web-download"]) {
            Ok(output) => {
                if output_is_already_current(&output) {
                    WslEnsureOutcome::AlreadyCurrent
                } else {
                    WslEnsureOutcome::Updated
                }
            }
            Err(fallback) => WslEnsureOutcome::Failed(format!(
                "wsl.exe --update failed ({primary}); --web-download fallback failed ({fallback})"
            )),
        },
    }
}

/// Cached presence, refreshing from disk `wsl.exe` when stale.
fn cached_presence_or_probe() -> Option<(bool, bool)> {
    let mut cache = PRESENCE.lock().expect("wsl presence cache poisoned");
    let fresh = cache
        .map(|(at, _, _)| at.elapsed() < PRESENCE_TTL)
        .unwrap_or(false);
    if fresh {
        return cache.map(|(_, p, d)| (p, d));
    }
    let probed = probe_presence();
    *cache = Some((Instant::now(), probed.0, probed.1));
    Some(probed)
}

/// Probe `wsl.exe --version` (never starts the VM / never blocks on the
/// interactive gate) plus `wsl --status` for the default version. Returns
/// `(present, wsl2_default)`.
fn probe_presence() -> (bool, bool) {
    let present = wsl_available_now();
    let wsl2_default = present && default_version_is_two();
    (present, wsl2_default)
}

/// `wsl.exe --version` spawns successfully and exits → WSL is available.
fn wsl_available_now() -> bool {
    match run_command("wsl.exe", &["--version"], PROBE_TIMEOUT) {
        Ok((Some(_), _)) => true,
        _ => false,
    }
}

/// Whether `wsl --status` reports a default version of 2.
fn default_version_is_two() -> bool {
    match run_command("wsl.exe", &["--status"], PROBE_TIMEOUT) {
        Ok((_, output)) => parse_default_version(&output) == Some(2),
        _ => false,
    }
}

/// Pin the default WSL version to 2 (`wsl --set-default-version 2`).
fn set_default_version_two() -> Result<String, String> {
    let (code, output) =
        run_command("wsl.exe", &["--set-default-version", "2"], DEFAULT_VERSION_TIMEOUT)?;
    match code {
        Some(0) => Ok(output),
        Some(code) => Err(format!(
            "wsl.exe --set-default-version 2 exited with code {code}: {output}"
        )),
        None => Err("wsl.exe --set-default-version 2 was terminated by the system".to_string()),
    }
}

/// Run `wsl.exe --update [extra...]` and return normalized output.
fn run_update(args: &[&str]) -> Result<String, String> {
    let mut full_args: Vec<&str> = Vec::with_capacity(args.len() + 1);
    full_args.push("--update");
    full_args.extend_from_slice(args);
    let (code, output) = run_command("wsl.exe", &full_args, Duration::from_secs(600))?;
    match code {
        Some(0) => Ok(output),
        Some(code) => Err(format!(
            "wsl.exe --update exited with code {code}: {output}"
        )),
        None => Err("wsl.exe --update was terminated by the system".to_string()),
    }
}

/// Run an executable hidden, capturing stdout+stderr, bounded by `timeout`.
/// Returns `(exit_code, merged_output)`. Spawn failure (missing binary) is a
/// plain `Err`.
fn run_command(program: &str, args: &[&str], timeout: Duration) -> Result<(Option<i32>, String), String> {
    #[cfg(target_os = "windows")]
    {
        use std::process::{Command, Output, Stdio};

        let mut cmd = Command::new(program);
        cmd.args(args);
        crate::platform::suppress_child_console(&mut cmd);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child =
            cmd.spawn()
                .map_err(|e| format!("failed to run {}: {e}", program))?;
        let deadline = Instant::now() + timeout;
        let exit_code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {}
                Err(e) => return Err(format!("waiting for {program} failed: {e}")),
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out after {}s", timeout.as_secs()));
            }
            std::thread::sleep(POLL_INTERVAL);
        };
        let output: Output = child
            .wait_with_output()
            .map_err(|e| format!("reading {program} output failed: {e}"))?;
        let merged = merge_output(&output);
        Ok((exit_code, merged))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (program, args, timeout);
        Err("command execution is not available on this platform".to_string())
    }
}

/// `wsl.exe --update` prints a no-op notice when nothing needs updating.
fn output_is_already_current(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("already")
        || lower.contains("already installed")
        || lower.contains("уже установлена")
        || lower.contains("уже есть")
}

/// Human-readable presence summary for `WslStateView.message`.
fn presence_message(present: bool, wsl2_default: bool) -> String {
    match (present, wsl2_default) {
        (false, _) => "wsl.exe is not available; Windows Subsystem for Linux needs to be installed".to_string(),
        (true, true) => "Windows Subsystem for Linux is installed (default version: 2)".to_string(),
        (true, false) => {
            "Windows Subsystem for Linux is installed but the default version \
             is not WSL 2 — it will be pinned on Docker start"
                .to_string()
        }
    }
}

/// Merge stdout and stderr, trimming each, dropping empties.
fn merge_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout} | {stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => String::new(),
    }
}

/// Strip the default WSL version out of `wsl --status` output.
fn parse_default_version(output: &str) -> Option<u8> {
    let lower = output.to_ascii_lowercase();
    for pat in [
        "default version: ",
        "default version is ",
        "default version of ",
    ] {
        if let Some(idx) = lower.find(pat) {
            let rest = &lower[idx + pat.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u8>() {
                return Some(n);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    use super::output_is_already_current;

    #[test]
    fn outcome_labels_are_stable() {
        assert_eq!(WslEnsureOutcome::NotApplicable.label(), "not applicable");
        assert_eq!(WslEnsureOutcome::Missing.label(), "wsl.exe missing");
        assert_eq!(WslEnsureOutcome::AlreadyCurrent.label(), "already current");
        assert_eq!(WslEnsureOutcome::Updated.label(), "updated");
        assert_eq!(WslEnsureOutcome::Failed(String::new()).label(), "failed");
    }

    #[test]
    fn default_version_parsing() {
        assert_eq!(parse_default_version("Default Version: 2"), Some(2));
        assert_eq!(
            parse_default_version(
                "The Windows Subsystem for Linux has a Default Version of 2."
            ),
            Some(2)
        );
        assert_eq!(parse_default_version("default version: 1"), Some(1));
        assert_eq!(parse_default_version("no version info here"), None);
    }

    #[test]
    fn presence_message_tracks_state() {
        assert!(presence_message(false, false).contains("install"));
        assert!(presence_message(true, true).contains("version: 2"));
        assert!(presence_message(true, false).contains("WSL 2"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn already_current_detection() {
        assert!(output_is_already_current(
            "Windows Subsystem for Linux is already installed."
        ));
        assert!(output_is_already_current("wsl уже установлена"));
        assert!(!output_is_already_current(
            "Installing: Windows Subsystem for Linux 2.4.6"
        ));
    }
}