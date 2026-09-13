// ============================================================
// Запуск процессов (console.rs) — этап 4
// ============================================================
// Низкоуровневый слой работы с процессами и PowerShell:
//
//   - piped_run    — запуск бинаря, стриминг stdout/stderr построчно
//                    в EventSink, отмена по флагу (kill процесса);
//   - run_elevated — запуск с правами администратора (UAC-промпт
//                    через Start-Process -Verb RunAs), вывод
//                    перехватывается в файлы и стримится после;
//   - run_tool_script — выполнение произвольного PS-скрипта из файла;
//   - download     — скачивание по URL с прогрессом (tc:dl байты/всего)
//                    и таймаутом.
//
// Всё, что здесь запускается, живёт в Windows-first режиме: Linux/macOS
// обёртки (sudo/brew) добавляются позже. Сюда же переехали базовые
// помощники событий (EventSink/timestamp/event) — слой, общий для
// installer и любых будущих сервисов (update, health).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use crate::modules::toolchain::models::*;
use crate::modules::toolchain::platforms;

// ------------------------------------------------------------
// События (общий слой)
// ------------------------------------------------------------

/// Получатель событий. Команды реализуют через app.emit
/// ("toolchain:task_event"), тесты — через буфер в памяти.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: ToolchainEvent);
}

/// Текущее время в RFC3339 — метка для событий и сессий.
pub fn timestamp() -> String {
    chrono::Local::now().to_rfc3339()
}

/// Собирает ToolchainEvent с общими полями задачи.
pub fn event(
    event_type: ToolchainEventType,
    task_index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
) -> ToolchainEvent {
    ToolchainEvent {
        event_type,
        task_index,
        total_tasks: total,
        task_id: task_id.to_string(),
        tool_id: tool_id.to_string(),
        timestamp: timestamp(),
        session_id: session_id.to_string(),
    }
}

// ------------------------------------------------------------
// Временные файлы задания (уникальность + уборка)
// ------------------------------------------------------------

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static TEMP_REGISTRY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Уникальный путь во временном каталоге: pid + счётчик + время.
/// Никаких предсказуемых имён вида `tc-{tool}-{pid}` — их можно
/// было бы перехватить (temp-file squatting) и подсунуть свой файл.
fn unique_temp_path(prefix: &str, suffix: &str) -> PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let safe_prefix: String = prefix
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    std::env::temp_dir().join(format!(
        "tc-{safe_prefix}-{}-{n}-{nanos}{suffix}",
        std::process::id()
    ))
}

/// Создаёт уникальный временный файл и регистрирует его для уборки.
pub fn tracked_temp_file(prefix: &str, suffix: &str) -> PathBuf {
    let path = unique_temp_path(prefix, suffix);
    if let Ok(mut reg) = TEMP_REGISTRY.lock() {
        reg.push(path.clone());
    }
    path
}

/// Убирает все временные артефакты текущего процесса, созданные через
/// tracked_temp_file. Вызывается в конце execute_plan (успех/сбой/отмена)
/// — гонки между параллельными заданиями нет: у каждого свои пути.
pub fn cleanup_tracked_temp_files() {
    if let Ok(mut reg) = TEMP_REGISTRY.lock() {
        for path in reg.drain(..) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Возраст, после которого осиротевшие временные файлы считаются мусором.
/// Крэш/убийство процесса посреди установки не может вызвать
/// cleanup_tracked_temp_files — зачистка переживает перезапуск здесь.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Убирает осиротевшие временные файлы задания (tc-* в системном temp):
/// результат крэша/убийства процесса. Вызывается ОДИН раз при старте
/// приложения. Свежие файлы (в т.ч. другого живого процесса) не трогаются.
pub fn sweep_stale_temp_files() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("tc-") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let too_old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > STALE_TEMP_AGE);
        if too_old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ------------------------------------------------------------
// Результат процесса
// ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PipedResult {
    pub success: bool,
    /// true — процесс убит из-за запрошенной отмены
    pub aborted: bool,
    pub code: i32,
    /// Последняя строка вывода с префиксом `tc:error ` — причина
    /// сбоя, написанная самим скриптом (например «404 Not Found»
    /// или «не найден: winget»).
    pub error_line: Option<String>,
}

