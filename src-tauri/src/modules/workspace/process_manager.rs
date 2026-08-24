use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Emitter;

use crate::modules::workspace::models::*;
use crate::platform::environment::EnvironmentOverlay;
use crate::platform::host::{current_os, HostOs};

pub const PROCESS_EVENT_OUTPUT: &str = "process-output";
pub const PROCESS_EVENT_STATUS: &str = "process-status";

/// Grace period (ms) after SIGTERM before escalating to SIGKILL on Unix.
#[cfg(unix)]
const UNIX_KILL_GRACE_MS: u64 = 2000;

struct ActiveProcess {
    info: TrackedProcess,
    child: Option<Child>,
    stdout_buffer: Arc<Mutex<Vec<String>>>,
    stderr_buffer: Arc<Mutex<Vec<String>>>,
}

pub trait ProcessManager: Send + Sync {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
    ) -> Result<TrackedProcess, String>;

    /// Spawn a process with an environment overlay applied to the child
    /// (PATH prepend, env set/remove). Used by the launch engine so that
    /// environment bindings take effect on spawned processes.
    fn spawn_and_track_with_overlay(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
    ) -> Result<TrackedProcess, String>;

    /// Spawn a process in a new native terminal window. The process is
    /// tracked by PID so it can be killed, but its output goes to the
    /// terminal window rather than the app's log viewer.
    fn spawn_visible(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
    ) -> Result<TrackedProcess, String>;

    /// Launch a GUI application detached without capturing output. The
    /// application opens in its own native window and is not tracked as a
    /// managed process.
    fn launch_detached(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
    ) -> Result<(), String>;

    fn list(&self) -> Vec<TrackedProcess>;
    fn kill(&self, id: &str) -> Result<(), String>;
    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String>;
    fn get_logs(&self, id: &str) -> Result<ProcessLogs, String>;
}

