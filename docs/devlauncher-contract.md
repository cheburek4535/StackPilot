# Contract: DevLauncher V2 — Graph-based project environment orchestration

Status: **authoritative** — created before implementation. Implementation must conform to this file.

---

## A. Architectural invariants

1. **Profile** — a persisted user scenario. A profile is a JSON file on disk containing the project path, environment binding, IDE preference, and a list of steps (or legacy actions). Profiles are named by their `name` field and stored under `<app_data>/profiles/<name>.json`.

2. **Run** — one execution instance of a profile. Each `run_profile` invocation creates a `LaunchRun` with a unique `run_id`. The same profile may be run multiple times concurrently (e.g. two terminals open). A run owns its own `StepExecutionState` instances.

3. **Step** — a node in a dependency graph. Each step has an explicit `StepKind` (the resource type it manages), a `CommandSpec`, and optional `depends_on: Vec<String>` referencing other step IDs. Steps without dependencies may execute in parallel. Steps with dependencies wait until all predecessors reach a terminal state.

4. **Resource types** — Steps represent at least one of these distinct resource types:
   - **Process (one-shot)**: command runs, output captured, process exits → step completes.
   - **Process (long-running service)**: command runs indefinitely, step completes only on explicit user stop or crash.
   - **Native terminal**: process runs inside a user-visible terminal window (cmd on Windows, Terminal.app on macOS, xterm/alacritty on Linux).
   - **External application**: detached GUI launch (IDE, browser, Docker Desktop) — no output tracking.
   - **URL**: open a URL in the system browser.
   - **Readiness check**: WaitForUrl or WaitForPort — polls until success or timeout.
   - **Delay**: sleep for a fixed duration.
   - **Script**: arbitrary shell script, monitored until exit.

5. **Step completion is not always process exit**. A `WaitForPort` step completes when the port opens, not when a process exits. A `Delay` step completes when the timer expires. A `URL` step completes immediately after the browser open call.

6. **Native terminal PID ≠ exact process tracking**. When a step runs in a native terminal, the tracked PID is the terminal process (cmd.exe on Windows, osascript/terminal emulator on Unix). The inner command's PID is not directly tracked. Output goes to the terminal window, not the app's log viewer. The `ProcessTrackingQuality` field on `ManagedProcess` records whether the PID is the actual command or the terminal wrapper.

7. **Independent steps may execute in parallel**. The orchestrator resolves the dependency graph and starts all steps with zero pending dependencies simultaneously. Each step runs on its own thread/task.

8. **Single step failure is interpreted through an explicit `FailurePolicy`**. The step's failure does not automatically abort the run. The policy determines whether dependent steps are skipped, the run stops, or execution continues with partial success.

9. **Cancellation**. A run can be cancelled by the user. Cancellation sends SIGTERM (Unix) or taskkill /F /T (Windows) to all running process-tree steps. Wait steps are interrupted via a cancellation token. URL and application steps are already finished. Cancellation is a cooperative operation with a grace period.

---

## B. Compatibility policy

### Legacy models — DO NOT REMOVE

| Legacy type | Location | Fate |
|---|---|---|
| `LaunchProfile` | `devlauncher/models.rs:114` | **Compatibility wrapper**. All existing Tauri commands accept and return this type. V2 profiles are internally stored as `LaunchProfileV2` but transparently round-tripped to/from `LaunchProfile` via a `From`/`Into` impl. The `actions` field is populated from the step graph's topological order. |
| `LaunchAction` | `devlauncher/models.rs:62` | **Legacy schema reader**. Still deserialized from old JSON files. New profiles store `LaunchStep[]`. When a legacy `LaunchProfile` is saved through `save_profile`, it is converted to `LaunchProfileV2` internally. |
| `ActionType` | `devlauncher/models.rs:3` | **Deprecated API**. Mapped 1:1 to `StepKind` via conversion. Existing code that pattern-matches on `ActionType` continues to work. New code uses `StepKind`. |
| `ActionStatus` | `devlauncher/models.rs:131` | **Retained**. Still returned from `execute_action` for backward compat. V2 run events carry `StepExecutionState` instead. |
| `run_profile` command | `devlauncher/commands.rs:223` | **Retained, re-implemented**. Still returns `Vec<(String, ActionStatus)>`. Internally delegates to the V2 orchestrator. The result is derived from `StepExecutionState` entries for backward compat. |
| `execute_action` command | `devlauncher/commands.rs:127` | **Retained**. Executes a single legacy action. Internally wraps it as a one-step run. |
| `spawn_process` / `spawn_process_visible` / `launch_application_detached` | `workspace/commands.rs:15,62,85` | **Retained**. These are raw workspace process commands, independent of DevLauncher. No changes. |
| `process-output` event | `workspace/process_manager.rs:12` | **Retained**. Still emitted per-line for captured processes. V2 adds `run-output` events but `process-output` continues for individual process consumers. |
| `process-status` event | `workspace/process_manager.rs:13` | **Retained**. Still emitted on process state changes. V2 adds `step-status-changed` and `run-status-changed` but `process-status` continues. |

