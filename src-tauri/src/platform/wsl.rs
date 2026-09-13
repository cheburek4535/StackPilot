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
//! becomes ready inside the step timeout. This module runs the same
//! `wsl.exe --update` *proactively*, before Docker Desktop — and therefore
//! the WSL2 VM — is started, so the interactive blocker never appears.
//! The update is idempotent: when the current version is already installed
//! `wsl.exe --update` exits immediately without changes.

/// Outcome of the pre-Docker WSL readiness routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WslEnsureOutcome {
    /// WSL is not applicable on this platform (non-Windows hosts). Nothing
    /// to automate — Docker uses its native backend there.
    NotApplicable,
    /// `wsl.exe` could not be executed at all. Docker Desktop will surface
    /// its own "install Windows Subsystem for Linux" experience.
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

/// Ensure WSL is up to date before Docker Desktop is launched.
///
/// * Windows with `wsl.exe` on PATH → runs `wsl.exe --update` (idempotent),
///   falling back to `wsl.exe --update --web-download` when the primary path
///   fails.
/// * Windows without `wsl.exe` → [`WslEnsureOutcome::Missing`].
/// * Other platforms → [`WslEnsureOutcome::NotApplicable`].
///
/// This is a blocking call: a real update can spend minutes downloading the
/// WSL package. Call it from `spawn_blocking` in async contexts.
pub fn ensure_ready_before_docker() -> WslEnsureOutcome {
    if !cfg!(target_os = "windows") {
        return WslEnsureOutcome::NotApplicable;
    }
    _windows::ensure_ready()
}

/// Windows-only implementation.
#[cfg(target_os = "windows")]
mod _windows {
    use super::WslEnsureOutcome;
    use crate::platform::suppress_child_console;
    use std::process::{Command, Output, Stdio};
    use std::time::{Duration, Instant};

    /// `wsl.exe --update` can download and install the whole WSL package;
    /// a generous cap keeps a stalled download from hanging the launch.
    const UPDATE_TIMEOUT: Duration = Duration::from_secs(600);
    /// Casual poll cadence while waiting for the child to exit.
    const POLL_INTERVAL: Duration = Duration::from_millis(200);

    pub(super) fn ensure_ready() -> WslEnsureOutcome {
        if !wsl_available() {
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

    /// Probe `wsl.exe --version` so a missing/unsupported WSL (which cannot
    /// be updated from here) is reported as `Missing` rather than a failed
    /// update. `--version` only prints version information; it never starts
    /// the WSL2 VM and therefore never triggers the interactive update gate.
    fn wsl_available() -> bool {
        let mut cmd = Command::new("wsl.exe");
        cmd.arg("--version");
        suppress_child_console(&mut cmd);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let Ok(mut child) = cmd.spawn() else {
            return false;
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) => {}
                Err(_) => break false,
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                break false;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Run `wsl.exe --update [extra...]` and return normalized output.
    fn run_update(args: &[&str]) -> Result<String, String> {
        let mut cmd = Command::new("wsl.exe");
        cmd.arg("--update");
        cmd.args(args);
        suppress_child_console(&mut cmd);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to run wsl.exe: {e}"))?;

        let deadline = Instant::now() + UPDATE_TIMEOUT;
        let exit_reason = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {}
                Err(e) => return Err(format!("waiting for wsl.exe failed: {e}")),
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "wsl.exe --update timed out after {}s",
                    UPDATE_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        };

        let output: Output = child
            .wait_with_output()
            .map_err(|e| format!("reading wsl.exe output failed: {e}"))?;

        let merged = merge_output(&output);
        match exit_reason {
            Some(0) => Ok(merged),
            Some(code) => Err(format!(
                "wsl.exe --update exited with code {code}: {merged}"
            )),
            None => Err(format!("wsl.exe terminated by signal: {merged}")),
        }
    }

    /// `wsl.exe --update` prints a no-op notice when nothing needs updating.
    pub(super) fn output_is_already_current(output: &str) -> bool {
        let lower = output.to_ascii_lowercase();
        lower.contains("already")
            || lower.contains("already installed")
            || lower.contains("уже установлена")
            || lower.contains("уже есть")
    }

    fn merge_output(output: &Output) -> String {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => format!("{stdout} | {stderr}"),
            (false, true) => stdout,
            (true, false) => stderr,
            (true, true) => String::new(),
        }
    }
}

/// Non-Windows hosts: no WSL, nothing to do.
#[cfg(not(target_os = "windows"))]
mod _windows {
    use super::WslEnsureOutcome;

    pub(super) fn ensure_ready() -> WslEnsureOutcome {
        WslEnsureOutcome::NotApplicable
    }
}

#[cfg(test)]
mod tests {
    use super::WslEnsureOutcome;

    #[test]
    fn outcome_labels_are_stable() {
        assert_eq!(WslEnsureOutcome::NotApplicable.label(), "not applicable");
        assert_eq!(WslEnsureOutcome::Missing.label(), "wsl.exe missing");
        assert_eq!(WslEnsureOutcome::AlreadyCurrent.label(), "already current");
        assert_eq!(WslEnsureOutcome::Updated.label(), "updated");
        assert_eq!(WslEnsureOutcome::Failed(String::new()).label(), "failed");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn already_current_detection() {
        use super::_windows::output_is_already_current;

        assert!(output_is_already_current(
            "Windows Subsystem for Linux is already installed."
        ));
        assert!(output_is_already_current("wsl уже установлена"));
        assert!(!output_is_already_current(
            "Installing: Windows Subsystem for Linux 2.4.6"
        ));
    }
}
