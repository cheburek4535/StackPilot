// ============================================================
// Безопасные пробы (domain/probe.rs)
// ============================================================
// Единственное место движка домена, где запускаются процессы.
// Гарантии каждой пробы:
//   - каталог-одобренная команда (только probes/health_checks из
//     tools.json; произвольные команды фронтенда невозможны по типам);
//   - таймаут на каждый запуск;
//   - захват stdout/stderr с ЖЁСТКИМ лимитом длины;
//   - санитизация вывода: значения секретных переменных окружения
//     (TOKEN/PASSWORD/KEY/SECRET...) вырезаются;
//   - stdin отключён (проба не может ждать ввода).
//
// Пробы никогда не пишут на диск и не меняют окружение.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::modules::toolchain::core::version;
use crate::modules::toolchain::platforms;

/// Максимум байт на поток вывода (stdout/stderr отдельно).
pub const MAX_OUTPUT_BYTES: usize = 8 * 1024;

/// Таймаут одной пробы по умолчанию (как в discovery: холодный npm/cmd
/// бывает медленным).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Результат безопасной пробы.
#[derive(Debug, Clone)]
pub struct ProbeOutput {
    /// true — процесс завершился с кодом 0.
    pub success: bool,
    pub exit_code: Option<i32>,
    /// Санитизированный stdout (усечён до MAX_OUTPUT_BYTES).
    pub stdout: String,
    /// Санитизированный stderr (усечён до MAX_OUTPUT_BYTES).
    pub stderr: String,
    /// true — не дождались завершения в отведённое время.
    pub timed_out: bool,
    /// true — бинаря нет вообще (чистое отсутствие, не ошибка опроса).
    pub not_found: bool,
    /// Some(описание) — запустить не удалось по иной причине
    /// (права/ошибка ОС): опрос неконclusive.
    pub launch_error: Option<String>,
}

impl ProbeOutput {
    fn empty() -> Self {
        Self {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            not_found: false,
            launch_error: None,
        }
    }
}

/// Имена переменных окружения, значения которых считаются секретами
/// и вырезаются из любого вывода проб (защита от эха инсталляторов).
const SECRET_NAME_MARKERS: [&str; 8] = [
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "KEY",
    "CREDENTIAL",
    "COOKIE",
    "AUTH",
];

fn looks_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_NAME_MARKERS.iter().any(|m| upper.contains(m))
}

/// Значения «секретоподобных» переменных окружения текущего процесса.
/// Собираются один раз на вызов run_probe (дёшево: переменных немного).
fn collect_env_secrets() -> Vec<String> {
    std::env::vars()
        .filter(|(k, v)| looks_secret(k) && v.len() >= 6)
        .map(|(_, v)| v)
        .collect()
}

/// Убирает секреты и усекает строку до лимита. Публично: используется
/// и для health-check detail.
pub fn sanitize_output(raw: &str) -> String {
    let secrets = collect_env_secrets();
    let mut out = raw.to_string();
    for secret in &secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), "***");
        }
    }
    // Управляющие символы (кроме \t) выпрямляем — вывод идёт в UI/логи.
    out.chars()
        .map(|c| if c.is_control() && c != '\t' { ' ' } else { c })
        .collect::<String>()
        .chars()
        .take(MAX_OUTPUT_BYTES)
        .collect()
}

/// Читает поток целиком, сохраняя не более MAX_OUTPUT_BYTES байт.
/// Хвост игнорируется, но чтение продолжается — процесс не должен
/// блокироваться на переполненном пайпе.
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> String {
    let mut buf = vec![0u8; 4096];
    let mut collected: Vec<u8> = Vec::new();
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let remaining = MAX_OUTPUT_BYTES.saturating_sub(collected.len());
                if remaining > 0 {
                    collected.extend_from_slice(&buf[..n.min(remaining)]);
                }
            }
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

/// Запускает каталог-одобренную команду с таймаутом и ограниченным
/// захватом вывода. Потоки читаются параллельными задачами (deadlock-safe),
/// ожидание процесса гоняется с таймаутом.
pub async fn run_probe(program: &str, args: &[String], timeout_after: Duration) -> ProbeOutput {
    let (program, args) = platforms::resolve_command(program, args);

    let mut child = match TokioCommand::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let mut out = ProbeOutput::empty();
            if e.kind() == std::io::ErrorKind::NotFound {
                out.not_found = true;
            } else {
                out.launch_error = Some(e.to_string());
            }
            return out;
        }
    };

    // Каждый поток читается в своей задаче: полный stderr-буфер не
    // блокирует чтение stdout (и наоборот).
    let stdout_task = child
        .stdout
        .take()
        .map(|mut s| tokio::spawn(async move { read_capped(&mut s).await }));
    let stderr_task = child
        .stderr
        .take()
        .map(|mut s| tokio::spawn(async move { read_capped(&mut s).await }));

    let wait_fut = async {
        let stdout_raw = match stdout_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };
        let stderr_raw = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };
        let status = child.wait().await.ok();
        (stdout_raw, stderr_raw, status)
    };

    match tokio::time::timeout(timeout_after, wait_fut).await {
        Ok((stdout_raw, stderr_raw, status)) => ProbeOutput {
            success: status.as_ref().map(|s| s.success()).unwrap_or(false),
            exit_code: status.as_ref().and_then(|s| s.code()),
            stdout: sanitize_output(stdout_raw.trim()),
            stderr: sanitize_output(stderr_raw.trim()),
            timed_out: false,
            not_found: false,
            launch_error: None,
        },
        Err(_) => {
            // Таймаут: убиваем процесс; kill_on_drop подстрахует при drop.
            let _ = child.start_kill();
            let mut out = ProbeOutput::empty();
            out.timed_out = true;
            out
        }
    }
}

