// ============================================================================
// Общий раннер процессов модуля project_creator.
//
// Единственное место, где:
//   - строится tokio::process::Command (кроссплатформенное разрешение:
//     Windows нативные exe, .cmd/.bat через cmd.exe, PowerShell; Unix: sh -c);
//   - читаются stdout/stderr процесса (streaming в ExecutionEventSink,
//     хвосты для ошибок, интерактивные триггеры, idle-fallback);
//   - применяется таймаут с kill;
//   - формируются ошибки выполнения (ProcessExecutionError).
//
// И StepExecutor::run_command, и все генераторы (cli, scaffold,
// spring-boot) работают через этот раннер: раньше у них были две разные
// реализации (см. исторические StepExecutor::run_command и generators::run_cli)
// с разным поведением cmd/PowerShell, stdin, таймаутов и UI-событий.
// ============================================================================

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

use crate::modules::project_creator::models::*;

/// Сколько последних строк stdout/stderr удерживается в хвостах
/// ProcessOutput и ProcessExecutionError.
pub const OUTPUT_TAIL_LINES: usize = 12;

/// Стандартные fallback-триггеры на все случаи, когда step-специфичных нет.
static FALLBACK_TRIGGERS: &[(&str, &str)] = &[
    // Node.js / npx
    ("need to install the following packages", "y"),
    ("ok to proceed?", "y"),
    ("do you want to proceed?", "y"),
    // Универсальные булевы паттерны
    ("would you like to", "n"),
    ("do you want to", "n"),
    ("(y/n)", "y"),
    ("[y/n]", "y"),
    ("proceed?", "y"),
    ("accept?", "y"),
    // Зависимости
    ("install dependencies", "y"),
    ("download and install", "n"),
    ("install the ios and android", "n"),
    ("initialize a new git", "y"),
    // CocoaPods (macOS)
    ("install cocoapods", "n"),
    // Expo
    ("do you want to log in", "n"),
    // Лицензии
    ("accept the license", "y"),
    ("review licenses", "y"),
];

/// Режим stdin запускаемого процесса.
#[derive(Debug, Clone, Default)]
pub enum StdinMode {
    /// stdin = null (неинтерактивные команды, генераторы).
    #[default]
    Null,
    /// stdin = pipe: детекция триггеров в stdout, ответы в stdin,
    /// idle-fallback (после паузы без вывода отправляется Enter).
    Piped(InteractiveRules),
}

/// Интерактивные правила для piped-режима.
#[derive(Debug, Clone, Default)]
pub struct InteractiveRules {
    /// (триггер, ответ) — step-специфичные; поверх них работают fallback-триггеры.
    pub entries: Vec<(String, ResponseType)>,
}

/// Полное описание одного запуска процесса.
#[derive(Debug, Clone, Default)]
pub struct ProcessSpec {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub timeout: Option<Duration>,
    pub stdin: StdinMode,
    /// Инжектировать CI-окружение: CI=1, NPM_CONFIG_YES, npm_config_yes.
    /// CI заставляет npx/npm/create-* CLI пропускать интерактивные промпты,
    /// NPM_CONFIG_YES отвечает «да» на подтверждение установки пакета у npx.
    pub ci_mode: bool,
}

impl ProcessSpec {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Self::default()
        }
    }
}

/// Результат успешно завершённого процесса.
#[derive(Debug, Clone)]
pub struct ProcessOutput {
    /// Последние OUTPUT_TAIL_LINES строк stdout.
    pub stdout_tail: String,
    /// Последние OUTPUT_TAIL_LINES строк stderr.
    pub stderr_tail: String,
    pub duration_ms: u64,
}

/// Категория ошибки выполнения процесса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessErrorKind {
    /// Процесс не удалось запустить.
    Spawn { source: String },
    /// Не удалось дождаться завершения.
    Wait { source: String },
    /// Таймаут: процесс убит после timeout_secs секунд.
    Timeout { timeout_secs: u64 },
    /// Процесс завершился с ошибкой (ненулевой код / сигнал).
    Exit { code: String },
    /// Ошибка чтения stdout/stderr процесса (пайп закрыт с ошибкой).
    /// Ошибки ридеров не замалчиваются: даже при успешном exit-коде
    /// ненадёжный вывод превращает результат в ошибку этого вида.
    ReadOutput {
        stream: &'static str,
        source: String,
    },
}