### Backward compatibility rules for JSON profiles

- Old JSON files containing `actions: [...]` (flat array) are read as-is into `LaunchProfile`.
- When a legacy profile is accessed via V2 APIs, it is converted: each `LaunchAction` becomes a `LaunchStep` with `depends_on: []` (sequential, same order as the array).
- When a V2 profile is saved, it is stored in V2 format. When read by legacy `list_profiles`, it is converted back to `LaunchProfile` with actions in topological order.
- `serde(default)` on all new fields ensures old files deserialize without errors.

---

## C. Target domain model

All types are `#[derive(Debug, Clone, Serialize, Deserialize)]`. Enum representations use stable string-compatible serde defaults (externally tagged).

### LaunchProfileV2

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfileV2 {
    /// Schema version marker. Always "2" for V2 profiles.
    #[serde(default = "default_version")]
    pub schema_version: String,  // "2"
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub steps: Vec<LaunchStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_ide: Option<PreferredIde>,
}
```

### LaunchStep

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchStep {
    pub id: String,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_policy: Option<FailurePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<RetryPolicy>,
}
```

### StepKind

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepKind {
    RunCommand {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visibility: Option<Visibility>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_mode: Option<ExecutionMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_policy: Option<CompletionPolicy>,
    },
    OpenApplication {
        path: String,
        #[serde(default)]
        args: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args_list: Option<Vec<String>>,
    },
    OpenUrl {
        url: String,
    },
    WaitForUrl {
        url: String,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    WaitForPort {
        host: String,
        port: u16,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    Delay {
        #[serde(default = "default_delay")]
        seconds: u64,
    },
    ExecuteScript {
        script: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
    },
}
```

### CommandSpec (for future structured command representation)

```rust
/// Structured command specification — alternative to a flat string.
/// NOT the primary representation in V2 (string commands remain the norm),
/// but available for tooling that needs to reason about arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}
```

### Visibility

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Captured output, shown in the app's log viewer.
    Captured,
    /// Runs in a user-visible native terminal window.
    VisibleTerminal,
    /// Fire-and-forget; output is discarded.
    Detached,
}
```

Default for `RunCommand` when `visibility` is absent: `Captured`.

### ExecutionMode

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Process must exit for the step to complete.
    OneShot,
    /// Process runs indefinitely; step completes only on user stop or crash.
    LongRunning,
}
```

Default for `RunCommand` when `execution_mode` is absent: `OneShot`.

### CompletionPolicy

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompletionPolicy {
    /// Step completes when the process exits (default for OneShot).
    ProcessExit,
    /// Step completes only when user manually stops it.
    Manual,
    /// Step completes after a readiness check succeeds.
    ReadinessCheck {
        check: readiness::ReadinessSpec,
    },
}
```

### FailurePolicy

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Stop the entire run. Only when a profile opts in explicitly.
    StopRun,
    /// Continue executing independent steps but skip dependents.
    SkipDependents,
    /// Continue everything; record as warning only.
    WarnAndContinue,
}
```

Default: `SkipDependents` for steps with dependents, `WarnAndContinue` for leaf
steps. A failed step must only ever skip its own downstream chain — a hard
`StopRun` abort (which kills every in-flight process, including a `docker
compose up` that is still starting containers) is reserved for profiles that
set it explicitly.

### RetryPolicy

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    #[serde(default = "default_retry_delay")]
    pub delay_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_multiplier: Option<f64>,
}
```

