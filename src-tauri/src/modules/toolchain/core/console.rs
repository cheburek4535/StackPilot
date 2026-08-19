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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
) -> ToolchainEvent {
    ToolchainEvent {
        event_type,
        task_index,
        total_tasks: total,
        task_id: task_id.to_string(),
        tool_id: tool_id.to_string(),
        timestamp: timestamp(),
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
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<PipedResult, String> {
    let (program, args) = platforms::resolve_command(program, args);
    let mut child = match TokioCommand::new(program.as_str())
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
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
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(
                BufReader::new(out),
                sink,
                index,
                total,
                task_id,
                tool_id,
                abort,
            )
            .await
        });
    }
    if let Some(err) = child.stderr.take() {
        let sink = Arc::clone(sink);
        let task_id = task_id.to_string();
        let tool_id = tool_id.to_string();
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(
                BufReader::new(err),
                sink,
                index,
                total,
                task_id,
                tool_id,
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
/// кодирование в UTF-8 и запуск через -File).
fn write_script(tool_id: &str, script: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("tc-{tool_id}-{}.ps1", std::process::id()));
    std::fs::write(&path, script).map_err(|e| format!("Не удалось записать PS-скрипт: {e}"))?;
    Ok(path)
}

/// Выполняет PS-скрипт: тот же стриминг и отмена, что у piped_run.
pub async fn run_tool_script(
    tool_id: &str,
    script: &str,
    task_id_temp: Option<&str>,
    index: usize,
    total: usize,
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
    let res = piped_run("powershell", &args, index, total, tid, tool_id, sink, abort).await;
    let _ = std::fs::remove_file(&path);
    res
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

/// Скачивает URL во временный файл. Прогресс уходит как строки
/// `tc:dl <получено> <всего байт>` (всего может быть -1, если не
/// известно) — фронтенд парсит их из TaskProgress.
///
/// Не используем WebClient.DownloadFile: его событие
/// DownloadProgressChanged в PowerShell 5.1 доставляется только
/// ПОСЛЕ завершения синхронного вызова — юзер видел бы «мнимую
/// скачку» без прогресса. Вместо этого читаем поток чанками
/// через HttpClient и выводим tc:dl на каждый чанк.
pub async fn download(
    url: &str,
    dest: &Path,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> Result<(), String> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Net.Http
$client = New-Object System.Net.Http.HttpClient
$client.Timeout = [TimeSpan]::FromMinutes(30)
$client.DefaultRequestHeaders.Add('User-Agent', 'StackPilot/0.1 (toolchain installer)')
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
    }}
    $lastPct = -1
    try {{
        while (($read = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {{
            $file.Write($buffer, 0, $read)
            $received += $read
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
    }}
    Write-Output "tc:dl done"
}} catch {{
    Write-Output "tc:error $($_.Exception.InnerException.Message)"
    exit 1
}}
"#,
        ps_quote(url),
        ps_quote(&dest.to_string_lossy())
    );

    let result = timeout(
        DOWNLOAD_TIMEOUT,
        run_tool_script(tool_id, &script, Some(task_id), index, total, sink, abort),
    )
    .await
    .map_err(|_| format!("Скачивание {url} превысило лимит времени"))?;

    let res = result?;
    if res.success {
        Ok(())
    } else if let Some(line) = res.error_line {
        if line.starts_with("HTTP 404") {
            Err(format!("Ошибка 404 Not Found для {url}"))
        } else {
            Err(format!("Скачивание {url}: {line}"))
        }
    } else {
        Err(format!("Скачивание {url} завершилось с кодом {}", res.code))
    }
}

// ------------------------------------------------------------
// Элевация прав (UAC)
// ------------------------------------------------------------

/// Временные файлы, куда Start-Process перенаправит вывод установщика.
/// (RedirectStandardOutput не умеет в pipe — только в файл.)
fn elevated_logs(tool_id: &str) -> (PathBuf, PathBuf) {
    let pid = std::process::id();
    (
        std::env::temp_dir().join(format!("tc-{tool_id}-{pid}-out.log")),
        std::env::temp_dir().join(format!("tc-{tool_id}-{pid}-err.log")),
    )
}

/// Запускает установщик С ПРАВАМИ АДМИНИСТРАТОРА: PowerShell
/// выводит UAC-промпт (пользователь подтверждает — это не тихая
/// установка), процесс ждётся (-Wait). Его stdout/stderr уходят
/// в файлы, которые стримим после завершения.
///
/// Применимо только на Windows; на других ОС возвращает Err.
pub async fn run_elevated(
    program: &str,
    args: &[String],
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
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

    // Каждый аргумент — отдельная двойная кавычка (как в командной
    // строке), потом склеиваем. Это надёжнее, чем ArgumentList-массив
    // (он не экранирует пробелы внутри аргументов).
    let arg_shell: Vec<String> = args
        .iter()
        .map(|a| format!("\"{}\"", a.replace('"', "`\"")))
        .collect();
    let arg_shell = arg_shell.join(" ");

    let script = format!(
        r#"$out = {}
$err = {}
$args = {}
if (-not (Test-Path -LiteralPath {})) {{
  Write-Output "tc:error не найден: {}"
  Exit 2
}}
try {{
  $p = Start-Process -FilePath {} -ArgumentList $args -Verb RunAs -Wait -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
}} catch {{
  Write-Output "tc:error $($_.Exception.Message)"
  Exit 1
}}
Write-Output "tc:uac exit $($p.ExitCode)"
"#,
        ps_quote(&out.to_string_lossy()),
        ps_quote(&err.to_string_lossy()),
        ps_quote(&arg_shell),
        ps_quote(&program),
        ps_quote(&program),
        ps_quote(&program),
    );

    let res = run_tool_script(tool_id, &script, Some(task_id), index, total, sink, abort).await?;

    // Стримим перехваченный вывод установщика; заодно ловим tc:error.
    let mut error_line: Option<String> = None;
    for (path, label) in [(out, "stdout"), (err, "stderr")] {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    if let Some(msg) = line.strip_prefix("tc:error ") {
                        error_line = Some(msg.to_string());
                    }
                    sink.emit(event(
                        ToolchainEventType::TaskProgress {
                            line: format!("[{label}] {line}"),
                        },
                        index,
                        total,
                        task_id,
                        tool_id,
                    ));
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }

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
        let res = piped_run("cmd", &args, 0, 1, "t", "tool", &trait_sink, no_abort())
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
}