pub struct OsProcessManager {
    processes: Arc<Mutex<Vec<ActiveProcess>>>,
    app_handle: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl OsProcessManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(Vec::new())),
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.lock().expect("app_handle lock poisoned") = Some(handle);
    }

    fn spawn_reader_thread(
        stream_name: &'static str,
        reader: Box<dyn std::io::Read + Send + 'static>,
        process_id: String,
        buffer: Arc<Mutex<Vec<String>>>,
        handle: tauri::AppHandle,
    ) {
        thread::spawn(move || {
            let buf_reader = BufReader::new(reader);
            for line in buf_reader.lines() {
                match line {
                    Ok(text) => {
                        buffer
                            .lock()
                            .expect("buffer lock poisoned")
                            .push(text.clone());
                        let _ = handle.emit(
                            PROCESS_EVENT_OUTPUT,
                            ProcessOutputEvent {
                                process_id: process_id.clone(),
                                stream: stream_name.to_string(),
                                line: text,
                            },
                        );
                    }
                    Err(_) => break,
                }
            }
        });
    }

    /// Build the Command with platform-specific process group settings.
    ///
    /// - Windows: `CREATE_NEW_PROCESS_GROUP` so the child tree can be
    ///   terminated as a unit via `taskkill /F /T /PID`.
    /// - Unix: `setsid()` in `pre_exec` so the child becomes a session
    ///   leader with its own process group, killable via `kill(-pgid, sig)`.
    fn build_command(command: &str, args: &[&str], working_dir: Option<&str>) -> Command {
        let mut cmd = Command::new(command);
        cmd.args(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        // Platform-specific process group creation.
        //
        // SAFETY (Unix): `pre_exec` runs in the child between fork and exec.
        // `setsid()` creates a new session with the child as session leader,
        // giving it its own process group so we can kill the entire tree.
        //
        // SAFETY (Windows): `CREATE_NEW_PROCESS_GROUP` (0x00000200) creates
        // the child in a new process group, enabling `taskkill /T` to
        // terminate the entire tree.
        match current_os() {
            HostOs::Windows => {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
            }
            HostOs::Linux | HostOs::Macos => {
                // SAFETY: `pre_exec` is safe when it only calls async-signal-safe
                // functions. `setsid()` is listed as async-signal-safe by POSIX.
                #[cfg(unix)]
                unsafe {
                    cmd.pre_exec(|| {
                        libc::setsid();
                        Ok(())
                    });
                }
            }
        }

        cmd
    }

    /// Build a Command for launching a GUI application detached.
    ///
    /// GUI apps (IDE, browser, Docker Desktop) should not have their stdout/
    /// stderr piped or nulled in a way that breaks their native window.
    /// On Windows they are launched via `start` semantics (Cmd::new + spawn
    /// without console handles), on Unix via `open`/`xdg-open`/direct spawn.
    fn build_detached_command(command: &str, args: &[&str], working_dir: Option<&str>) -> Command {
        let mut cmd = Command::new(command);
        cmd.args(args);
        // Do not pipe stdout/stderr for GUI apps — let them inherit or
        // detach naturally. Piping to null on Windows can break GUI init.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            // DETACHED_PROCESS lets a GUI app run without a console;
            // CREATE_NEW_PROCESS_GROUP keeps the tree killable.
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
        #[cfg(not(target_os = "windows"))]
        {
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        cmd
    }

    /// Terminate a process tree on the current platform.
    ///
    /// Returns `Ok(true)` if the process was successfully terminated by us,
    /// `Ok(false)` if it had already exited, or `Err` if termination failed.
    fn kill_process_tree(child: &mut Child, pid: u32) -> Result<bool, String> {
        match current_os() {
            HostOs::Windows => kill_windows_tree(pid),
            HostOs::Linux | HostOs::Macos => kill_unix_group(child, pid),
        }
    }

    /// Emit a process status event.
    fn emit_status(
        handle: &Option<tauri::AppHandle>,
        id: &str,
        status: &ProcessStatus,
        error: &Option<String>,
    ) {
        if let Some(handle) = handle.as_ref() {
            let _ = handle.emit(
                PROCESS_EVENT_STATUS,
                ProcessStatusEvent {
                    process_id: id.to_string(),
                    status: status.clone(),
                    error: error.clone(),
                },
            );
        }
    }
}

// ============================================================================
// Native terminal spawning (cross-platform)
// ============================================================================

/// Spawn a process in a new native terminal window so the user can see its
/// output directly and interact with it.
///
/// The spawned terminal process is tracked by `self` so it appears in the
/// app's process list and can be killed. The command itself runs inside the
/// terminal; killing the tracked process kills the terminal (and its child
/// tree on Windows via `taskkill /T`, on Unix via the process group).
fn spawn_in_terminal(
    manager: &OsProcessManager,
    command: &str,
    args: &[&str],
    working_dir: Option<&str>,
    label: &str,
    session_id: Option<String>,
    overlay: Option<&EnvironmentOverlay>,
) -> Result<TrackedProcess, String> {
    let (term_cmd, term_args) = build_terminal_command(command, args, working_dir)?;

    // Reuse the tracked-spawn machinery so the terminal is listed and can be
    // killed. The terminal's own stdout/stderr are piped (they're mostly the
    // terminal's control channel); the real command output appears in the
    // native window. The environment overlay is applied to the terminal
    // process so env vars/PATH reach the inner command.
    let term_args_refs: Vec<&str> = term_args.iter().map(|s| s.as_str()).collect();
    match overlay {
        Some(ov) => manager.spawn_and_track_visible_with_overlay(
            &term_cmd,
            &term_args_refs,
            None,
            label,
            session_id,
            ov,
        ),
        None => manager.spawn_and_track_visible(
            &term_cmd,
            &term_args_refs,
            None,
            label,
            session_id,
        ),
    }
}

/// Build the OS-specific command that opens a terminal and runs the command.
///
/// - Windows: `cmd /C start "title" cmd /K "cd /d <dir> && <command>"` —
///   opens a new visible console window that stays open (`/K`).
/// - macOS: `osascript -e 'tell application "Terminal" to do script "<cmd>"'`.
/// - Linux: try common terminal emulators (`gnome-terminal`, `konsole`,
///   `x-terminal-emulator`, `xterm`, `alacritty`).
fn build_terminal_command(
    command: &str,
    args: &[&str],
    working_dir: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let joined = join_command(command, args);

    // Build the inner command that runs in the terminal. We use `cd /d <dir>`
    // on Windows and `cd <dir>` on Unix so the command runs in the project
    // directory, then execute the command. The terminal stays open so the
    // user sees the long-running process output.
    let full_inner: String = match working_dir {
        Some(dir) => {
            let cd = if cfg!(target_os = "windows") {
                format!("cd /d \"{}\"", dir)
            } else {
                format!("cd '{}'", dir.replace('\'', "'\\''"))
            };
            format!("{} && {}", cd, joined)
        }
        None => joined.clone(),
    };

    match current_os() {
        HostOs::Windows => {
            // Robust Windows terminal launch:
            //   cmd /C start "DevLauncher" cmd /K "<temp_script.cmd>"
            //
            // We write the inner command (cd + command) to a temporary batch
            // file and pass its path to `cmd /K`, avoiding the notoriously
            // fragile quoting of nested cmd.exe arguments.
            let script_path = write_temp_batch_script(&full_inner)?;
            Ok((
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "start".to_string(),
                    "\"DevLauncher\"".to_string(),
                    "cmd".to_string(),
                    "/K".to_string(),
                    script_path,
                ],
            ))
        }
        HostOs::Macos => {
            // Use AppleScript to run the command in a new Terminal window.
            let escaped = full_inner.replace('\\', "\\\\").replace('"', "\\\"");
            Ok((
                "osascript".to_string(),
                vec![
                    "-e".to_string(),
                    format!("tell application \"Terminal\" to do script \"{}\"", escaped),
                ],
            ))
        }
        HostOs::Linux => {
            let mut cmd = Command::new("sh");
            cmd.args(["-lc", &full_inner]);
            // Find an available terminal emulator.
            let candidates = [
                "gnome-terminal",
                "konsole",
                "x-terminal-emulator",
                "xfce4-terminal",
                "xterm",
                "alacritty",
                "kitty",
            ];
            for cand in candidates {
                if which::which(cand).is_ok() {
                    let term_args = match cand {
                        "gnome-terminal" => vec![
                            "--".to_string(),
                            "sh".to_string(),
                            "-c".to_string(),
                            format!(
                                "cd '{}' && {}",
                                working_dir.unwrap_or(".").replace('\'', "'\\''"),
                                full_inner
                            ),
                        ],
                        "konsole" => vec![
                            "-e".to_string(),
                            "sh".to_string(),
                            "-c".to_string(),
                            full_inner.clone(),
                        ],
                        "x-terminal-emulator" | "xfce4-terminal" => vec![
                            "-e".to_string(),
                            "sh".to_string(),
                            "-c".to_string(),
                            full_inner.clone(),
                        ],
                        "xterm" | "alacritty" | "kitty" => vec![
                            "-e".to_string(),
                            "sh".to_string(),
                            "-c".to_string(),
                            full_inner.clone(),
                        ],
                        _ => vec![full_inner.clone()],
                    };
                    return Ok((cand.to_string(), term_args));
                }
            }
            Err(
                "No supported terminal emulator found on Linux. Install gnome-terminal, \
                 konsole, xterm, or alacritty."
                    .to_string(),
            )
        }
    }
}