impl PipedResult {
    fn from_status(
        status: std::process::ExitStatus,
        aborted: bool,
        error_line: Option<String>,
    ) -> Self {
        PipedResult {
            success: !aborted && status.success(),
            aborted,
            code: status.code().unwrap_or(-1),
            error_line,
        }
    }
}

// ------------------------------------------------------------
// Стриминг вывода
// ------------------------------------------------------------

/// Читает поток построчно и шлёт каждую непустую строку как
/// TaskProgress. Прерывается по флагу отмены (aborted = true).
/// Возвращает (aborted, последняя строка с префиксом `tc:error`).
async fn stream_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: BufReader<R>,
    sink: Arc<dyn EventSink>,
    index: usize,
    total: usize,
    task_id: String,
    tool_id: String,
    session_id: String,
    abort: Arc<AtomicBool>,
) -> (bool, Option<String>) {
    let mut aborted = false;
    let mut error_line: Option<String> = None;
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(msg) = line.strip_prefix("tc:error ") {
            error_line = Some(msg.to_string());
        }
        sink.emit(event(
            ToolchainEventType::TaskProgress { line },
            index,
            total,
            &task_id,
            &tool_id,
            &session_id,
        ));
        if abort.load(Ordering::SeqCst) {
            aborted = true;
            break;
        }
    }
    (aborted, error_line)
}

// ------------------------------------------------------------
// Обычный запуск со стримингом
// ------------------------------------------------------------

