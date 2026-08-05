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
}

impl PipedResult {
    fn from_status(status: std::process::ExitStatus, aborted: bool) -> Self {
        PipedResult {
            success: !aborted && status.success(),
            aborted,
            code: status.code().unwrap_or(-1),
        }
    }
}

// ------------------------------------------------------------
// Стриминг вывода
// ------------------------------------------------------------

/// Читает поток построчно и шлёт каждую непустую строку как
/// TaskProgress. Прерывается по флагу отмены (aborted = true).
async fn stream_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: BufReader<R>,
    sink: Arc<dyn EventSink>,
    index: usize,
    total: usize,
    task_id: String,
    tool_id: String,
    abort: Arc<AtomicBool>,
) -> bool {
    let mut aborted = false;
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
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
    aborted
}

// ------------------------------------------------------------
// Обычный запуск со стримингом
// ------------------------------------------------------------

/// Запускает процесс, стримит его вывод построчно, ждёт завершения.
/// stdout и stderr читаются параллельно (отдельные задачи), чтобы
/// полный stderr-буфер не заблокировал установщик (deadlock-риск).
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
    let mut child = match TokioCommand::new(program)
        .args(args)
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

    let mut readers = tokio::task::JoinSet::new();
    // stdout и stderr читаются параллельно (разные типы потоков, поэтому
    // два отдельных блока и одна общая функция stream_lines).
    let mut stream = |reader: BufReader<tokio::process::ChildStdout>| {
        let sink = Arc::clone(sink);
        let task_id = task_id.to_string();
        let tool_id = tool_id.to_string();
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(reader, sink, index, total, task_id, tool_id, abort).await
        });
    };
    if let Some(out) = child.stdout.take() {
        stream(BufReader::new(out));
    }
    let mut stream_err = |reader: BufReader<tokio::process::ChildStderr>| {
        let sink = Arc::clone(sink);
        let task_id = task_id.to_string();
        let tool_id = tool_id.to_string();
        let abort = Arc::clone(&abort);
        readers.spawn(async move {
            stream_lines(reader, sink, index, total, task_id, tool_id, abort).await
        });
    };
    if let Some(err) = child.stderr.take() {
        stream_err(BufReader::new(err));
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
    loop {
        tokio::select! {
            _ = abort_rx.recv() => {
                aborted = true;
                let _ = child.kill().await;
                break;
            }
            joined = readers.join_next() => {
                if joined.is_none() {
                    // все читатели закрылись — процесс завершился
                    break;
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
    Ok(PipedResult::from_status(status, aborted))
}

// ------------------------------------------------------------
// PowerShell
// ------------------------------------------------------------

/// Пишет PS-скрипт во временный файл (избегаем base64-экранок —
/// кодирование в UTF-8 и запуск через -File).
fn write_script(tool_id: &str, script: &str) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "tc-{tool_id}-{}.ps1",
        std::process::id()
    ));
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
        r#"$wc = New-Object System.Net.WebClient
$wc.DownloadProgressChanged += {{
  param($s, $e)
  if ($e.TotalBytesToReceive -gt 0) {{
    Write-Output "tc:dl $($e.BytesReceived) $($e.TotalBytesToReceive)"
  }} else {{
    Write-Output "tc:dl $($e.BytesReceived) -1"
  }}
}}
$wc.DownloadFile({}, {})
Write-Output "tc:dl done"
"#,
        ps_quote(url),
        ps_quote(&dest.to_string_lossy())
    );

    let result = timeout(
        DOWNLOAD_TIMEOUT,
run_tool_script(tool_id, &script, Some(task_id), index,
 total, sink, abort),
    )
    .await
    .map_err(|_| format!("Скачивание {url} превысило лимит времени"))?;

    let res = result?;
    if res.success {
        Ok(())
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
$p = Start-Process -FilePath {} -ArgumentList $args -Verb RunAs -Wait -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
Write-Output "tc:uac exit $($p.ExitCode)"
"#,
        ps_quote(&out.to_string_lossy()),
        ps_quote(&err.to_string_lossy()),
        ps_quote(&arg_shell),
        ps_quote(program),
    );

    let res = run_tool_script(tool_id, &script, Some(task_id), index, total, sink, abort).await?;

    // Стримим перехваченный вывод установщика.
    for (path, label) in [(out, "stdout"), (err, "stderr")] {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if !line.is_empty() {
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
        let args = vec!["/c".to_string(), "ping".to_string(), "-n".to_string(), "30".to_string(), "127.0.0.1".to_string()];
        let tool_id = "sleep";
        let task_id = "sleep";

        let res_tok = piped_run("cmd", &args, 0, 1, task_id, tool_id, &trait_sink, abort.clone());
        tokio::time::sleep(Duration::from_millis(500)).await;
        abort.store(true, Ordering::SeqCst);

        let res = tokio::time::timeout(Duration::from_secs(10), res_tok)
            .await
            .expect("отмена должна завершить piped_run быстро");
        let res = res.unwrap();
        assert!(res.aborted, "процесс должен быть помечен как отменённый");
    }

    #[test]
    fn ps_quote_escapes_apostrophes() {
        assert_eq!(ps_quote("O'Brien"), "'O''Brien'");
        assert_eq!(ps_quote("plain"), "'plain'");
    }
}