impl ProcessErrorKind {
    pub fn reason(&self) -> String {
        match self {
            ProcessErrorKind::Spawn { source } => format!("failed to spawn: {source}"),
            ProcessErrorKind::Wait { source } => format!("process wait failed: {source}"),
            ProcessErrorKind::Timeout { timeout_secs } => {
                format!("timed out after {timeout_secs} seconds")
            }
            ProcessErrorKind::Exit { code } => format!("exited with status {code}"),
            ProcessErrorKind::ReadOutput { stream, source } => {
                format!("failed to read {stream}: {source}")
            }
        }
    }
}

/// Ошибка выполнения процесса: причина, команда и её аргументы, рабочая
/// директория и хвосты обоих потоков — всё, что нужно для StepResult/
/// GenerationReport.
#[derive(Debug, Clone)]
pub struct ProcessExecutionError {
    pub kind: ProcessErrorKind,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

impl ProcessExecutionError {
    pub fn reason(&self) -> String {
        self.kind.reason()
    }

    /// Командная строка «command arg1 arg2 ...» для сообщений об ошибках.
    pub fn command_line(&self) -> String {
        command_display(&self.command, &self.args)
    }

    /// Полное описание ошибки: точная причина (вид ошибки), команда и её
    /// аргументы, рабочая директория, хвосты stdout/stderr.
    pub fn format_command_error(&self) -> String {
        let args = if self.args.is_empty() {
            "<none>".to_string()
        } else {
            self.args.join(" ")
        };
        format!(
            "Command failed ({})\ncommand: {}\nargs: {}\nworking directory: {}\n{}",
            self.reason(),
            self.command,
            args,
            self.working_dir.display(),
            format_output_tails(&self.stdout_tail, &self.stderr_tail),
        )
    }
}

impl std::fmt::Display for ProcessExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format_command_error())
    }
}

impl std::error::Error for ProcessExecutionError {}

/// Событийный приёмник: маршрутизирует stdout/stderr процесса в канал
/// ExecutionEvent — тот же механизм, что использует StepExecutor для
/// Step::Command. Шаг (id/index/total/name/description) фиксируется в
/// приёмнике на время выполнения шага.
#[derive(Clone)]
pub struct ExecutionEventSink {
    tx: mpsc::Sender<ExecutionEvent>,
    step_id: String,
    step_index: usize,
    total_steps: usize,
    step_name: String,
    step_description: String,
}

impl ExecutionEventSink {
    pub fn new(
        tx: mpsc::Sender<ExecutionEvent>,
        step_id: impl Into<String>,
        step_index: usize,
        total_steps: usize,
        step_name: impl Into<String>,
        step_description: impl Into<String>,
    ) -> Self {
        Self {
            tx,
            step_id: step_id.into(),
            step_index,
            total_steps,
            step_name: step_name.into(),
            step_description: step_description.into(),
        }
    }

    pub async fn emit_stdout(&self, text: &str) {
        self.emit(ExecutionEventType::StepProgress {
            stdout: text.to_string(),
            stderr: String::new(),
        })
        .await;
    }

    pub async fn emit_stderr(&self, text: &str) {
        self.emit(ExecutionEventType::StepProgress {
            stdout: String::new(),
            stderr: text.to_string(),
        })
        .await;
    }

    async fn emit(&self, event_type: ExecutionEventType) {
        let _ = self
            .tx
            .send(ExecutionEvent {
                event_type,
                step_id: self.step_id.clone(),
                step_index: self.step_index,
                total_steps: self.total_steps,
                step_name: self.step_name.clone(),
                step_description: self.step_description.clone(),
                timestamp: local_time(),
            })
            .await;
    }
}

/// Единый раннер процессов модуля project_creator. Stateless: никаких
/// внутренних буферов или состояния между запусками.
pub struct ProcessRunner;

/// Шина запуска процессов: абстракция над ProcessRunner для тестов.
/// StepExecutor и генераторы запускают CLI только через CommandRunner —
/// мок-реализации подменяют реальные процессы в тестах исполнения
/// (см. MockCommandRunner в engine/mod.rs).
#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(
        &self,
        spec: ProcessSpec,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<ProcessOutput, ProcessExecutionError>;
}