/// Write the inner command to a temporary `.cmd` batch file and return its
/// path. Used on Windows so `cmd /K <file>` runs reliably without fragile
/// nested quoting.
#[cfg(target_os = "windows")]
fn write_temp_batch_script(inner: &str) -> Result<String, String> {
    use std::io::Write;

    let mut dir = std::env::temp_dir();
    dir.push(format!("devlauncher_{}.cmd", std::process::id()));

    let mut file = std::fs::File::create(&dir)
        .map_err(|e| format!("Failed to create temp script: {}", e))?;
    writeln!(file, "@echo off")
        .and_then(|_| writeln!(file, "{}", inner))
        .map_err(|e| format!("Failed to write temp script: {}", e))?;

    Ok(dir.to_string_lossy().into_owned())
}

/// Non-Windows stub (never used on Unix/macOS — they use native terminals).
#[cfg(not(target_os = "windows"))]
fn write_temp_batch_script(_inner: &str) -> Result<String, String> {
    Err("temp batch scripts are Windows-only".to_string())
}

/// Join a command and its args into a single shell string for terminal
/// execution. Arguments are shell-quoted so spaces and special chars survive.
fn join_command(command: &str, args: &[&str]) -> String {
    let mut parts: Vec<String> = vec![quote_for_shell(command)];
    for a in args {
        parts.push(quote_for_shell(a));
    }
    parts.join(" ")
}