// ------------------------------------------------------------
// Разбор версий из шумного вывода
// ------------------------------------------------------------

/// Достаёт версию из сырого вывода пробы. Правила:
///   - берём строки по очереди; строка должна содержать цифру;
///   - parse_version достаёт числа из текста («node v22.12.0» → 22.12.0);
///   - пустой/мусорный вывод → None (Unparseable у вызывающего).
/// Многострочный вывод сокращается до первой разбираемой строки.
pub fn extract_version_token(raw: &str) -> Option<(String, Vec<u32>)> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(parsed) = version::parse_version(trimmed) {
            return Some((trimmed.to_string(), parsed));
        }
    }
    None
}

/// Каноническая строка версии «22.12.0» из компонентов.
pub fn version_string(parts: &[u32]) -> String {
    parts
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn sanitize_replaces_secret_values() {
        // Ставим секретоподобную переменную и проверяем, что её ЗНАЧЕНИЕ
        // вырезается из произвольного вывода (эхо инсталлятора).
        let name = "TC_TEST_SECRET_VALUE_XYZ";
        std::env::set_var(name, "super-secret-value-123456");
        let sanitized = sanitize_output("token=super-secret-value-123456 done");
        assert!(
            sanitized.contains("***"),
            "секрет должен быть вырезан: {sanitized}"
        );
        assert!(!sanitized.contains("super-secret-value-123456"));
        std::env::remove_var(name);
    }

    #[test]
    fn sanitize_truncates_long_output() {
        let long = "x".repeat(MAX_OUTPUT_BYTES * 3);
        let sanitized = sanitize_output(&long);
        assert!(
            sanitized.len() <= MAX_OUTPUT_BYTES,
            "вывод обязан быть ограничен"
        );
    }

    #[test]
    fn sanitize_keeps_normal_output() {
        assert_eq!(sanitize_output("v1.2.3\nok"), "v1.2.3 ok");
    }

    #[test]
    fn extract_version_from_noisy_line() {
        let (raw, parsed) = extract_version_token("psql (PostgreSQL) 17.2").unwrap();
        assert_eq!(parsed, vec![17, 2]);
        assert_eq!(raw, "psql (PostgreSQL) 17.2");
    }

    #[test]
    fn malformed_version_is_none_not_panic() {
        assert!(extract_version_token("").is_none());
        assert!(extract_version_token("no digits here").is_none());
        // Цифры есть, но это мусор — parse_version вернёт что-то числовое:
        // главное — не паника и детерминированный результат.
        let _ = extract_version_token("build 00000042 rev zzz");
    }

    #[test]
    fn malformed_multiline_takes_first_parseable() {
        let (raw, parsed) =
            extract_version_token("warning: blah\ngit version 2.45.1\nextra").unwrap();
        assert_eq!(parsed, vec![2, 45, 1]);
        assert_eq!(raw, "git version 2.45.1");
    }

    #[test]
    fn version_string_formats_components() {
        assert_eq!(version_string(&[22, 12, 0]), "22.12.0");
        assert_eq!(version_string(&[3]), "3");
    }

    #[tokio::test]
    async fn missing_binary_launch_failed_cleanly() {
        let out = run_probe("definitely-no-such-binary-xyz", &[], Duration::from_secs(5)).await;
        assert!(
            out.not_found,
            "отсутствие бинаря — чистый промах, не ошибка"
        );
        assert!(out.launch_error.is_none());
        assert!(!out.success);
        assert!(!out.timed_out);
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn echo_probe_returns_sanitized_stdout() {
        let out = run_probe(
            "cmd",
            &["/c".to_string(), "echo".to_string(), "1.2.3".to_string()],
            Duration::from_secs(10),
        )
        .await;
        assert!(out.success);
        assert_eq!(out.stdout, "1.2.3");
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn failing_command_reports_nonzero_exit() {
        let out = run_probe(
            "cmd",
            &[
                "/c".to_string(),
                "exit".to_string(),
                "/b".to_string(),
                "7".to_string(),
            ],
            Duration::from_secs(10),
        )
        .await;
        assert!(!out.success);
        assert_eq!(out.exit_code, Some(7));
    }

    #[tokio::test]
    async fn timeout_is_reported_and_process_killed() {
        // Спим заведомо дольше таймаута: проба обязана вернуть timed_out
        // быстро, а процесс — быть убитым (иначе тест висел бы).
        let started = Instant::now();
        // ping сам по себе живёт ~N секунд: -n/-c задают число попыток.
        #[cfg(target_os = "windows")]
        let args = vec!["-n".to_string(), "30".to_string(), "127.0.0.1".to_string()];
        #[cfg(not(target_os = "windows"))]
        let args = vec!["-c".to_string(), "30".to_string(), "127.0.0.1".to_string()];

        let out = run_probe("ping", &args, Duration::from_millis(700)).await;
        assert!(out.timed_out, "проба должна упереться в таймаут");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "таймаут обязан сработать быстро, а не через полный сон команды"
        );
    }
}