#[async_trait::async_trait]
impl CommandRunner for ProcessRunner {
    async fn run(
        &self,
        spec: ProcessSpec,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<ProcessOutput, ProcessExecutionError> {
        ProcessRunner::run(spec, sink).await
    }
}

impl ProcessRunner {
    pub async fn run(
        spec: ProcessSpec,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<ProcessOutput, ProcessExecutionError> {
        let command = spec.command.clone();
        let args = spec.args.clone();
        let working_dir = spec
            .working_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("."));
        let start = Instant::now();

        let mut cmd = build_command(&spec);
        if let Some(dir) = &spec.working_dir {
            cmd.current_dir(dir);
        }
        if let Some(env) = &spec.env {
            cmd.envs(env);
        }
        if spec.ci_mode {
            cmd.env("CI", "1");
            cmd.env("NPM_CONFIG_YES", "true");
            cmd.env("npm_config_yes", "true");
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        match &spec.stdin {
            StdinMode::Null => {
                cmd.stdin(std::process::Stdio::null());
            }
            StdinMode::Piped(_) => {
                cmd.stdin(std::process::Stdio::piped());
            }
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(source) => {
                return Err(ProcessExecutionError {
                    kind: ProcessErrorKind::Spawn {
                        source: source.to_string(),
                    },
                    command,
                    args,
                    working_dir,
                    stdout_tail: String::new(),
                    stderr_tail: String::new(),
                });
            }
        };

        let stdout_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let stdin = match &spec.stdin {
            StdinMode::Piped(_) => child.stdin.take(),
            StdinMode::Null => None,
        };
        let stdout = child.stdout.take().expect("stdout should be piped");
        let stderr = child.stderr.take().expect("stderr should be piped");

        let interactive = match &spec.stdin {
            StdinMode::Piped(rules) => rules.clone(),
            StdinMode::Null => InteractiveRules::default(),
        };
        let sink_stdout = sink.cloned();
        let sink_stderr = sink.cloned();
        let stdout_tail_task = Arc::clone(&stdout_tail);
        let stderr_tail_task = Arc::clone(&stderr_tail);

        // Ошибки ридеров stdout/stderr не замалчиваются: первая ошибка чтения
        // попадает в общий слот и, если процесс в остальном завершился
        // успешно, превращается в различимый ProcessErrorKind::ReadOutput.
        let reader_error: Arc<Mutex<Option<(&'static str, String)>>> = Arc::new(Mutex::new(None));
        let stdout_reader_error = Arc::clone(&reader_error);
        let stderr_reader_error = Arc::clone(&reader_error);

        let stdout_task = tokio::spawn(async move {
            if let Err(source) =
                read_stdout_loop(stdout, stdin, interactive, stdout_tail_task, sink_stdout).await
            {
                if let Ok(mut slot) = stdout_reader_error.lock() {
                    *slot = Some(("stdout", source));
                }
            }
        });
        let stderr_task = tokio::spawn(async move {
            if let Err(source) = read_stderr_loop(stderr, stderr_tail_task, sink_stderr).await {
                if let Ok(mut slot) = stderr_reader_error.lock() {
                    *slot = Some(("stderr", source));
                }
            }
        });

        // Таймаут: убиваем процесс и возвращаем различимый Timeout-вид
        // ошибки (в отличие от обычного Exit/Wait/Spawn).
        let wait_result = match spec.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, child.wait()).await {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(source)) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Err(ProcessErrorKind::Wait {
                        source: source.to_string(),
                    })
                }
                Err(_elapsed) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Err(ProcessErrorKind::Timeout {
                        timeout_secs: timeout.as_secs(),
                    })
                }
            },
            None => match child.wait().await {
                Ok(status) => Ok(status),
                Err(source) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Err(ProcessErrorKind::Wait {
                        source: source.to_string(),
                    })
                }
            },
        };

        let _ = stdout_task.await;
        let _ = stderr_task.await;

        let reader_failure = reader_error.lock().ok().and_then(|slot| slot.clone());

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout_detail = tail_text(&stdout_tail);
        let stderr_detail = tail_text(&stderr_tail);

        match wait_result {
            Ok(status) if status.success() => match reader_failure {
                Some((stream, source)) => Err(ProcessExecutionError {
                    kind: ProcessErrorKind::ReadOutput { stream, source },
                    command,
                    args,
                    working_dir,
                    stdout_tail: stdout_detail,
                    stderr_tail: stderr_detail,
                }),
                None => Ok(ProcessOutput {
                    stdout_tail: stdout_detail,
                    stderr_tail: stderr_detail,
                    duration_ms,
                }),
            },
            Ok(status) => Err(ProcessExecutionError {
                kind: ProcessErrorKind::Exit {
                    code: exit_status_text(&status),
                },
                command,
                args,
                working_dir,
                stdout_tail: stdout_detail,
                stderr_tail: stderr_detail,
            }),
            Err(kind) => Err(ProcessExecutionError {
                kind,
                command,
                args,
                working_dir,
                stdout_tail: stdout_detail,
                stderr_tail: stderr_detail,
            }),
        }
    }
}