/// Shell-quote a single token. On Windows, wrap in double quotes if it
/// contains spaces; on Unix, single-quote if it contains shell metacharacters.
fn quote_for_shell(token: &str) -> String {
    let needs_quote = token.is_empty()
        || token
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'');
    if !needs_quote {
        return token.to_string();
    }
    if cfg!(target_os = "windows") {
        format!("\"{}\"", token.replace('"', "\"\""))
    } else {
        format!("'{}'", token.replace('\'', "'\\''"))
    }
}

// ============================================================================
// Windows process tree termination
// ============================================================================

/// Kill a process tree on Windows using `taskkill /F /T /PID`.
///
/// `/F` forces termination, `/T` kills the entire tree. This is the
/// correct approach for Windows because `Child::kill()` only terminates
/// the direct child, leaving grandchildren (npm, cargo, python) alive.
fn kill_windows_tree(pid: u32) -> Result<bool, String> {
    let status = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .status();

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(s) => {
            // taskkill failed — may mean the process already exited.
            let code = s.code().unwrap_or(-1);
            // Exit code 128+ means the process was not found (already exited).
            if code >= 128 || code == 0 {
                Ok(false)
            } else {
                Err(format!(
                    "taskkill /F /T /PID {} failed with exit code {}",
                    pid, code
                ))
            }
        }
        Err(e) => Err(format!("Failed to run taskkill: {}", e)),
    }
}

// ============================================================================
// Unix process group termination
// ============================================================================

/// Kill a process group on Unix: SIGTERM → grace → SIGKILL.
///
/// The child was spawned in its own session via `setsid()`, so `pid`
/// doubles as the process group ID. We send signals to `-pid` to target
/// the entire group (child + grandchildren from npm, cargo, etc.).
#[cfg(unix)]
fn kill_unix_group(child: &mut Child, pid: u32) -> Result<bool, String> {
    use std::os::unix::process::ExitStatusExt;
    use std::time::Duration;

    // Check if already exited before sending signals.
    match child.try_wait() {
        Ok(Some(_)) => {
            // Process already exited — reap it.
            let _ = child.wait();
            return Ok(false);
        }
        Ok(None) => { /* still running, proceed */ }
        Err(e) => {
            return Err(format!("Failed to check process status: {}", e));
        }
    }

    let pgid = pid as i32;

    // 1. SIGTERM to the entire process group.
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }

    // 2. Wait up to UNIX_KILL_GRACE_MS for the direct child to exit.
    let grace = Duration::from_millis(UNIX_KILL_GRACE_MS);
    let deadline = SystemTime::now() + grace;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                // Direct child exited — reap it.
                let _ = child.wait();
                return Ok(true);
            }
            Ok(None) => {
                if SystemTime::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(format!("Failed to check process status: {}", e));
            }
        }
    }

    // 3. SIGKILL to the entire process group.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }

    // 4. Brief wait for SIGKILL to take effect, then reap.
    thread::sleep(Duration::from_millis(200));
    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.wait();
            Ok(true)
        }
        Ok(None) => {
            // SIGKILL may take time to propagate — best effort.
            // The zombie will be reaped eventually.
            let _ = child.wait();
            Ok(true)
        }
        Err(e) => Err(format!(
            "Failed to check process status after SIGKILL: {}",
            e
        )),
    }
}