### LaunchRun

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchRun {
    pub run_id: String,
    pub profile_name: String,
    pub status: RunStatus,
    pub steps: Vec<StepExecutionState>,
    pub created_at: String, // ISO 8601 or epoch seconds
    pub finished_at: Option<String>,
}
```

### RunStatus

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    PartialSuccess,
}
```

### StepExecutionState

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecutionState {
    pub step_id: String,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries_remaining: Option<u32>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
    Retrying,
}
```

### ManagedProcess

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedProcess {
    pub id: String,
    pub pid: u32,
    pub label: String,
    pub status: ProcessStatus,
    pub started_at: String,
    pub duration_secs: u64,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_quality: Option<ProcessTrackingQuality>,
}
```

### ProcessTrackingQuality

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTrackingQuality {
    /// PID points to the actual command process.
    Exact,
    /// PID points to a terminal wrapper (cmd.exe, osascript, xterm).
    TerminalWrapper,
    /// PID was the process but it has since been reaped / replaced.
    Approximate,
    /// Process was launched detached; no PID tracking.
    Detached,
}
```

### Diagnostic

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub run_id: String,
    pub step_id: Option<String>,
    pub source: LogSource,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    Process,
    Orchestrator,
    Preflight,
    Readiness,
    User,
}
```

---

## D. Event contract

All events are emitted via `tauri::AppHandle::emit()`. Event names are stable strings. Payloads are JSON-serializable structs owned by the backend. The frontend subscribes but never dictates payload shape.

### run-created

```
Event name: "devlauncher:run-created"
Payload: LaunchRun
```

Emitted when a new run is created (before any step starts). Contains the full `LaunchRun` with all steps in `Pending` status.

### run-status-changed

```
Event name: "devlauncher:run-status-changed"
Payload: { run_id: String, status: RunStatus }
```

Emitted when the overall run status changes (Running → Succeeded, Running → Failed, Running → Cancelled, etc.).

### step-status-changed

```
Event name: "devlauncher:step-status-changed"
Payload: { run_id: String, step: StepExecutionState }
```

Emitted whenever a step transitions to a new status (Pending → Running, Running → Succeeded, Running → Failed, etc.).

### process-started

```
Event name: "devlauncher:process-started"
Payload: { run_id: String, step_id: String, process: ManagedProcess }
```

Emitted when a step spawns a tracked process. Carries the initial `ManagedProcess` snapshot.

### process-output

```
Event name: "process-output"
Payload: ProcessOutputEvent { process_id, stream, line }
```

**Retained from legacy**. Emitted per-line for captured processes. V2 does not change this event.

### process-status

```
Event name: "process-status"
Payload: ProcessStatusEvent { process_id, status, error }
```

**Retained from legacy**. Emitted on process state changes.

### diagnostic

```
Event name: "devlauncher:diagnostic"
Payload: Diagnostic
```

Emitted for preflight failures (Docker daemon not running), readiness timeouts, orchestrator warnings, and other diagnostic messages.

### run-finished

```
Event name: "devlauncher:run-finished"
Payload: LaunchRun
```

Emitted when a run reaches a terminal status (Succeeded, Failed, Cancelled, PartialSuccess). Contains the full `LaunchRun` snapshot with final step states.

---

## E. Path rules

### Profile project root

- `LaunchProfile.project_path` is the absolute path to the project root directory.
- When `project_path` is `None`, the workspace's current project (`WorkspaceState.project.get_current()`) is used as fallback.
- Path resolution occurs in `resolve_working_dir()` before any step execution.

### Relative working directories

- `working_dir: "./backend"` is resolved relative to the **profile's project root** (not the app's working directory).
- Resolution: `PathBuf::from(project_root).join("./backend")`.
- If `project_path` is also `None` and no workspace project is set, the relative directory is used as-is (likely fails with "No such file or directory" — this is logged as a diagnostic).

### Absolute working directories

- Passed through unchanged to `Command::current_dir()`.
- Platform validation: on Windows, drive letter must exist; on Unix, path must start with `/`.

### Normalization

- Path comparison uses `platform::paths::paths_eq()` (case-insensitive on Windows, case-sensitive on Unix).
- Profile manager uses `normalize_path()` (backslash → forward slash, strip trailing separators) for matching `find_by_project_path`.