/// Запускает процесс, стримит его вывод построчно, ждёт завершения.
/// stdout и stderr читаются параллельно (отдельные задачи), чтобы
/// полный stderr-буфер не заблокировал установщик (deadlock-риск).
///
/// Windows: .cmd/.bat-бинари (npm, code...) разрешаются через
/// platforms::resolve_command (обёртка cmd /c).
pub async fn piped_run(
    program: &str,
    args: &[String],
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<PipedResult, String> {
    let (program, args) = platforms::resolve_command(program, args);
    let mut cmd = TokioCommand::new(program.as_str());
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Err(format!("Не удалось запустить `{program}`: {e}"));
        }
    };

    let mut readers: tokio::task::JoinSet<(bool, Option<String>)> = tokio::task::JoinSet::new();
    // stdout и stderr читаются параллельно (разные типы потоков).
    if let Some(out) = child.stdout.take() {
        let sink = Arc::clone(sink);
        let task_id = task_id.to_string();
        let tool_id = tool_id.to_string();
        let session_id = session_id.to_string();
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(
                BufReader::new(out),
                sink,
                index,
                total,
                task_id,
                tool_id,
                session_id,
                abort,
            )
            .await
        });
    }
    if let Some(err) = child.stderr.take() {
        let sink = Arc::clone(sink);
        let task_id = task_id.to_string();
        let tool_id = tool_id.to_string();
        let session_id = session_id.to_string();
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(
                BufReader::new(err),
                sink,
                index,
                total,
                task_id,
                tool_id,
                session_id,
                abort,
            )
            .await
        });
    }

    // Флаг отмены опрашивается второстепенной задачей: сам piped_run
    // в это время ждёт читателей, и проверка строго внутри цикла
    // join_next() не сработала бы — join_next блокируется, пока жив
    // процесс (читатели кончатся только вместе с процессом).
    let (abort_tx, mut abort_rx) = tokio::sync::mpsc::channel::<()>(1);
    let abort_watch = Arc::clone(&abort);
    let abort_poller = tokio::spawn(async move {
        loop {
            if abort_watch.load(Ordering::SeqCst) {
                let _ = abort_tx.send(()).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    let mut aborted = false;
    let mut error_line: Option<String> = None;
    loop {
        tokio::select! {
            _ = abort_rx.recv() => {
                aborted = true;
                let _ = child.kill().await;
                break;
            }
            joined = readers.join_next() => {
                match joined {
                    // Читатель завершился: забираем его tc:error-строку.
                    Some(Ok((_, Some(line)))) => error_line = Some(line),
                    Some(_) => {}
                    // все читатели закрылись — процесс завершился
                    None => break,
                }
            }
        }
    }
    abort_poller.abort();

    // Если процесс убит по отмене, дочитываем остатки, чтобы трубы
    // закрылись (иначе ждать child.kill().await затем child.wait() висит).
    while readers.join_next().await.is_some() {}

    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => return Err(format!("Ошибка ожидания процесса: {e}")),
    };
    Ok(PipedResult::from_status(status, aborted, error_line))
}

// ------------------------------------------------------------
// PowerShell
// ------------------------------------------------------------

/// Пишет PS-скрипт во временный файл (избегаем base64-экранок —
/// кодирование в UTF-8 и запуск через -File). Имя уникально для
/// каждого вызова (см. tracked_temp_file) и файл убирается уборкой
/// задания даже при отмене/таймауте.
fn write_script(tool_id: &str, script: &str) -> Result<PathBuf, String> {
    let path = tracked_temp_file(&format!("{tool_id}-script"), ".ps1");
    // UTF-8 BOM (0xEF, 0xBB, 0xBF) is mandatory for PowerShell 5.1 to correctly
    // parse UTF-8 scripts. Without it, it falls back to the system's ANSI codepage (e.g. CP-1251),
    // which misinterprets some UTF-8 bytes (like the em-dash U+2014 'E2 80 94') as smart quotes (0x94 = ”),
    // breaking the script syntax.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&path, &bytes).map_err(|e| format!("Не удалось записать PS-скрипт: {e}"))?;
    Ok(path)
}

/// Выполняет PS-скрипт: тот же стриминг и отмена, что у piped_run.
pub async fn run_tool_script(
    tool_id: &str,
    script: &str,
    task_id_temp: Option<&str>,
    index: usize,
    total: usize,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<PipedResult, String> {
    let path = write_script(tool_id, script)?;
    let args = vec![
        "-NoProfile".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-File".to_string(),
        path.to_string_lossy().into_owned(),
    ];
    let tid = task_id_temp.unwrap_or(tool_id);
    piped_run(
        "powershell",
        &args,
        index,
        total,
        tid,
        tool_id,
        session_id,
        sink,
        abort,
    )
    .await
}

/// Одиночное значение внутри PS-строки: обрамляем кавычками и
/// экранируем апострофы ('' — escape в PowerShell).
pub fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ------------------------------------------------------------
// Скачивание с прогрессом
// ------------------------------------------------------------

/// Лимит на скачивание одного установщика (30 минут) — защита от
/// зависшего сервера; большие инсталляторы (MSVC) в него укладываются.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Жёсткий лимит размера одного скачиваемого установщика (4 ГБ):
/// самый крупный официальный инсталлятор каталога (MSVC Build Tools,
/// Android SDK) укладывается; «бесконечный» поток от сломанного или
/// враждебного сервера обрывается вместо бесконечного роста на диске.
const MAX_DOWNLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Предел автоматических редиректов HttpClient (по умолчанию в .NET — 50):
/// явный и меньший предел. Даунгрейд https→http редиректом блокируется
/// самим HttpClientHandler (политика .NET), http→https разрешён.
const MAX_REDIRECTS: u32 = 5;

/// Проверяет URL источника перед скачиванием: только https (кроме
/// localhost для тестов/оффлайн-стендов). http-скачивание установщика —
/// открытая дверь MITM; file:// и прочие схемы не поддерживаются.
pub fn validate_download_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if lower.starts_with("http://") {
        let host = lower
            .trim_start_matches("http://")
            .split(['/', ':', '?', '#'])
            .next()
            .unwrap_or("");
        if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
            return Ok(());
        }
        return Err(format!(
            "Источник «{url}» использует http без шифрования — скачивание отклонено (требуется https)"
        ));
    }
    Err(format!(
        "Источник «{url}» не является https-ссылкой — скачивание отклонено"
    ))
}

/// Скачивает URL во временный файл. Прогресс уходит как строки
/// `tc:dl <получено> <всего байт>` (всего может быть -1, если не
/// известно) — фронтенд парсит их из TaskProgress.
///
/// Целостность: `expected_sha256` (hex из tools.json) проверяется
/// ПОСЛЕ скачивания и ДО возврата Ok — файл с неверной суммой
/// удаляется, задача падает. None = источник без контрольной суммы:
/// скачивание завершается предупреждением `tc:warn unverified` —
/// честная пометка «целостность не проверялась», а не мнимое
/// «проверено».
/// PowerShell-скрипт скачивания: https-валидация уже прошла; здесь —
/// редиректы (ограничены), таймаут, прогресс tc:dl и ЖЁСТКИЙ лимит
/// размера. Вынесен отдельно, чтобы тесты могли проверить инварианты
/// без сети.
fn download_script(url: &str, dest: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Net.Http
$handler = New-Object System.Net.Http.HttpClientHandler
$handler.MaxAutomaticRedirections = {2}
$client = New-Object System.Net.Http.HttpClient($handler)
$client.Timeout = [TimeSpan]::FromMinutes(30)
$client.DefaultRequestHeaders.Add('User-Agent', 'StackPilot/0.1 (toolchain installer)')
$maxBytes = [long]{3}
try {{
    $resp = $client.GetAsync({0}, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).Result
    if (-not $resp.IsSuccessStatusCode) {{
        Write-Output "tc:error HTTP $([int]$resp.StatusCode) для {0}"
        exit $([int]$resp.StatusCode)
    }}
    $stream = $resp.Content.ReadAsStreamAsync().Result
    $file = [System.IO.File]::Create({1})
    $buffer = New-Object byte[] 262144
    $received = [long]0
    $totalBytes = [long]0
    if ($resp.Content.Headers.ContentLength) {{
        $totalBytes = [long]$resp.Content.Headers.ContentLength
        if ($totalBytes -gt $maxBytes) {{
            Write-Output "tc:error размер источника {0} превышает лимит $maxBytes байт"
            $file.Close()
            $stream.Dispose()
            $resp.Dispose()
            exit 1
        }}
    }}
    $lastPct = -1
    try {{
        while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {{
            $received += $read
            if ($received -gt $maxBytes) {{
                Write-Output "tc:error скачивание {0} превысило лимит $maxBytes байт — прервано"
                exit 1
            }}
            $file.Write($buffer, 0, $read)
            if ($totalBytes -gt 0) {{
                $pct = [int]($received * 100 / $totalBytes)
                if ($pct -ne $lastPct) {{
                    Write-Output "tc:dl $received $totalBytes"
                    $lastPct = $pct
                }}
            }} else {{
                Write-Output "tc:dl $received -1"
            }}
        }}
        Write-Output "tc:dl $received $totalBytes"
    }} finally {{
        $file.Close()
        $stream.Dispose()
        $resp.Dispose()
        $handler.Dispose()
        $client.Dispose()
    }}
    Write-Output "tc:dl done"
}} catch {{
    Write-Output "tc:error $($_.Exception.InnerException.Message)"
    exit 1
}}
"#,
        ps_quote(url),
        ps_quote(&dest.to_string_lossy()),
        MAX_REDIRECTS,
        MAX_DOWNLOAD_BYTES
    )
}