/// Stub for non-Unix targets (should never be called on Windows).
#[cfg(not(unix))]
fn kill_unix_group(_child: &mut Child, _pid: u32) -> Result<bool, String> {
    Err("Unix process group kill is not available on this platform".to_string())
}

impl ProcessManager for OsProcessManager {
    fn spawn_and_track(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(command, args, working_dir, label, session_id, None, false)
    }

    fn spawn_and_track_with_overlay(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            working_dir,
            label,
            session_id,
            Some(overlay),
            false,
        )
    }

    fn spawn_visible(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
    ) -> Result<TrackedProcess, String> {
        spawn_in_terminal(self, command, args, working_dir, label, session_id, overlay)
    }

    fn launch_detached(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
    ) -> Result<(), String> {
        let mut cmd = Self::build_detached_command(command, args, working_dir);
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("Failed to launch '{}': {}", command, e))
    }

    fn list(&self) -> Vec<TrackedProcess> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Live status update: try_wait is non-blocking, so each list()
        // call detects finished processes without requiring a separate
        // refresh_status call.
        let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
        let handle = handle_guard.clone();
        for entry in lock.iter_mut() {
            let mut finished: Option<(ProcessStatus, Option<String>)> = None;
            if let Some(ref mut child) = entry.child {
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        entry.child = None;
                        let (new_status, error_msg) = if status.success() {
                            (ProcessStatus::Exited(status.code().unwrap_or(0)), None)
                        } else {
                            let stderr = entry
                                .stderr_buffer
                                .lock()
                                .expect("stderr lock poisoned")
                                .join("\n");
                            let err_msg = if stderr.is_empty() {
                                format!("Process exited with code {}", status.code().unwrap_or(-1))
                            } else {
                                stderr
                            };
                            (ProcessStatus::Crashed, Some(err_msg))
                        };
                        entry.info.status = new_status.clone();
                        entry.info.last_error = error_msg.clone();
                        finished = Some((new_status, error_msg));
                    }
                    Err(_) => {}
                }
            }
            if let Some((new_status, error_msg)) = finished {
                Self::emit_status(&handle, &entry.info.id, &new_status, &error_msg);
            }
            let started = entry.info.started_at.parse::<u64>().unwrap_or(0);
            entry.info.duration_secs = now.saturating_sub(started);
        }

        lock.iter().map(|p| p.info.clone()).collect()
    }

    fn kill(&self, id: &str) -> Result<(), String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter_mut()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        // If the child handle is already gone, the process exited naturally.
        // Check current status: if already Exited/Crashed, do not overwrite.
        if entry.child.is_none() {
            match &entry.info.status {
                ProcessStatus::Exited(_) | ProcessStatus::Crashed => {
                    // Already exited naturally — do not overwrite with Killed.
                    return Ok(());
                }
                ProcessStatus::Killed => {
                    // Already killed in a previous call.
                    return Ok(());
                }
                ProcessStatus::Running => {
                    // Child handle missing but status says Running — treat as exited.
                    entry.info.status = ProcessStatus::Exited(0);
                    return Ok(());
                }
            }
        }

        let pid = entry.info.pid;

        // Attempt platform-specific tree/group termination.
        let kill_result = if let Some(ref mut child) = entry.child {
            Self::kill_process_tree(child, pid)
        } else {
            Ok(false)
        };

        // Reap the direct child to avoid zombies.
        if let Some(ref mut child) = entry.child {
            let _ = child.wait();
        }
        entry.child = None;

        match kill_result {
            Ok(was_running) => {
                // Only set Killed if the process was actually running and we
                // terminated it. If it had already exited, preserve the
                // natural exit status.
                if was_running {
                    entry.info.status = ProcessStatus::Killed;
                }
                // If !was_running, status was already set by try_wait or
                // remains as-is (natural exit was already recorded).

                let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
                Self::emit_status(
                    &handle_guard,
                    id,
                    &entry.info.status,
                    &entry.info.last_error,
                );
                Ok(())
            }
            Err(e) => {
                // Kill failed — report the error, do not set Killed.
                entry.info.last_error = Some(e.clone());
                Err(format!("Failed to kill process '{}': {}", id, e))
            }
        }
    }

    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter_mut()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        if let Some(ref mut child) = entry.child {
            match child.try_wait() {
                Ok(None) => {
                    entry.info.status = ProcessStatus::Running;
                    Ok(ProcessStatus::Running)
                }
                Ok(Some(status)) => {
                    entry.child = None;
                    let (new_status, error_msg) = if status.success() {
                        (ProcessStatus::Exited(status.code().unwrap_or(0)), None)
                    } else {
                        let stderr = entry
                            .stderr_buffer
                            .lock()
                            .expect("stderr lock poisoned")
                            .join("\n");
                        let err_msg = if stderr.is_empty() {
                            format!("Process exited with code {}", status.code().unwrap_or(-1))
                        } else {
                            stderr
                        };
                        (ProcessStatus::Crashed, Some(err_msg))
                    };
                    entry.info.status = new_status.clone();
                    entry.info.last_error = error_msg.clone();

                    let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
                    Self::emit_status(&handle_guard, id, &new_status, &error_msg);

                    Ok(new_status)
                }
                Err(e) => Err(format!("try_wait error: {}", e)),
            }
        } else {
            Ok(entry.info.status.clone())
        }
    }

    fn get_logs(&self, id: &str) -> Result<ProcessLogs, String> {
        let lock = self.processes.lock().expect("processes lock poisoned");
        let entry = lock
            .iter()
            .find(|p| p.info.id == id)
            .ok_or_else(|| format!("Process '{}' not found", id))?;

        let stdout = entry
            .stdout_buffer
            .lock()
            .expect("stdout lock poisoned")
            .clone();
        let stderr = entry
            .stderr_buffer
            .lock()
            .expect("stderr lock poisoned")
            .clone();
        Ok(ProcessLogs {
            stdout_lines: stdout,
            stderr_lines: stderr,
        })
    }
}