/// Прочитать stdout побайтово: стриминг в sink, накопление хвоста,
/// детекция триггеров и отправка ответов в stdin (если stdin piped),
/// idle-fallback при молчании процесса. Ошибка чтения — Err (не
/// замалчивается, как раньше, а становится ProcessErrorKind::ReadOutput).
async fn read_stdout_loop(
    mut stdout: tokio::process::ChildStdout,
    mut stdin: Option<tokio::process::ChildStdin>,
    interactive: InteractiveRules,
    stdout_tail: Arc<Mutex<Vec<String>>>,
    sink: Option<ExecutionEventSink>,
) -> Result<(), String> {
    // 256-байтовый буфер — не ждём \n, читаем как только данные появляются
    let mut buf = [0u8; 256];
    // Скользящее окно 2048 символов
    let mut rolling = String::with_capacity(2048);
    // Таймаут бездействия перед fallback-опросом
    let idle_timeout = Duration::from_secs(4);

    loop {
        let read_fut = stdout.read(&mut buf);
        let result = tokio::time::timeout(idle_timeout, read_fut).await;

        match result {
            // EOF — процесс закрыл stdout
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);

                capture_tail(&stdout_tail, &chunk);

                // Даже успешные команды стримят вывод в UI
                if let Some(sink) = &sink {
                    sink.emit_stdout(&chunk).await;
                }

                // Добавляем в скользящее окно
                rolling.push_str(&chunk);
                if rolling.len() > 2048 {
                    rolling.drain(..rolling.len() - 2048);
                }

                // Ищем триггеры (только если stdin открыт)
                if let Some(stdin) = &mut stdin {
                    let matched = check_triggers(&rolling, &interactive, stdin).await;
                    if matched {
                        rolling.clear(); // Очищаем окно после ответа
                    }
                }
            }
            // Ошибка чтения: фиксируем и завершаем цикл — ошибка не
            // замалчивается (см. ReadOutput)
            Ok(Err(source)) => return Err(source.to_string()),
            // Таймаут — процесс молчит, возможно ждёт ввода без триггера
            Err(_elapsed) => {
                let Some(stdin) = &mut stdin else {
                    continue;
                };
                // Пробуем последний раз проверить буфер и отправить fallback
                if !rolling.is_empty() {
                    let matched = check_triggers(&rolling, &interactive, stdin).await;
                    if matched {
                        rolling.clear();
                        continue;
                    }
                }
                // Если процесс ещё жив — отправляем "y\n" как последнее средство
                let _ = stdin.write_all(b"\n").await;
                let _ = stdin.flush().await;
            }
        }
    }
    Ok(())
}

/// Прочитать stderr побайтово: стриминг в sink и накопление хвоста.
/// Не-UTF-8 байты (Windows cmd пишет в OEM-кодировке) конвертируются
/// lossy — это НЕ ошибка чтения; ошибкой считается только сбой самого
/// пайпа (см. ReadOutput).
async fn read_stderr_loop(
    mut stderr: tokio::process::ChildStderr,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    sink: Option<ExecutionEventSink>,
) -> Result<(), String> {
    let mut buf = [0u8; 256];
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                capture_tail(&stderr_tail, &chunk);
                if let Some(sink) = &sink {
                    sink.emit_stderr(&chunk).await;
                }
            }
            Err(source) => return Err(source.to_string()),
        }
    }
    Ok(())
}

/// Отправить ответ в stdin процесса согласно ResponseType.
async fn send_response(stdin: &mut tokio::process::ChildStdin, rt: &ResponseType) {
    let bytes: Vec<u8> = match rt {
        ResponseType::Text(val) => format!("{}\n", val).into_bytes(),
        ResponseType::Confirm(true) => b"y\n".to_vec(),
        ResponseType::Confirm(false) => b"n\n".to_vec(),
        ResponseType::Select(idx) => {
            // *idx* раз нажать стрелку вниз, затем Enter
            let mut seq = Vec::new();
            for _ in 0..*idx {
                seq.extend_from_slice(b"\x1b[B"); // Down arrow
            }
            seq.push(b'\n'); // Enter
            seq
        }
        ResponseType::Keys(raw) => raw.as_bytes().to_vec(),
    };
    let _ = stdin.write_all(&bytes).await;
    let _ = stdin.flush().await;
}