### Validation

- `project_path` existence is NOT validated at profile save time (the path may not exist yet during project creation).
- At launch time, if the resolved working directory does not exist, the step fails with an explicit error: `"Working directory '{path}' does not exist"`.

### Path resolution for applications and commands

- `OpenApplication.path` is resolved via `platform::ide::resolve_ide_executable()` which checks PATH, App Paths registry, known install dirs, and flatpak.
- Bare command names in `RunCommand.command` are resolved through `platform::command::resolve_spawn_plan()` which handles `.cmd`/`.bat` shims on Windows.
- Commands containing `/` or `\` are treated as explicit paths and not resolved.

---

## F. Process lifecycle rules

| Lifecycle | `Visibility` | `ExecutionMode` | Tracking | Step completion |
|---|---|---|---|---|
| **One-shot command** | `Captured` | `OneShot` | Full stdout/stderr capture. PID tracked. | Process exit (exit code 0 = success). |
| **Long-running service** | `Captured` | `LongRunning` | Full stdout/stderr capture. PID tracked. | Never completes from exit; user must stop it. |
| **Interactive command** | `VisibleTerminal` | `LongRunning` | Terminal PID tracked (not inner PID). Output in terminal. | User closes terminal or stops process. |
| **Detached GUI application** | `Detached` | — | No output tracking. PID may or may not be captured. | Immediate — step completes on successful launch. |
| **Native terminal launch** | `VisibleTerminal` | `LongRunning` | PID = terminal process. `ProcessTrackingQuality::TerminalWrapper`. | Terminal window closed or tracked process exits. |
| **External application** | `Detached` | — | `launch_detached()`. No process tracked. | Immediate. |
| **Readiness waiter** | — | — | No process. TCP/HTTP polling. | Success (port open / HTTP 2xx) or timeout. |
| **Delay** | — | — | No process. Thread sleep with cancellation check. | Timer expires. |
| **URL open** | — | — | No process. `webbrowser::open()`. | Immediate. |

---

## G. Failure and cancellation rules

### Failure policies

| Policy | Behavior |
|---|---|
| `StopRun` | Cancel all pending and running steps. Emit `run-status-changed` with `Failed`. |
| `SkipDependents` | Mark the failed step as `Failed`. Mark all transitive dependents as `Skipped`. Continue independent branches. |
| `WarnAndContinue` | Mark the failed step as `Failed`. All other steps continue regardless of dependency. |

### Retry

- `RetryPolicy` is checked on step failure.
- `max_retries` retries are attempted with `delay_secs` between attempts.
- `backoff_multiplier` multiplies the delay on each subsequent retry.
- Retries reset the step status to `Retrying` (emits `step-status-changed`).
- After all retries exhausted, the step's `FailurePolicy` is applied.

### Timeout

- `WaitForUrl` and `WaitForPort` have `timeout_secs`.
- On timeout, the step fails with `FailurePolicy` applied.
- `Diagnostic` with `severity: Warning` is emitted: `"Readiness check timed out after {timeout}s"`.

### Cancellation

- User cancels a run via `cancel_run(run_id)`.
- Orchestrator:
  1. Sets run status to `Cancelled`.
  2. Emits `run-status-changed` with `Cancelled`.
  3. For each running process step: calls `ProcessManager::kill()` (tree kill).
  4. For each pending wait/delay step: triggers cancellation token; step wakes and transitions to `Cancelled`.
  5. Pending steps that have not started are marked `Cancelled`.
- Already-completed steps retain their terminal status (Succeeded/Failed).
- `Cancelled` is distinct from `Failed`: cancellation is user-initiated, not an error.

### Partial success

- When some steps succeed and others fail (with `WarnAndContinue` or `SkipDependents`), the run status is `PartialSuccess`.
- `PartialSuccess` is only used when at least one step succeeded AND at least one failed/cancelled.
- The frontend displays this as "Completed with warnings" rather than a red failure.

### Diagnostic

- Every failure produces a `Diagnostic` event with `severity: Error`.
- Every timeout produces a `severity: Warning`.
- Docker preflight failures produce `severity: Warning` with source `Preflight`.
- These are emitted regardless of `FailurePolicy` — they inform, they don't control.