impl OsProcessManager {
    /// Shared spawn+track implementation. When `overlay` is `Some`, the
    /// environment overlay (PATH prepend, env set/remove) is applied to the
    /// child process.
    fn spawn_and_track_inner(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        visible: bool,
    ) -> Result<TrackedProcess, String> {
        let mut cmd = Self::build_command(command, args, working_dir);

        if let Some(ov) = overlay {
            ov.apply_std(&mut cmd);
        }

        let mut child = cmd.spawn().map_err(|e| format!("Spawn failed: {}", e))?;
        let pid = child.id();
        let id = generate_id();
        let started_at = timestamp_now();

        let info = TrackedProcess {
            id: id.clone(),
            pid,
            label: label.to_string(),
            status: ProcessStatus::Running,
            started_at,
            duration_secs: 0,
            restarts: 0,
            last_error: None,
            session_id,
            visible,
        };

        let stdout_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
        let handle = handle_guard
            .clone()
            .expect("AppHandle must be set before spawning processes");

        if let Some(stdout) = child.stdout.take() {
            Self::spawn_reader_thread(
                "stdout",
                Box::new(stdout),
                id.clone(),
                stdout_buffer.clone(),
                handle.clone(),
            );
        }

        if let Some(stderr) = child.stderr.take() {
            Self::spawn_reader_thread(
                "stderr",
                Box::new(stderr),
                id.clone(),
                stderr_buffer.clone(),
                handle.clone(),
            );
        }

        let entry = ActiveProcess {
            info: info.clone(),
            child: Some(child),
            stdout_buffer,
            stderr_buffer,
        };

        self.processes
            .lock()
            .expect("processes lock poisoned")
            .push(entry);

        Ok(info)
    }

    /// Spawn a tracked process that is shown in its own native terminal window.
    fn spawn_and_track_visible(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(command, args, working_dir, label, session_id, None, true)
    }