/// Проверить rolling-буфер на совпадение с любым триггером.
/// Возвращает true, если совпадение найдено и ответ отправлен.
async fn check_triggers(
    rolling: &str,
    interactive: &InteractiveRules,
    stdin: &mut tokio::process::ChildStdin,
) -> bool {
    let lower = rolling.to_lowercase();
    for (trigger, response_type) in &interactive.entries {
        if lower.contains(&trigger.to_lowercase()) {
            send_response(stdin, response_type).await;
            return true;
        }
    }
    // Fallback — более широкая сеть
    for (trigger, response) in FALLBACK_TRIGGERS {
        if lower.contains(trigger) {
            let _ = stdin.write_all(response.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
            return true;
        }
    }
    false
}

// ============================================================================
// Кроссплатформенное построение команды
// ============================================================================

/// Кроссплатформенное построение команды.
///
/// Delegates to [`crate::platform::command`] for platform-aware dispatch:
///   - Windows: PowerShell via `powershell -Command`; .cmd/.bat via
///     `cmd /D /C`; direct commands for everything else.
///   - Unix: `sh -c` with sh-quoted arguments (preserves existing behavior).
fn build_command(spec: &ProcessSpec) -> TokioCommand {
    use crate::platform::command::{build_tokio_command, infer_command_mode};

    let mode = infer_command_mode(&spec.command, &spec.args, None);
    // infer_command_mode only returns OS-compatible shells, so Ok is guaranteed.
    let mut cmd = build_tokio_command(&spec.command, &spec.args, mode)
        .expect("infer_command_mode must return a valid mode for the current OS");

    // PowerShell heuristic: extra args appended after -Command <script>.
    if crate::platform::command::is_powershell_command(&spec.command) && !spec.args.is_empty() {
        // Re-build with args appended to the command string.
        let mut full_script = spec.command.clone();
        for arg in &spec.args {
            full_script.push(' ');
            full_script.push_str(arg);
        }
        cmd = TokioCommand::new("powershell");
        cmd.arg("-Command");
        cmd.arg(&full_script);
    }

    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);

    cmd
}

// ============================================================================
// Кроссплатформенные хелперы — делегируют в crate::platform
// ============================================================================

/// Параметр в командную строку cmd.exe (делегирует в platform::command).
pub fn win_quote_arg(arg: &str) -> String {
    crate::platform::command::win_quote_arg(arg)
}

/// Командная строка для `cmd /S /C` (обёртка с внешними кавычками).
pub fn win_command_line(command: &str, args: &[String]) -> String {
    let mut line = win_quote_arg(command);
    for arg in args {
        line.push(' ');
        line.push_str(&win_quote_arg(arg));
    }
    format!("\"{}\"", line)
}

/// Параметр для `sh -c` (делегирует в platform::command).
pub fn sh_quote(arg: &str) -> String {
    crate::platform::command::sh_quote(arg)
}

/// Имя исполняемого файла для прямого запуска на Windows
/// (делегирует в platform::command).
pub fn windows_command_program(command: &str) -> String {
    crate::platform::command::resolve_windows_program_name(command)
}

pub fn is_windows_batch(command: &str) -> bool {
    crate::platform::paths::is_batch_file(command)
}

/// Безопасная строка для cmd /C: кавычки только вокруг отдельных токенов,
/// без внешней пары, которая превращала `node -e "..."` в один аргумент.
pub fn windows_shell_line(command: &str, args: &[String]) -> String {
    let mut line = win_quote_arg(command);
    for arg in args {
        line.push(' ');
        line.push_str(&win_quote_arg(arg));
    }
    line
}

// ============================================================================
// Общие помощники
// ============================================================================

/// Человекочитаемое «command arg1 arg2 ...» для сообщений об ошибках.
pub fn command_display(command: &str, args: &[String]) -> String {
    let mut display = command.to_string();
    for arg in args {
        display.push(' ');
        display.push_str(arg);
    }
    display
}

fn capture_tail(tail: &Arc<Mutex<Vec<String>>>, text: &str) {
    if let Ok(mut lines) = tail.lock() {
        for line in text.lines() {
            lines.push(line.to_string());
            if lines.len() > OUTPUT_TAIL_LINES {
                lines.remove(0);
            }
        }
    }
}

fn tail_text(tail: &Arc<Mutex<Vec<String>>>) -> String {
    tail.lock()
        .map(|lines| lines.join("\n"))
        .unwrap_or_default()
}