pub async fn download(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<(), String> {
    validate_download_url(url)?;

    let script = download_script(url, dest);

    let result = timeout(
        DOWNLOAD_TIMEOUT,
        run_tool_script(
            tool_id,
            &script,
            Some(task_id),
            index,
            total,
            session_id,
            sink,
            abort,
        ),
    )
    .await
    .map_err(|_| format!("Скачивание {url} превысило лимит времени"))?;

    let res = result?;
    if !res.success {
        return if let Some(line) = res.error_line {
            if line.starts_with("HTTP 404") {
                Err(format!("Ошибка 404 Not Found для {url}"))
            } else {
                Err(format!("Скачивание {url}: {line}"))
            }
        } else {
            Err(format!("Скачивание {url} завершилось с кодом {}", res.code))
        };
    }

    // --- Граница доверия: целостность скачанного файла ---
    match expected_sha256.and_then(super::crypto::normalize_digest) {
        Some(expected) => {
            sink.emit(event(
                ToolchainEventType::TaskProgress {
                    line: "tc:info Проверка SHA-256 скачанного файла…".to_string(),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            let actual = super::crypto::sha256_file_hex(dest)?;
            if actual != expected {
                let _ = std::fs::remove_file(dest);
                return Err(format!(
                    "Контрольная сумма {url} НЕ совпала (ожидался sha256 {expected}, получен {actual}). Файл удалён — установка прервана"
                ));
            }
            sink.emit(event(
                ToolchainEventType::TaskProgress {
                    line: "tc:info SHA-256 совпал — источник проверен".to_string(),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            Ok(())
        }
        None => {
            // Честная пометка: без суммы из каталога мы НЕ можем
            // утверждать, что файл подлинный.
            sink.emit(event(
                ToolchainEventType::TaskProgress {
                    line: "tc:warn unverified: у источника нет контрольной суммы в каталоге — целостность не проверялась".to_string(),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            Ok(())
        }
    }
}

// ------------------------------------------------------------
// Элевация прав (UAC)
// ------------------------------------------------------------

/// Временные файлы, куда Start-Process перенаправит вывод установщика.
/// (RedirectStandardOutput не умеет в pipe — только в файл.)
/// Имена уникальны и зарегистрированы для уборки задания.
fn elevated_logs(tool_id: &str) -> (PathBuf, PathBuf) {
    (
        tracked_temp_file(&format!("{tool_id}-elevated-out"), ".log"),
        tracked_temp_file(&format!("{tool_id}-elevated-err"), ".log"),
    )
}

/// Кавычит один аргумент по правилам командной строки Windows
/// (CommandLineToArgvW): обрамление в двойные кавычки, экранирование
/// внутренних кавычек и обратных слешей перед ними.
///
/// Не используется в run_elevated: оборачивание каждого аргумента
/// в кавычки ломает NSIS-флаги (`/S` → `"/S"` → installer игнорирует).
/// Оставлен для будущих установщиков, которым нужна quote-windows
/// семантика на уровне отдельных аргументов.
#[allow(dead_code)]
fn quote_windows_arg(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => {
                backslashes += 1;
            }
            '"' => {
                // Каждую кавычку предваряем удвоенными слешами + \"
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(c);
            }
        }
    }
    // Слеши перед закрывающей кавычкой удваиваются.
    for _ in 0..backslashes {
        out.push('\\');
        out.push('\\');
    }
    out.push('"');
    out
}

/// C#-раннер: запускает установщик В УЖЕ ПОВЫШЕННОМ процессе (wrapper
/// исполняется после UAC-подтверждения), перенаправляя stdout/stderr
/// в файлы построчно. В отличие от Start-Process -Verb RunAs, здесь
/// вывод реально перехватывается (RunAs не умеет -RedirectStandardOutput),
/// а файлы параллельно стримятся в UI (tail_elevated_logs) — долгие
/// установки (MSVC, .NET SDK) видят живой прогресс, а не «висит».
const ELEVATED_RUNNER_CS: &str = r#"
using System;
using System.IO;
using System.Diagnostics;
using System.Threading.Tasks;
public static class TcElevatedRunner {
    public static int Run(string program, string args, string outFile, string errFile) {
        var psi = new ProcessStartInfo(program, args) {
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true
        };
        using (var p = Process.Start(psi)) {
            if (p == null) return -1;
            Task o = null;
            Task e = null;
            if (!string.IsNullOrEmpty(outFile)) o = Task.Run(() => CopyStream(p.StandardOutput, outFile));
            if (!string.IsNullOrEmpty(errFile)) e = Task.Run(() => CopyStream(p.StandardError, errFile));
            p.WaitForExit();
            try { if (o != null) o.Wait(); } catch { }
            try { if (e != null) e.Wait(); } catch { }
            return p.ExitCode;
        }
    }
    private static void CopyStream(StreamReader reader, string file) {
        using (var w = new StreamWriter(file, true, new System.Text.UTF8Encoding(false)) { AutoFlush = true }) {
            string line;
            while ((line = reader.ReadLine()) != null) w.WriteLine(line);
        }
    }
}
"#;

/// Собирает PS-скрипт-обёртку, который выполняется ПОВЫШЕННЫМ процессом:
/// компилирует C#-раннер (Add-Type), запускает установщик с перенаправлением
/// вывода в файлы и возвращает его код завершения.
fn elevated_wrapper_script(program: &str, arg_shell: &str, out: &Path, err: &Path) -> String {
    format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
{cs}
'@
$code = [TcElevatedRunner]::Run({program}, {args}, {out}, {err})
exit $code
"#,
        cs = ELEVATED_RUNNER_CS.trim(),
        program = ps_quote(program),
        args = ps_quote(arg_shell),
        out = ps_quote(&out.to_string_lossy()),
        err = ps_quote(&err.to_string_lossy()),
    )
}

/// Состояние тайлера: последняя строка tc:error и позиции дочитывания файлов.
#[derive(Default)]
struct TailState {
    error_line: Option<String>,
    positions: (u64, u64),
}

/// Живой стриминг лог-файлов повышённого установщика (tail): файлы
/// опрашиваются каждые 250 мс, новые строки уходят в EventSink как
/// TaskProgress. Возвращает состояние тайлера для финального дочёта.
async fn tail_elevated_logs(
    out: PathBuf,
    err: PathBuf,
    sink: Arc<dyn EventSink>,
    index: usize,
    total: usize,
    task_id: String,
    tool_id: String,
    session_id: String,
    abort: Arc<AtomicBool>,
) -> TailState {
    use std::io::{Read, Seek, SeekFrom};
    let mut state = TailState::default();
    let mut positions: (u64, u64) = (0, 0);
    loop {
        if abort.load(Ordering::SeqCst) {
            state.positions = positions;
            return state;
        }
        for (path, pos) in [(&out, &mut positions.0), (&err, &mut positions.1)] {
            let Ok(mut file) = std::fs::File::open(path) else {
                continue;
            };
            let Ok(len) = file.metadata().map(|m| m.len()) else {
                continue;
            };
            if len < *pos {
                *pos = 0; // файл пересоздан — читаем заново
            }
            if len == *pos {
                continue;
            }
            if file.seek(SeekFrom::Start(*pos)).is_err() {
                continue;
            }
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_err() {
                continue;
            }
            // Позиция — по ФАКТИЧЕСКИ прочитанным байтам, а не по метаданным
            // до чтения: между metadata() и read_to_end файл мог вырасти,
            // и иначе байты между старым и новым концом были бы потеряны.
            *pos += buf.len() as u64;
            let text = String::from_utf8_lossy(&buf);
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(msg) = line.strip_prefix("tc:error ") {
                    state.error_line = Some(msg.to_string());
                }
                sink.emit(event(
                    ToolchainEventType::TaskProgress {
                        line: line.to_string(),
                    },
                    index,
                    total,
                    &task_id,
                    &tool_id,
                    &session_id,
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Дочитывает хвост лог-файла с заданной позиции (гонка между последним
/// тиком тайлера и завершением установщика). Строки уходят в EventSink,
/// tc:error перехватывается.
fn drain_elevated_log(
    path: &Path,
    from: u64,
    sink: &Arc<dyn EventSink>,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    error_line: &mut Option<String>,
) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return;
    };
    if len <= from {
        return;
    }
    if file.seek(SeekFrom::Start(from)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(msg) = line.strip_prefix("tc:error ") {
            *error_line = Some(msg.to_string());
        }
        sink.emit(event(
            ToolchainEventType::TaskProgress {
                line: line.to_string(),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
    }
}

/// Запускает установщик С ПРАВАМИ АДМИНИСТРАТОРА: PowerShell
/// выводит UAC-промпт (пользователь подтверждает — это не тихая
/// установка), процесс ждётся (-Wait). stdout/stderr установщика
/// перехватываются в файлы и СТРИМЯТСЯ В ЖИВУЮ в UI (долгие установки
/// — MSVC Build Tools, .NET SDK — показывают прогресс, а не «висит»).
///
/// Вывод идёт в скрытое окно (CreateNoWindow): консоль установщика
/// больше не отвлекает, прогресс виден в приложении.
///
/// Применимо только на Windows; на других ОС возвращает Err.
pub async fn run_elevated(
    program: &str,
    args: &[String],
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<PipedResult, String> {
    if std::env::consts::OS != "windows" {
        return Err("Запуск с правами администратора поддерживается только на Windows".to_string());
    }

    let (program, args) = platforms::resolve_command(program, args);
    let (out, err) = elevated_logs(tool_id);
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&err);

    // Каждый аргумент — самостоятельная кавычка по правилам
    // CommandLineToArgvW; склеенная строка отдаётся установщику.
    let arg_shell: Vec<String> = args.iter().map(|a| quote_windows_arg(a)).collect();
    let arg_shell = arg_shell.join(" ");

    // Обёртка, исполняемая повышено: компилирует C#-раннер и запускает
    // установщик с перенаправлением вывода в файлы.
    let wrapper = write_script(tool_id, &elevated_wrapper_script(&program, &arg_shell, &out, &err))?;

    // Живой стриминг: пока UAC-процесс работает, файлы выводятся в UI.
    let tail = tokio::spawn(tail_elevated_logs(
        out.clone(),
        err.clone(),
        Arc::clone(sink),
        index,
        total,
        task_id.to_string(),
        tool_id.to_string(),
        session_id.to_string(),
        Arc::clone(&abort),
    ));

    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$wrapper = {}
if (-not (Test-Path -LiteralPath {})) {{
  Write-Output "tc:error не найден: {}"
  Exit 2
}}
try {{
  $p = Start-Process -FilePath 'powershell' -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',$wrapper) -Verb RunAs -Wait -PassThru -WindowStyle Hidden
  Write-Output "tc:uac exit $($p.ExitCode)"
  exit $p.ExitCode
}} catch {{
  Write-Output "tc:error $($_.Exception.Message)"
  Exit 1
}}
"#,
        ps_quote(&wrapper.to_string_lossy()),
        ps_quote(&program),
        ps_quote(&program),
    );

    let res = run_tool_script(
        tool_id,
        &script,
        Some(task_id),
        index,
        total,
        session_id,
        sink,
        abort,
    )
    .await?;

    // Стоп тайлеру и финальный дочёт хвоста (гонка с последним тиком).
    tail.abort();
    let tail_state = tail.await.ok().unwrap_or_default();
    let mut error_line: Option<String> = tail_state.error_line;
    drain_elevated_log(
        &out,
        tail_state.positions.0,
        sink,
        index,
        total,
        task_id,
        tool_id,
        session_id,
        &mut error_line,
    );
    drain_elevated_log(
        &err,
        tail_state.positions.1,
        sink,
        index,
        total,
        task_id,
        tool_id,
        session_id,
        &mut error_line,
    );
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&err);

    let mut res = res;
    if res.error_line.is_none() {
        res.error_line = error_line;
    }
    Ok(res)
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(crate) struct TestSink {
        events: Mutex<Vec<ToolchainEvent>>,
    }

    impl EventSink for TestSink {
        fn emit(&self, event: ToolchainEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn no_abort() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn piped_run_streams_and_succeeds() {
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        let args = vec![
            "/c".to_string(),
            "echo".to_string(),
            "hello-console".to_string(),
        ];
        let res = piped_run(
            "cmd",
            &args,
            0,
            1,
            "t",
            "tool",
            "s-1",
            &trait_sink,
            no_abort(),
        )
        .await
        .unwrap();

        assert!(res.success);
        assert!(!res.aborted);
        let has_line = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(&e.event_type, ToolchainEventType::TaskProgress { line } if line == "hello-console"));
        assert!(has_line, "строка из процесса должна уйти как TaskProgress");
        // события помечены session_id владельца
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .all(|e| e.session_id == "s-1"));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn abort_kills_process() {
        // Процесс «засыпает» на 30 секунд — тест должен оборвать его флагом.
        let abort = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        let args = vec![
            "/c".to_string(),
            "ping".to_string(),
            "-n".to_string(),
            "30".to_string(),
            "127.0.0.1".to_string(),
        ];
        let tool_id = "sleep";
        let task_id = "sleep";

        let res_tok = piped_run(
            "cmd",
            &args,
            0,
            1,
            task_id,
            tool_id,
            "s-abort",
            &trait_sink,
            abort.clone(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
        abort.store(true, Ordering::SeqCst);

        let res = tokio::time::timeout(Duration::from_secs(10), res_tok)
            .await
            .expect("отмена должна завершить piped_run быстро");
        let res = res.unwrap();
        assert!(res.aborted, "процесс должен быть помечен как отменённый");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn piped_run_captures_error_line() {
        // Скрипт печатает tc:error и выходит с ненулевым кодом —
        // причина сбоя должна попасть в PipedResult.error_line.
        // PowerShell (как в проде): Write-Output не добавляет кавычек.
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        let args = vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Write-Output 'tc:error test-boom'; exit 3".to_string(),
        ];
        let res = piped_run(
            "powershell",
            &args,
            0,
            1,
            "t",
            "tool",
            "s-err",
            &trait_sink,
            no_abort(),
        )
        .await
        .unwrap();

        assert!(!res.success);
        assert_eq!(res.code, 3);
        assert_eq!(res.error_line.as_deref(), Some("test-boom"));
    }

    #[test]
    fn ps_quote_escapes_apostrophes() {
        assert_eq!(ps_quote("O'Brien"), "'O''Brien'");
        assert_eq!(ps_quote("plain"), "'plain'");
    }

    #[test]
    fn windows_arg_quoting_matches_commandline_rules() {
        // Простые аргументы — просто в кавычках.
        assert_eq!(quote_windows_arg("quiet"), "\"quiet\"");
        // Внутренние кавычки экранируются \", предшествующие слеши удваиваются.
        assert_eq!(
            quote_windows_arg(r#"--override "--superpassword x""#),
            r#""--override \"--superpassword x\"""#
        );
        // Слеши перед закрывающей кавычкой удваиваются.
        assert_eq!(quote_windows_arg(r"C:\dir\"), r#""C:\dir\\""#);
        // Слеш перед обычным символом не трогается.
        assert_eq!(quote_windows_arg(r"C:\dir\x"), r#""C:\dir\x""#);
    }

    #[test]
    fn download_url_validation_enforces_https() {
        assert!(validate_download_url("https://example.com/setup.exe").is_ok());
        assert!(validate_download_url("http://localhost/x.exe").is_ok());
        assert!(validate_download_url("http://127.0.0.1:8080/x.exe").is_ok());
        assert!(validate_download_url("http://example.com/setup.exe").is_err());
        assert!(validate_download_url("ftp://example.com/setup.exe").is_err());
        assert!(validate_download_url("file:///C:/evil.exe").is_err());
    }

    #[test]
    fn temp_files_are_unique_and_tracked() {
        let a = tracked_temp_file("uniq-test", ".tmp");
        let b = tracked_temp_file("uniq-test", ".tmp");
        assert_ne!(a, b, "временные пути обязаны быть уникальными");
        assert!(a.starts_with(std::env::temp_dir()));
        cleanup_tracked_temp_files();
    }

    /// Крэш посреди установки не может вызвать cleanup_tracked_temp_files:
    /// зачистка осиротевших файлов обязана переживать перезапуск приложения.
    #[test]
    fn sweep_removes_only_old_orphaned_temp_files() {
        let dir = std::env::temp_dir();
        let old = dir.join(format!("tc-sweep-old-{}", std::process::id()));
        let fresh = dir.join(format!("tc-sweep-fresh-{}", std::process::id()));
        std::fs::write(&old, "x").unwrap();
        std::fs::write(&fresh, "x").unwrap();
        // «Осиротевший» файл: метка на два дня назад.
        let two_days_ago = std::time::SystemTime::now() - Duration::from_secs(2 * 86400);
        let handle = std::fs::File::options().write(true).open(&old).unwrap();
        handle.set_modified(two_days_ago).unwrap();
        drop(handle);

        sweep_stale_temp_files();

        assert!(!old.exists(), "старый осиротевший файл обязан быть убран");
        assert!(
            fresh.exists(),
            "свежий файл (другой живой процесс/задание) не трогается"
        );
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&fresh);
    }

    /// Регрессия безопасности: скрипт скачивания обязан нести ЖЁСТКИЙ
    /// лимит размера (бесконечный поток не растёт на диске) и ЯВНЫЙ
    /// предел редиректов (HttpClient по умолчанию разрешает 50).
    #[test]
    fn download_script_carries_size_and_redirect_bounds() {
        let script = download_script(
            "https://example.com/x.exe",
            Path::new(r"C:\temp\tc-x-1.exe"),
        );
        assert!(
            script.contains(&format!("$maxBytes = [long]{MAX_DOWNLOAD_BYTES}")),
            "лимит размера в скрипте"
        );
        assert!(
            script.contains("превысило лимит"),
            "проверка в цикле чтения"
        );
        assert!(
            script.contains("размер источника"),
            "проверка ContentLength"
        );
        assert!(
            script.contains(&format!("MaxAutomaticRedirections = {MAX_REDIRECTS}")),
            "явный предел редиректов"
        );
    }
}