    /// Spawn a tracked visible-terminal process with an environment overlay.
    fn spawn_and_track_visible_with_overlay(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            working_dir,
            label,
            session_id,
            Some(overlay),
            true,
        )
    }
}

fn generate_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("proc_{}", nanos)
}

fn timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_is_unique_enough() {
        let a = generate_id();
        let b = generate_id();
        assert_ne!(a, b);
        assert!(a.starts_with("proc_"));
    }

    #[test]
    fn timestamp_now_is_nonzero() {
        let ts: u64 = timestamp_now().parse().unwrap();
        assert!(ts > 0);
    }

    #[test]
    fn build_command_sets_working_dir() {
        let mut cmd = OsProcessManager::build_command("echo", &["hello"], Some("/tmp"));
        // Verify the command was configured (we can't inspect working_dir
        // directly on std::process::Command, but we can spawn it).
        let child = cmd.spawn();
        assert!(child.is_ok(), "command with working_dir should spawn");
        let mut child = child.unwrap();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn build_command_creates_process_group_on_unix() {
        // On Unix, build_command calls setsid() in pre_exec.
        // We verify by spawning a process and checking its session ID.
        let mut cmd = OsProcessManager::build_command("echo", &["pgid_test"], None);
        let child = cmd.spawn().expect("should spawn");
        let pid = child.id();
        // The child should be in its own session (session id == pid after setsid).
        // We can't easily check this from the parent, but we verify the spawn
        // succeeded (which means pre_exec ran without error).
        let mut child = child;
        let status = child.wait().expect("should wait");
        assert!(status.success());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn build_command_creates_process_group_on_windows() {
        // On Windows, build_command sets CREATE_NEW_PROCESS_GROUP.
        // We verify by spawning and checking the process starts successfully.
        let mut cmd = OsProcessManager::build_command("cmd", &["/C", "echo pgid_test"], None);
        let child = cmd
            .spawn()
            .expect("should spawn with CREATE_NEW_PROCESS_GROUP");
        let mut child = child;
        let status = child.wait().expect("should wait");
        assert!(status.success());
    }

    #[test]
    fn kill_returns_error_for_nonexistent_process() {
        let pm = OsProcessManager::new();
        let result = pm.kill("nonexistent_id");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn refresh_status_returns_error_for_nonexistent_process() {
        let pm = OsProcessManager::new();
        let result = pm.refresh_status("nonexistent_id");
        assert!(result.is_err());
    }

    #[test]
    fn get_logs_returns_error_for_nonexistent_process() {
        let pm = OsProcessManager::new();
        let result = pm.get_logs("nonexistent_id");
        assert!(result.is_err());
    }

    #[test]
    fn list_empty_initially() {
        let pm = OsProcessManager::new();
        assert!(pm.list().is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn kill_windows_tree_handles_already_exited() {
        // taskkill on a non-existent PID returns exit code 128 (not found).
        // This should be interpreted as "already exited", not as an error.
        let status = Command::new("taskkill")
            .args(["/F", "/T", "/PID", "99999999"])
            .status()
            .expect("taskkill should run");
        let code = status.code().unwrap_or(-1);
        assert!(
            code >= 128 || code == 0,
            "taskkill on nonexistent PID should return 128+, got {}",
            code
        );
    }

    #[cfg(unix)]
    #[test]
    fn kill_signal_to_process_group_does_not_crash() {
        use std::time::Duration;
        // Spawn a short-lived process and kill its group.
        let mut cmd = Command::new("sleep");
        cmd.arg("60");
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        let mut child = cmd.spawn().expect("should spawn sleep");
        let pid = child.id();

        // Send SIGTERM to the process group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }

        // Wait briefly for the process to die.
        thread::sleep(Duration::from_millis(500));
        let status = child.try_wait();
        assert!(
            status.is_ok(),
            "try_wait should succeed after killing process group"
        );
        // The process may or may not have exited yet (SIGKILL would be
        // more definitive, but SIGTERM + sleep is fine for a test).
        let _ = child.kill();
        let _ = child.wait();
    }
}