fn format_output_tails(stdout: &str, stderr: &str) -> String {
    let stdout = if stdout.trim().is_empty() {
        "<empty>"
    } else {
        stdout
    };
    let stderr = if stderr.trim().is_empty() {
        "<empty>"
    } else {
        stderr
    };
    format!("stdout tail:\n{stdout}\nstderr tail:\n{stderr}")
}

fn exit_status_text(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "terminated by signal".to_string(),
    }
}

pub fn local_time() -> String {
    Local::now().format("%H::%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_command_program_resolves_npm_ecosystem() {
        assert_eq!(windows_command_program("npx"), "npx.cmd");
        assert_eq!(windows_command_program("npm"), "npm.cmd");
        assert_eq!(windows_command_program("pnpm"), "pnpm.cmd");
        assert_eq!(windows_command_program("yarn"), "yarn.cmd");
        assert_eq!(windows_command_program("vite"), "vite.cmd");
        assert_eq!(windows_command_program("nest"), "nest.cmd");
        assert_eq!(windows_command_program("composer"), "composer.bat");
    }

    #[test]
    fn windows_command_program_keeps_paths_and_extensions() {
        // Пути и уже расширенные имена не трогаются
        assert_eq!(
            windows_command_program("C:\\Users\\John Doe\\app\\venv\\Scripts\\pip.exe"),
            "C:\\Users\\John Doe\\app\\venv\\Scripts\\pip.exe"
        );
        assert_eq!(
            windows_command_program("C:\\tools\\script.cmd"),
            "C:\\tools\\script.cmd"
        );
        assert_eq!(windows_command_program("script.bat"), "script.bat");
        // Обычные команды без расширения
        assert_eq!(windows_command_program("node"), "node");
        assert_eq!(windows_command_program("python"), "python");
        assert_eq!(windows_command_program("dotnet"), "dotnet");
    }

    #[test]
    fn windows_batch_detection() {
        assert!(is_windows_batch("npx.cmd"));
        assert!(is_windows_batch("composer.bat"));
        assert!(!is_windows_batch("node"));
        assert!(!is_windows_batch("npm.cmd.exe"));
        assert!(!is_windows_batch(""));
    }

    #[test]
    fn node_e_script_stays_single_cmd_argument() {
        // JS-скрипт с пробелами и скобками обязан остаться ОДНИМ аргументом
        // cmd — без этого node -e превращается в набор токенов.
        let script = "const fs=require('fs');const j=JSON.parse(fs.readFileSync('package.json','utf8'));j.name='my app';fs.writeFileSync('package.json',JSON.stringify(j,null,2)+'\\n')";
        let line = windows_shell_line("node", &["-e".into(), script.into()]);
        assert!(line.starts_with("node -e "), "{line}");
        assert!(line.contains(&format!("\"{}\"", script)), "{line}");

        // win_command_line (внешняя обёртка для cmd /S /C) — тоже без потери
        let line = win_command_line("node", &["-e".into(), script.into()]);
        assert!(line.contains(&format!("\"{}\"", script)), "{line}");
    }

    #[test]
    fn win_command_line_quotes_spaces_and_metachars() {
        // Команда-путь с пробелами (venv\Scripts\pip.exe) оборачивается во
        // внешние кавычки: cmd /S /C снимает внешнюю пару, внутренние
        // кавычки сохраняются — токен не режется по пробелу.
        let line = win_command_line(
            "C:\\Users\\John Doe\\app\\venv\\Scripts\\pip.exe",
            &[
                "install".into(),
                "-r".into(),
                "C:\\req file.txt".into(),
                "alembic".into(),
            ],
        );
        assert_eq!(
            line,
            "\"\"C:\\Users\\John Doe\\app\\venv\\Scripts\\pip.exe\" install -r \"C:\\req file.txt\" alembic\""
        );
        // Простая команда: внешняя обёртка есть, лишних кавычек внутри нет.
        let plain = win_command_line("php", &["--version".into()]);
        assert_eq!(plain, "\"php --version\"");
        // %: cmd раскрывает %VAR%, поэтому аргумент обязан быть в кавычках.
        let curl = win_command_line("curl", &["-w".into(), "%{http_code}".into()]);
        assert_eq!(curl, "\"curl -w \"%{http_code}\"\"");
    }

    #[test]
    fn sh_quote_protects_metachars() {
        assert_eq!(sh_quote("plain"), "plain");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("a&b;c"), "'a&b;c'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("$HOME"), "'$HOME'");
    }

    #[test]
    fn timeout_error_is_distinguishable_and_formatted() {
        let err = ProcessExecutionError {
            kind: ProcessErrorKind::Timeout { timeout_secs: 42 },
            command: "npx".into(),
            args: vec!["create-vite@latest".into(), "my-app".into()],
            working_dir: PathBuf::from(r"C:\Projects\app"),
            stdout_tail: "fetching template...".into(),
            stderr_tail: String::new(),
        };
        assert_eq!(err.reason(), "timed out after 42 seconds");
        assert!(matches!(
            err.kind,
            ProcessErrorKind::Timeout { timeout_secs: 42 }
        ));
        let text = err.format_command_error();
        assert!(
            text.starts_with("Command failed (timed out after 42 seconds)"),
            "{text}"
        );
        assert!(text.contains("command: npx"), "{text}");
        assert!(text.contains("args: create-vite@latest my-app"), "{text}");
        assert!(
            text.contains("working directory: C:\\Projects\\app"),
            "{text}"
        );
        assert!(
            text.contains("stdout tail:\nfetching template..."),
            "{text}"
        );
        assert!(text.contains("stderr tail:\n<empty>"), "{text}");
    }

    #[test]
    fn spawn_exit_wait_kinds_are_distinct() {
        let exit = ProcessExecutionError {
            kind: ProcessErrorKind::Exit { code: "22".into() },
            command: "curl".into(),
            args: vec![],
            working_dir: PathBuf::from("."),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        };
        assert_eq!(exit.reason(), "exited with status 22");

        let spawn = ProcessExecutionError {
            kind: ProcessErrorKind::Spawn {
                source: "Access is denied".into(),
            },
            command: "npm".into(),
            args: vec![],
            working_dir: PathBuf::from("."),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        };
        assert_eq!(spawn.reason(), "failed to spawn: Access is denied");

        let wait = ProcessExecutionError {
            kind: ProcessErrorKind::Wait {
                source: "broken pipe".into(),
            },
            command: "npm".into(),
            args: vec![],
            working_dir: PathBuf::from("."),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        };
        assert_eq!(wait.reason(), "process wait failed: broken pipe");

        let read = ProcessExecutionError {
            kind: ProcessErrorKind::ReadOutput {
                stream: "stdout",
                source: "broken pipe".into(),
            },
            command: "npm".into(),
            args: vec![],
            working_dir: PathBuf::from("."),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        };
        assert_eq!(read.reason(), "failed to read stdout: broken pipe");
        let text = read.format_command_error();
        assert!(
            text.contains("failed to read stdout: broken pipe"),
            "{text}"
        );
        assert!(text.contains("args: <none>"), "{text}");
    }

    #[test]
    fn command_line_joins_command_and_args() {
        let err = ProcessExecutionError {
            kind: ProcessErrorKind::Exit { code: "1".into() },
            command: "npm".into(),
            args: vec!["install".into(), "react".into()],
            working_dir: PathBuf::from("."),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        };
        assert_eq!(err.command_line(), "npm install react");
    }

    #[test]
    fn capture_tail_keeps_last_lines() {
        let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let text: String = (0..OUTPUT_TAIL_LINES + 5)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        capture_tail(&tail, &text);
        let joined = tail_text(&tail);
        assert_eq!(joined.lines().count(), OUTPUT_TAIL_LINES);
        assert!(joined.starts_with("line5"), "{joined}");
        assert!(joined.ends_with("line16"), "{joined}");
    }

    #[test]
    fn command_display_joins_command_and_args() {
        assert_eq!(
            command_display("npm", &["install".into(), "react".into()]),
            "npm install react"
        );
        assert_eq!(command_display("echo", &[]), "echo");
    }

    #[tokio::test]
    async fn runner_streams_stdout_stderr_and_captures_tails() {
        let (tx, mut rx) = mpsc::channel(64);
        let sink = ExecutionEventSink::new(tx, "test_step", 0, 1, "Test", "desc");

        let spec = if cfg!(target_os = "windows") {
            ProcessSpec {
                command: "cmd".into(),
                args: vec!["/C".into(), "echo out-line && echo err-line 1>&2".into()],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(30)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        } else {
            ProcessSpec {
                command: "echo out-line; echo err-line 1>&2".into(),
                args: vec![],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(30)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        };

        let output = ProcessRunner::run(spec, Some(&sink))
            .await
            .expect("process must succeed");
        assert!(
            output.stdout_tail.contains("out-line"),
            "stdout tail: {}",
            output.stdout_tail
        );
        assert!(
            output.stderr_tail.contains("err-line"),
            "stderr tail: {}",
            output.stderr_tail
        );

        let mut saw_out = false;
        let mut saw_err = false;
        while let Ok(event) = rx.try_recv() {
            if let ExecutionEventType::StepProgress { stdout, stderr } = &event.event_type {
                if stdout.contains("out-line") {
                    saw_out = true;
                }
                if stderr.contains("err-line") {
                    saw_err = true;
                }
            }
        }
        assert!(saw_out, "stdout обязан стримиться в события UI");
        assert!(saw_err, "stderr обязан стримиться в события UI");
    }

    #[tokio::test]
    async fn runner_reports_exit_with_stderr_tail() {
        // Команда пишет ТОЛЬКО в stderr и завершается ненулевым кодом:
        // хвост stderr обязан дойти до ошибки, вид — различимый Exit.
        let spec = if cfg!(target_os = "windows") {
            ProcessSpec {
                command: "cmd".into(),
                args: vec!["/C".into(), "echo boom 1>&2 & exit /b 7".into()],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(30)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        } else {
            ProcessSpec {
                command: "echo boom >&2; exit 7".into(),
                args: vec![],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(30)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        };

        let err = ProcessRunner::run(spec, None).await.unwrap_err();
        assert!(
            matches!(&err.kind, ProcessErrorKind::Exit { code } if code == "7"),
            "{:?}",
            err.kind
        );
        assert!(
            err.stderr_tail.contains("boom"),
            "stderr tail: {}",
            err.stderr_tail
        );
        let text = err.format_command_error();
        assert!(text.contains("exited with status 7"), "{text}");
        assert!(text.contains("stdout tail:"), "{text}");
        assert!(text.contains("stderr tail:\nboom"), "{text}");
    }

    #[tokio::test]
    async fn runner_reports_spawn_failure() {
        // Windows-запуск несуществующей программы — различимый Spawn-вид.
        // На Unix обёртка `sh -c` стартует всегда (команда упадёт в Exit 127),
        // поэтому проверка ограничена Windows — основной платформой.
        if !cfg!(target_os = "windows") {
            return;
        }
        let spec = ProcessSpec::new("pc_runner_does_not_exist_xyz_98765");
        let err = ProcessRunner::run(spec, None).await.unwrap_err();
        assert!(
            matches!(err.kind, ProcessErrorKind::Spawn { .. }),
            "{:?}",
            err.kind
        );
        assert!(
            err.format_command_error().contains("failed to spawn"),
            "{}",
            err.format_command_error()
        );
    }

    #[tokio::test]
    async fn runner_executes_absolute_executable_path() {
        // Абсолютный путь к .cmd-файлу запускается через cmd.exe
        // (CreateProcess batch-файлы не умеет) — команда не обязана быть
        // в PATH. Примечание: имя файла без пробелов — cmd /C с кавычками
        // в пути, содержащем пробелы, конфликтует с повторным кавычкованием
        // аргументов Rust'ом (тройные кавычки режутся cmd по пробелу).
        if !cfg!(target_os = "windows") {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_abs_cmd_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bat = dir.join("abs_probe.cmd");
        std::fs::write(&bat, "@echo off\r\necho abs-path-ok\r\n").unwrap();
        let spec = ProcessSpec {
            command: bat.to_string_lossy().into_owned(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout: Some(Duration::from_secs(30)),
            stdin: StdinMode::Null,
            ci_mode: false,
        };
        let output = ProcessRunner::run(spec, None)
            .await
            .expect("absolute .cmd path must run");
        assert!(
            output.stdout_tail.contains("abs-path-ok"),
            "stdout tail: {}",
            output.stdout_tail
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn runner_kills_process_on_timeout() {
        // Процесс без дочерних: после kill закрываются оба пайпа, чтение
        // завершается сразу (дочерние процессы вроде ping/sleep держали бы
        // pipe-хендлы и блокировали read-цикл до своего выхода).
        let spec = if cfg!(target_os = "windows") {
            ProcessSpec {
                command: "ping".into(),
                args: vec!["-n".into(), "60".into(), "127.0.0.1".into()],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(1)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        } else {
            ProcessSpec {
                command: "exec sleep 60".into(),
                args: vec![],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(1)),
                stdin: StdinMode::Null,
                ci_mode: false,
            }
        };

        let start = Instant::now();
        let err = ProcessRunner::run(spec, None).await.unwrap_err();
        assert!(
            matches!(err.kind, ProcessErrorKind::Timeout { timeout_secs: 1 }),
            "{:?}",
            err.kind
        );
        assert_eq!(err.reason(), "timed out after 1 seconds");
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "процесс обязан быть убит после таймаута"
        );
    }
}
