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
    /// Bounded log buffer for V2 enhanced output tracking.
    log_buffer: Arc<crate::modules::workspace::models::BoundedLogBuffer>,
    /// Sequence counters for output events.
    stdout_seq: Arc<std::sync::atomic::AtomicU64>,
    stderr_seq: Arc<std::sync::atomic::AtomicU64>,
    /// Run/step ownership for output events.
    run_id: Option<String>,
    step_id: Option<String>,
    /// Reader thread join handles for cleanup.
    _reader_handles: Vec<thread::JoinHandle<()>>,
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

    /// Spawn a process with ownership metadata for V2 run/step tracking.
    fn spawn_and_track_owned(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String>;

    /// Spawn a visible process with ownership metadata.
    fn spawn_visible_owned(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String>;

    /// Spawn a visible process with ownership metadata and an optional
    /// startup-probe marker. When `startup_marker` is set, the command run
    /// inside the terminal writes its exit code to that file after it exits,
    /// so the orchestrator can verify the command actually started (and did
    /// not fail instantly) instead of trusting that a terminal window opened.
    ///
    /// The default implementation ignores the marker and delegates to
    /// [`Self::spawn_visible_owned`] so managers that do not implement it
    /// degrade gracefully.
    fn spawn_visible_owned_with_startup_marker(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        run_id: Option<String>,
        step_id: Option<String>,
        _startup_marker: Option<&str>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_visible_owned(
            command,
            args,
            working_dir,
            label,
            session_id,
            overlay,
            run_id,
            step_id,
        )
    }

    /// Spawn a tracked process with ownership metadata and an environment
    /// overlay applied to the child (PATH prepend, env set/remove).
    ///
    /// This is the overlay-capable counterpart of [`Self::spawn_and_track_owned`].
    /// The default implementation reports the overlay as unsupported so
    /// managers that do not implement it degrade gracefully.
    fn spawn_and_track_owned_with_overlay(
        &self,
        _command: &str,
        _args: &[&str],
        _working_dir: Option<&str>,
        _label: &str,
        _session_id: Option<String>,
        _overlay: &EnvironmentOverlay,
        _run_id: Option<String>,
        _step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        Err(
            "Owned spawn with environment overlay is not supported by this process manager"
                .to_string(),
        )
    }

    /// Spawn a tracked process whose LAST argument is appended to the
    /// command line verbatim (never re-quoted).
    ///
    /// Windows `cmd /C` / `cmd /K` parse their argument from the RAW
    /// command line; backslash-escaped quotes from standard argument
    /// quoting (`\"`) break commands that contain quoted paths. Callers
    /// pass the cmd-style-quoted tail separately (e.g. a script for
    /// `cmd /C`), and the implementation appends it raw.
    ///
    /// The default implementation degrades to a regular owned spawn
    /// (the tail appended as a normal argument) for managers that do not
    /// implement the raw distinction.
    fn spawn_and_track_owned_with_raw_tail(
        &self,
        command: &str,
        args: &[&str],
        raw_tail: Option<&str>,
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        let mut all: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        if let Some(raw) = raw_tail {
            all.push(raw.to_string());
        }
        let refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        self.spawn_and_track_owned(
            command,
            &refs,
            working_dir,
            label,
            session_id,
            run_id,
            step_id,
        )
    }

    fn list(&self) -> Vec<TrackedProcess>;
    fn kill(&self, id: &str) -> Result<(), String>;
    fn refresh_status(&self, id: &str) -> Result<ProcessStatus, String>;
    fn get_logs(&self, id: &str) -> Result<ProcessLogs, String>;

    /// Get the bounded log buffer for a process (V2).
    fn get_log_buffer(
        &self,
        id: &str,
    ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>>;
    /// Get truncation metadata for a process's logs.
    fn get_log_truncation(&self, id: &str) -> Option<(LogTruncation, LogTruncation)>;
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
        // Use try_lock to avoid blocking; if poisoned, replace anyway
        match self.app_handle.lock() {
            Ok(mut h) => *h = Some(handle),
            Err(mut e) => {
                **e.get_mut() = Some(handle);
            }
        }
    }

    fn spawn_reader_thread(
        stream_name: &'static str,
        reader: Box<dyn std::io::Read + Send + 'static>,
        process_id: String,
        buffer: Arc<Mutex<Vec<String>>>,
        log_buffer: Arc<BoundedLogBuffer>,
        seq: Arc<std::sync::atomic::AtomicU64>,
        handle: Option<tauri::AppHandle>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let buf_reader = BufReader::new(reader);
            for line in buf_reader.lines() {
                match line {
                    Ok(text) => {
                        let sequence = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                        // Push to unbounded legacy buffer
                        if let Ok(mut buf) = buffer.lock() {
                            buf.push(text.clone());
                        }

                        // Push to bounded buffer
                        if stream_name == "stderr" {
                            log_buffer.push_stderr(text.clone());
                        } else {
                            log_buffer.push_stdout(text.clone());
                        }

                        // Emit process-output event with enhanced metadata
                        if let Some(ref h) = handle {
                            let _ = h.emit(
                                PROCESS_EVENT_OUTPUT,
                                ProcessOutputEvent {
                                    process_id: process_id.clone(),
                                    stream: stream_name.to_string(),
                                    line: text,
                                    sequence: Some(sequence),
                                    timestamp: Some(default_now_iso()),
                                    run_id: run_id.clone(),
                                    step_id: step_id.clone(),
                                },
                            );
                        }
                    }
                    Err(_) => break,
                }
            }
        })
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
        match current_os() {
            HostOs::Windows => {
                use std::os::windows::process::CommandExt;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                // CREATE_NO_WINDOW: these are captured (hidden) processes —
                // without it every one would flash a console window in
                // release builds, which have no console to inherit.
                cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | crate::platform::CREATE_NO_WINDOW);
            }
            HostOs::Linux | HostOs::Macos => {
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
    fn build_detached_command(command: &str, args: &[&str], working_dir: Option<&str>) -> Command {
        let plan = crate::platform::command::resolve_spawn_plan(command);
        // Batch shims run through `cmd /C <cmd-style-quoted line>`. The
        // line is appended RAW: standard argument quoting would escape its
        // quotes with backslashes, which cmd's /C parsing breaks.
        let (spawn_program, spawn_args, raw_last): (String, Vec<String>, Option<String>) =
            if plan.batch_shim {
                let line = crate::platform::command::batch_shim_cmd_line(&plan.program, args);
                (
                    "cmd".to_string(),
                    vec![line[0].clone()],
                    Some(line[1].clone()),
                )
            } else {
                (
                    plan.program,
                    args.iter().map(|s| s.to_string()).collect(),
                    None,
                )
            };
        let spawn_args_refs: Vec<&str> = spawn_args.iter().map(|s| s.as_str()).collect();

        let mut cmd = Command::new(spawn_program);
        cmd.args(spawn_args_refs);
        if let Some(raw) = raw_last {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.raw_arg(&raw);
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = raw;
            }
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
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
    fn kill_process_tree(child: &mut Child, pid: u32) -> Result<bool, String> {
        match current_os() {
            HostOs::Windows => kill_windows_tree(pid),
            HostOs::Linux | HostOs::Macos => kill_unix_group(child, pid),
        }
    }

    /// Emit a process status event. Uses optional AppHandle — no panics.
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
/// The launch is delegated to `platform::terminal::resolve_terminal_plan`,
/// which picks the platform's best terminal (Windows Terminal → cmd on
/// Windows, Terminal.app on macOS, xterm/alacritty/... on Linux) and builds
/// the exact command line. The terminal process is tracked as a
/// `TerminalWrapper` so stopping it stops the process tree.
fn spawn_in_terminal(
    manager: &OsProcessManager,
    command: &str,
    args: &[&str],
    working_dir: Option<&str>,
    label: &str,
    session_id: Option<String>,
    overlay: Option<&EnvironmentOverlay>,
    run_id: Option<String>,
    step_id: Option<String>,
    startup_marker: Option<&str>,
) -> Result<TrackedProcess, String> {
    use crate::platform::terminal::{
        resolve_terminal_plan, TerminalBackend, TerminalConfig, TerminalWindowPolicy,
    };

    // The inner command executed inside the terminal window.
    let inner = join_command(command, args);

    let config = TerminalConfig {
        backend: TerminalBackend::Default,
        command: inner,
        working_dir: working_dir.map(String::from),
        window_policy: TerminalWindowPolicy::NewWindow,
        label: Some(label.to_string()),
        keep_open: true,
        env: None,
        startup_marker: startup_marker.map(String::from),
    };

    let plan = resolve_terminal_plan(&config)
        .map_err(|e| format!("Failed to resolve terminal plan: {}", e))?;

    // The terminal plan's LAST argument is the `cmd /K` / `wt` payload. It
    // must reach the spawned process VERBATIM: standard argument quoting
    // would backslash-escape its quotes (`\"`), which cmd's /K parsing does
    // not understand — quoted paths would be split and the command would die
    // with "C:\Program is not recognized". The payload is therefore popped
    // from the arg list and appended raw (unquoted): cmd parses everything
    // after /K itself, and wt forwards the tail to cmd unchanged.
    // (The plan's `raw_tail` field — the payload wrapped in cmd-style quotes
    // `"<cmd>"` — is NOT what the spawner appends: cmd's first/last-quote
    // stripping mangles the nested quotes and the whole command fails.)
    let (plan_args, raw_tail) = if plan.raw_tail.is_some() {
        let mut args = plan.args.clone();
        let raw = args.pop();
        (args, raw)
    } else {
        (plan.args.clone(), None)
    };
    let plan_args: Vec<&str> = plan_args.iter().map(|s| s.as_str()).collect();
    manager.spawn_and_track_visible_inner(
        &plan.program,
        &plan_args,
        raw_tail.as_deref(),
        // The working directory reaches the terminal process out-of-band:
        // `cmd` inherits it as the spawn's current directory, and `wt`
        // receives it via `--startingDirectory` (baked-in `cd /d` prefixes
        // break under wt's command-line re-parsing — see terminal.rs).
        working_dir,
        label,
        session_id,
        overlay,
        run_id,
        step_id,
    )
}

/// Join a program + args into a single command line, quoting each token
/// for the target shell.
fn join_command(command: &str, args: &[&str]) -> String {
    let mut parts: Vec<String> = vec![quote_for_shell(command)];
    for a in args {
        parts.push(quote_for_shell(a));
    }
    parts.join(" ")
}

#[cfg(test)]
fn batch_escape(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut in_quotes = false;
    for ch in cmd.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                out.push(ch);
            }
            '%' => {
                out.push_str("%%");
            }
            '&' | '|' | '<' | '>' | '^' if !in_quotes => {
                out.push('^');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

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

fn kill_windows_tree(pid: u32) -> Result<bool, String> {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console(&mut cmd);
    let status = cmd.status();

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(s) => {
            let code = s.code().unwrap_or(-1);
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

#[cfg(unix)]
fn kill_unix_group(child: &mut Child, pid: u32) -> Result<bool, String> {
    use std::os::unix::process::ExitStatusExt;
    use std::time::Duration;

    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.wait();
            return Ok(false);
        }
        Ok(None) => {}
        Err(e) => {
            return Err(format!("Failed to check process status: {}", e));
        }
    }

    let pgid = pid as i32;

    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }

    let grace = Duration::from_millis(UNIX_KILL_GRACE_MS);
    let deadline = SystemTime::now() + grace;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
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

    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }

    thread::sleep(Duration::from_millis(200));
    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.wait();
            Ok(true)
        }
        Ok(None) => {
            let _ = child.wait();
            Ok(true)
        }
        Err(e) => Err(format!(
            "Failed to check process status after SIGKILL: {}",
            e
        )),
    }
}

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
        self.spawn_and_track_inner(
            command,
            args,
            None,
            working_dir,
            label,
            session_id,
            None,
            false,
            None,
            None,
        )
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
            None,
            working_dir,
            label,
            session_id,
            Some(overlay),
            false,
            None,
            None,
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
        spawn_in_terminal(
            self,
            command,
            args,
            working_dir,
            label,
            session_id,
            overlay,
            None,
            None,
            None,
        )
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

    fn spawn_and_track_owned(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            None,
            working_dir,
            label,
            session_id,
            None,
            false,
            run_id,
            step_id,
        )
    }

    fn spawn_visible_owned(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        spawn_in_terminal(
            self,
            command,
            args,
            working_dir,
            label,
            session_id,
            overlay,
            run_id,
            step_id,
            None,
        )
    }

    fn spawn_visible_owned_with_startup_marker(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        run_id: Option<String>,
        step_id: Option<String>,
        startup_marker: Option<&str>,
    ) -> Result<TrackedProcess, String> {
        spawn_in_terminal(
            self,
            command,
            args,
            working_dir,
            label,
            session_id,
            overlay,
            run_id,
            step_id,
            startup_marker,
        )
    }

    fn spawn_and_track_owned_with_overlay(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: &EnvironmentOverlay,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            None,
            working_dir,
            label,
            session_id,
            Some(overlay),
            false,
            run_id,
            step_id,
        )
    }

    fn spawn_and_track_owned_with_raw_tail(
        &self,
        command: &str,
        args: &[&str],
        raw_tail: Option<&str>,
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            raw_tail,
            working_dir,
            label,
            session_id,
            None,
            false,
            run_id,
            step_id,
        )
    }

    fn list(&self) -> Vec<TrackedProcess> {
        let mut lock = self.processes.lock().expect("processes lock poisoned");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let handle_guard = self.app_handle.lock().expect("app_handle lock poisoned");
        let handle = handle_guard.clone();
        for entry in lock.iter_mut() {
            let mut finished: Option<(ProcessStatus, Option<String>)> = None;
            if let Some(ref mut child) = entry.child {
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        entry.child = None;
                        let code = status.code().unwrap_or(-1);
                        let (new_status, error_msg) = if status.success() {
                            (ProcessStatus::Exited(code), None)
                        } else {
                            let stderr = entry
                                .stderr_buffer
                                .lock()
                                .expect("stderr lock poisoned")
                                .join("\n");
                            let err_msg = if stderr.is_empty() {
                                format!("Process exited with code {}", code)
                            } else {
                                stderr
                            };
                            (ProcessStatus::ExitedWithError(code), Some(err_msg))
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
        if entry.child.is_none() {
            match &entry.info.status {
                ProcessStatus::Exited(_)
                | ProcessStatus::ExitedWithError(_)
                | ProcessStatus::Crashed => {
                    return Ok(());
                }
                ProcessStatus::Killed => {
                    return Ok(());
                }
                ProcessStatus::Running => {
                    entry.info.status = ProcessStatus::Exited(0);
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }

        let pid = entry.info.pid;

        let kill_result = if let Some(ref mut child) = entry.child {
            Self::kill_process_tree(child, pid)
        } else {
            Ok(false)
        };

        if let Some(ref mut child) = entry.child {
            let _ = child.wait();
        }
        entry.child = None;

        match kill_result {
            Ok(was_running) => {
                if was_running {
                    entry.info.status = ProcessStatus::Killed;
                }

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
                    let code = status.code().unwrap_or(-1);
                    let (new_status, error_msg) = if status.success() {
                        (ProcessStatus::Exited(code), None)
                    } else {
                        let stderr = entry
                            .stderr_buffer
                            .lock()
                            .expect("stderr lock poisoned")
                            .join("\n");
                        let err_msg = if stderr.is_empty() {
                            format!("Process exited with code {}", code)
                        } else {
                            stderr
                        };
                        (ProcessStatus::ExitedWithError(code), Some(err_msg))
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

    fn get_log_buffer(
        &self,
        id: &str,
    ) -> Option<Arc<crate::modules::workspace::models::BoundedLogBuffer>> {
        self.processes.lock().ok().and_then(|procs| {
            procs
                .iter()
                .find(|p| p.info.id == id)
                .map(|p| p.log_buffer.clone())
        })
    }

    fn get_log_truncation(&self, id: &str) -> Option<(LogTruncation, LogTruncation)> {
        self.processes.lock().ok().and_then(|procs| {
            procs.iter().find(|p| p.info.id == id).map(|p| {
                (
                    p.log_buffer.stdout_truncation(),
                    p.log_buffer.stderr_truncation(),
                )
            })
        })
    }
}

impl OsProcessManager {
    /// Shared spawn+track implementation with ownership metadata.
    fn spawn_and_track_inner(
        &self,
        command: &str,
        args: &[&str],
        raw_tail: Option<&str>,
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        visible: bool,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        let plan = crate::platform::command::resolve_spawn_plan(command);
        // Batch shims must go through `cmd /C`. The command line built by
        // `batch_shim_cmd_line` is already cmd-style quoted; passing it
        // through argument quoting would re-escape the quotes (`\"`), which
        // cmd's /C parsing does not understand. It is appended RAW instead.
        let (spawn_program, spawn_args, raw_last): (String, Vec<String>, Option<String>) =
            if plan.batch_shim {
                let line = crate::platform::command::batch_shim_cmd_line(&plan.program, args);
                (
                    "cmd".to_string(),
                    vec![line[0].clone()],
                    Some(line[1].clone()),
                )
            } else {
                (
                    plan.program,
                    args.iter().map(|s| s.to_string()).collect(),
                    raw_tail.map(String::from),
                )
            };
        let mut spawn_args = spawn_args;
        let mut raw_last = raw_last;
        let spawn_args_refs: Vec<&str> = spawn_args.iter().map(|s| s.as_str()).collect();

        let mut cmd = Self::build_command(&spawn_program, &spawn_args_refs, working_dir);

        if let Some(raw) = raw_last.take() {
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.raw_arg(&raw);
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = raw;
            }
        }

        if let Some(ov) = overlay {
            ov.apply_std(&mut cmd);
        }

        let mut child = cmd.spawn().map_err(|e| format!("Spawn failed: {}", e))?;
        let pid = child.id();
        let id = generate_process_id();
        let started_at = timestamp_now();

        let command_line = join_command(command, args);
        let tracking_quality = if visible {
            Some(ProcessTrackingQuality::TerminalWrapper)
        } else {
            Some(ProcessTrackingQuality::Exact)
        };

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
            run_id: run_id.clone(),
            step_id: step_id.clone(),
            command: Some(command_line),
            working_dir: working_dir.map(String::from),
            tracking_quality,
        };

        let stdout_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log_buffer = Arc::new(BoundedLogBuffer::new(10000, 10 * 1024 * 1024));
        let stdout_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stderr_seq = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Get app handle — optional, no panic
        let handle = self.app_handle.lock().ok().and_then(|h| h.clone());

        let mut reader_handles = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            let h = Self::spawn_reader_thread(
                "stdout",
                Box::new(stdout),
                id.clone(),
                stdout_buffer.clone(),
                log_buffer.clone(),
                stdout_seq.clone(),
                handle.clone(),
                run_id.clone(),
                step_id.clone(),
            );
            reader_handles.push(h);
        }

        if let Some(stderr) = child.stderr.take() {
            let h = Self::spawn_reader_thread(
                "stderr",
                Box::new(stderr),
                id.clone(),
                stderr_buffer.clone(),
                log_buffer.clone(),
                stderr_seq.clone(),
                handle.clone(),
                run_id.clone(),
                step_id.clone(),
            );
            reader_handles.push(h);
        }

        let entry = ActiveProcess {
            info: info.clone(),
            child: Some(child),
            stdout_buffer,
            stderr_buffer,
            log_buffer,
            stdout_seq,
            stderr_seq,
            run_id,
            step_id,
            _reader_handles: reader_handles,
        };

        self.processes
            .lock()
            .expect("processes lock poisoned")
            .push(entry);

        Ok(info)
    }

    /// Spawn a tracked visible-terminal process.
    fn spawn_and_track_visible(
        &self,
        command: &str,
        args: &[&str],
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            None,
            working_dir,
            label,
            session_id,
            None,
            true,
            None,
            None,
        )
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
            None,
            working_dir,
            label,
            session_id,
            Some(overlay),
            true,
            None,
            None,
        )
    }

    /// Spawn with ownership metadata and optional overlay.
    fn spawn_and_track_visible_inner(
        &self,
        command: &str,
        args: &[&str],
        raw_tail: Option<&str>,
        working_dir: Option<&str>,
        label: &str,
        session_id: Option<String>,
        overlay: Option<&EnvironmentOverlay>,
        run_id: Option<String>,
        step_id: Option<String>,
    ) -> Result<TrackedProcess, String> {
        self.spawn_and_track_inner(
            command,
            args,
            raw_tail,
            working_dir,
            label,
            session_id,
            overlay,
            true,
            run_id,
            step_id,
        )
    }
}

/// Generate a collision-safe process ID using UUID v4.
fn generate_process_id() -> String {
    format!("proc_{}", uuid::Uuid::new_v4())
}

fn timestamp_now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs().to_string()
}

fn default_now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_is_unique_enough() {
        let a = generate_process_id();
        let b = generate_process_id();
        assert_ne!(a, b);
        assert!(a.starts_with("proc_"));
    }

    #[test]
    fn generate_id_is_uuid_format() {
        let id = generate_process_id();
        let uuid_part = id.strip_prefix("proc_").unwrap();
        // UUID v4 format: 8-4-4-4-12
        assert_eq!(uuid_part.len(), 36);
        assert!(uuid_part.chars().filter(|c| *c == '-').count() == 4);
    }

    #[test]
    fn timestamp_now_is_nonzero() {
        let ts: u64 = timestamp_now().parse().unwrap();
        assert!(ts > 0);
    }

    #[test]
    fn batch_escape_protects_metacharacters() {
        assert_eq!(
            batch_escape(r#""C:\my dir\npm run dev""#),
            r#""C:\my dir\npm run dev""#
        );
        assert_eq!(batch_escape("a & b | c"), "a ^& b ^| c");
        assert_eq!(batch_escape("echo 100%"), "echo 100%%");
        assert_eq!(batch_escape("a<b>c"), "a^<b^>c");
    }

    #[test]
    fn build_command_sets_working_dir() {
        let wd = std::env::temp_dir();
        let wd = wd.to_str().expect("temp dir is valid UTF-8");
        // `echo` не является исполняемым файлом на Windows (это встроенная
        // команда cmd), поэтому там используем `cmd /C echo`.
        #[cfg(target_os = "windows")]
        let (cmd_name, cmd_args): (&str, &[&str]) = ("cmd", &["/C", "echo hello"]);
        #[cfg(not(target_os = "windows"))]
        let (cmd_name, cmd_args): (&str, &[&str]) = ("echo", &["hello"]);
        let mut cmd = OsProcessManager::build_command(cmd_name, cmd_args, Some(wd));
        let child = cmd.spawn();
        assert!(child.is_ok(), "command with working_dir should spawn");
        let mut child = child.unwrap();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn build_command_creates_process_group_on_unix() {
        let mut cmd = OsProcessManager::build_command("echo", &["pgid_test"], None);
        let child = cmd.spawn().expect("should spawn");
        let mut child = child;
        let status = child.wait().expect("should wait");
        assert!(status.success());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn build_command_creates_process_group_on_windows() {
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

        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }

        thread::sleep(Duration::from_millis(500));
        let status = child.try_wait();
        assert!(
            status.is_ok(),
            "try_wait should succeed after killing process group"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn process_status_serialization_roundtrip() {
        let statuses = vec![
            ProcessStatus::Starting,
            ProcessStatus::Running,
            ProcessStatus::Ready,
            ProcessStatus::Exited(0),
            ProcessStatus::Exited(1),
            ProcessStatus::ExitedWithError(1),
            ProcessStatus::Crashed,
            ProcessStatus::Killed,
            ProcessStatus::TimedOut,
            ProcessStatus::Cancelled,
            ProcessStatus::ExternalLaunchAccepted,
            ProcessStatus::Unknown,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let back: ProcessStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, back, "Roundtrip failed for {:?}", status);
        }
    }

    #[test]
    fn process_output_event_has_optional_fields() {
        let event = ProcessOutputEvent {
            process_id: "p1".into(),
            stream: "stdout".into(),
            line: "hello".into(),
            sequence: Some(42),
            timestamp: Some("2026-01-01T00:00:00Z".into()),
            run_id: Some("r1".into()),
            step_id: Some("s1".into()),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("r1"));
        assert!(json.contains("s1"));
        assert!(json.contains("42"));
    }

    #[test]
    fn bounded_log_buffer_available_on_process() {
        let pm = OsProcessManager::new();
        assert!(pm.get_log_buffer("nonexistent").is_none());
        assert!(pm.get_log_truncation("nonexistent").is_none());
    }
}
