use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command as TokioCommand;

use crate::modules::project_creator::models::*;

/// Генератор — исполняет один шаг `Step::Generate` пайплайна.
/// Реализации обязаны быть потокобезопасными и не хранить состояние.
#[async_trait]
pub trait Generator: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn generate(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String>;
}

pub struct GeneratorRegistry {
    generators: HashMap<String, Arc<dyn Generator>>,
}

impl GeneratorRegistry {
    pub fn new() -> Self {
        Self {
            generators: HashMap::new(),
        }
    }

    /// Реестр со встроенными генераторами движка.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(CliGenerator));
        registry.register(Arc::new(SpringBootGenerator));
        registry.register(Arc::new(FsCleanupGenerator));
        registry
    }

    pub fn register(&mut self, generator: Arc<dyn Generator>) {
        let id = generator.id().to_string();
        self.generators.insert(id, generator);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Generator>> {
        self.generators.get(id).cloned()
    }

    pub fn list_descriptors(&self) -> Vec<GeneratorDescriptor> {
        self.generators
            .values()
            .map(|g| GeneratorDescriptor {
                id: g.id().to_string(),
                name: g.name().to_string(),
                description: g.description().to_string(),
            })
            .collect()
    }
}

// ============================================================================
// CliGenerator — универсальный CLI-раннер.
//
// Выполняет произвольную команду с точным контролем над рабочей директорией
// и аргументами. Используется шагами, которым нужно переопределить вызов CLI
// (например create-next-app ВНУТРИ frontend/ с аргументом "." вместо имени
// проекта — иначе CLI создаёт вложенную папку, и получается матрёшка).
//
// Конфиг (Step::Generate.generator_config):
//   {
//     "command": "npx",
//     "args": ["create-next-app@latest", ".", "--skip-install"],
//     "working_dir": "frontend",        // относительно project_path; опционально
//     "env": {"CI": "1"},               // опционально
//     "timeout_secs": 600               // опционально, по умолчанию 600
//   }
// ============================================================================
pub struct CliGenerator;

#[async_trait]
impl Generator for CliGenerator {
    fn id(&self) -> &str {
        "cli"
    }
    fn name(&self) -> &str {
        "CLI Command"
    }
    fn description(&self) -> &str {
        "Executes a CLI command to generate project scaffolding"
    }
    async fn generate(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        let command = config
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "CliGenerator: missing 'command' in config".to_string())?;
        let args: Vec<String> = config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let working_dir = config
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(|d| project_path.join(d))
            .unwrap_or_else(|| project_path.to_path_buf());
        let env: Option<HashMap<String, String>> = config
            .get("env")
            .and_then(|e| e.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            });
        let timeout_secs = config
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);

        run_cli(command, &args, &working_dir, env.as_ref(), timeout_secs).await
    }
}

// ============================================================================
// SpringBootGenerator — генерация Spring Boot через Spring Initializr.
//
// Проблема: HTTP GET к start.spring.io возвращает ошибку (например, 400 при
// несовместимых зависимостях), а движок молча писал HTML/JSON-ответ в
// project.zip и падал на распаковке («Error opening archive»). Здесь:
//   1. curl --fail-with-body: при HTTP >= 400 процесс завершается с кодом 22,
//      но ТЕЛО ОШИБКИ сохраняется в project.zip (обычный curl -f его теряет);
//   2. проверка на Rust: project.zip обязан быть настоящим ZIP-архивом
//      (магические байты PK\x03\x04). Если это не ZIP — из тела достаётся
//      реальная причина (JSON-поле message от Initializr) и генерация
//      останавливается с понятной ошибкой.
// ============================================================================
pub struct SpringBootGenerator;

#[async_trait]
impl Generator for SpringBootGenerator {
    fn id(&self) -> &str {
        "spring-boot"
    }
    fn name(&self) -> &str {
        "Spring Initializr"
    }
    fn description(&self) -> &str {
        "Downloads and unpacks a Spring Boot starter from start.spring.io"
    }
    async fn generate(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        let project_name = config
            .get("project_name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| context.project_name.clone())
            .unwrap_or_else(|| "app".to_string());
        let deps = config
            .get("dependencies")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| "web".to_string());

        let url = format!(
            "https://start.spring.io/starter.zip?name={}&groupId=com.example&artifactId={}&dependencies={}",
            urlencode(&project_name),
            urlencode(&project_name),
            urlencode(&deps)
        );
        let zip_path = project_path.join("project.zip");

        // 1) Скачивание. --fail-with-body: тело HTTP-ошибки сохраняется в файл.
        download_starter(&url, &zip_path).await?;

        // 2) Валидация на Rust: настоящий ZIP или тело ошибки Initializr?
        let bytes = std::fs::read(&zip_path).map_err(|e| {
            format!("Spring Initializr error: failed to read downloaded archive 'project.zip': {}", e)
        })?;
        if !is_zip_archive(&bytes) {
            // HTTP-ответ был не архивом, а JSON/HTML с ошибкой — показываем её.
            let reason = extract_error_text(&bytes);
            return Err(format!("Spring Initializr error: {}", reason));
        }

        // 3) Распаковка.
        let zip_str = zip_path.to_string_lossy().to_string();
        let (unzip_cmd, unzip_args) = if cfg!(target_os = "windows") {
            ("tar", vec!["-xf".to_string(), zip_str])
        } else {
            ("unzip", vec!["-o".to_string(), zip_str])
        };
        run_cli(unzip_cmd, &unzip_args, project_path, None, 300).await?;

        // 4) Уборка.
        let _ = std::fs::remove_file(&zip_path);

        Ok(GenerationReport {
            created_files: Vec::new(),
            modified_files: Vec::new(),
            skipped_files: Vec::new(),
            message: format!(
                "Spring Boot project '{}' generated (dependencies: {})",
                project_name, deps
            ),
        })
    }
}

// ============================================================================
// FsCleanupGenerator — программная зачистка файлов/папок из корня проекта.
//
// Используется после `prisma init`: новые версии Prisma разворачивают в
// проекте каталог AI-навыков (.agents/, .claude/, .windsurf/ + skills-lock.json —
// десятки тысяч файлов). Флаг --no-skills есть только в новых версиях CLI,
// поэтому движок дополнительно удаляет артефакты программно.
//
// Защита от удаления пользовательских данных: папки .claude/.windsurf/.agents
// удаляются ТОЛЬКО при наличии маркера skills-lock.json (создаётся самим
// prisma init) — если маркера нет, папки не трогаются.
//
// Конфиг: { "paths": [".agents", ".claude", ".windsurf", "skills-lock.json"] }
// ============================================================================
pub struct FsCleanupGenerator;

#[async_trait]
impl Generator for FsCleanupGenerator {
    fn id(&self) -> &str {
        "fs-cleanup"
    }
    fn name(&self) -> &str {
        "Filesystem cleanup"
    }
    fn description(&self) -> &str {
        "Removes Prisma agent skill artifacts from the project root"
    }
    async fn generate(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        let paths: Vec<String> = config
            .get("paths")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return Ok(GenerationReport {
                created_files: Vec::new(),
                modified_files: Vec::new(),
                skipped_files: Vec::new(),
                message: "Nothing to clean up: no paths configured".into(),
            });
        }

        // Маркер, что артефакты создал именно prisma init.
        let prisma_skills = project_path.join("skills-lock.json").exists();

        let mut removed: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for p in &paths {
            let full = project_path.join(p);
            if full.is_dir() {
                if !prisma_skills {
                    skipped.push(format!("{} (no skills-lock.json marker)", p));
                    continue;
                }
                match std::fs::remove_dir_all(&full) {
                    Ok(_) => removed.push(p.clone()),
                    Err(e) => {
                        return Err(format!("Failed to remove directory '{}': {}", p, e));
                    }
                }
            } else if full.is_file() {
                match std::fs::remove_file(&full) {
                    Ok(_) => removed.push(p.clone()),
                    Err(e) => return Err(format!("Failed to remove file '{}': {}", p, e)),
                }
            } else {
                skipped.push(p.clone());
            }
        }

        let message = if removed.is_empty() {
            "Prisma agent skills not found — nothing to clean up".to_string()
        } else {
            format!("Removed Prisma agent artifacts: {}", removed.join(", "))
        };
        Ok(GenerationReport {
            created_files: Vec::new(),
            modified_files: Vec::new(),
            skipped_files: skipped,
            message,
        })
    }
}

// ============================================================================
// Общие помощники
// ============================================================================

/// Запустить CLI-команду кроссплатформенно (cmd /C на Windows, sh -c на unix)
/// и дождаться завершения с таймаутом. stderr/stdout пишутся в консоль,
/// последние строки попадают в сообщение об ошибке.
async fn run_cli(
    command: &str,
    args: &[String],
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
    timeout_secs: u64,
) -> Result<GenerationReport, String> {
    let mut cmd = spawn_command(command, args);
    cmd.current_dir(working_dir);
    if let Some(env) = env {
        cmd.envs(env);
    }
    // CI=1 заставляет npx/npm/create-* CLI пропускать интерактивные промпты.
    cmd.env("CI", "1");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn command '{}': {}", command, e))?;
    let stdout = child.stdout.take().expect("stdout should be piped");
    let stderr = child.stderr.take().expect("stderr should be piped");
    let out_handle = tokio::spawn(async move { tail_lines(stdout).await });
    let err_handle = tokio::spawn(async move { tail_lines(stderr).await });

    let waited = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            return Err(format!("Command '{}' process error: {}", command, e));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(format!(
                "Command '{}' timed out after {} seconds",
                command, timeout_secs
            ));
        }
    };
    let out_tail = out_handle.await.unwrap_or_default();
    let err_tail = err_handle.await.unwrap_or_default();

    if status.success() {
        return Ok(GenerationReport {
            created_files: Vec::new(),
            modified_files: Vec::new(),
            skipped_files: Vec::new(),
            message: format!("Command '{}' completed successfully", command),
        });
    }

    let mut detail = err_tail;
    if detail.trim().is_empty() {
        detail = out_tail;
    }
    let detail = truncate(detail.trim(), 500);
    Err(format!(
        "Command '{}' failed with exit code {}: {}",
        command,
        status.code().unwrap_or(-1),
        detail
    ))}

/// Скачать starter.zip с start.spring.io через curl.
///
/// `--fail-with-body` (curl >= 7.76): при HTTP >= 400 процесс выходит с
/// кодом 22, но тело ответа (JSON/HTML ошибки) сохраняется в файл — его
/// потом разбирает Rust-сторона и показывает реальную причину.
async fn download_starter(url: &str, zip_path: &Path) -> Result<(), String> {
    let zip_str = zip_path.to_string_lossy().to_string();
    let args: Vec<String> = vec![
        "--fail-with-body".into(),
        "-sSL".into(),
        "--max-time".into(),
        "120".into(),
        url.to_string(),
        "-o".into(),
        zip_str,
    ];
    let mut cmd = spawn_command("curl", &args);
    cmd.current_dir(zip_path.parent().unwrap_or(Path::new(".")));
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn curl: {}", e))?;
    let stderr = child.stderr.take().expect("stderr should be piped");
    let err_handle = tokio::spawn(async move { tail_lines(stderr).await });

    let waited = tokio::time::timeout(Duration::from_secs(180), child.wait()).await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            return Err(format!("Failed to download Spring Boot starter: {}", e));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(
                "Failed to download Spring Boot starter: timed out after 180 seconds".to_string(),
            );
        }
    };
    let stderr_tail = err_handle.await.unwrap_or_default();

    if status.success() {
        return Ok(());
    }

    // HTTP >= 400: --fail-with-body сохранил тело ошибки в project.zip.
    let reason = match std::fs::read(zip_path) {
        Ok(bytes) if !bytes.is_empty() => extract_error_text(&bytes),
        _ => {
            let tail = stderr_tail.trim();
            if tail.is_empty() {
                format!("HTTP request failed (exit code {})", status.code().unwrap_or(-1))
            } else {
                truncate(tail, 500)
            }
        }
    };
    Err(format!("Spring Initializr error: {}", reason))
}

/// Настоящий ZIP-архив? Проверяем магические байты (PK\x03\x04 — обычный
/// архив, PK\x05\x06 — пустой архив). Тело HTTP-ошибки (JSON/HTML) сюда
/// не подходит — это и есть детектор «ответ был не архивом».
fn is_zip_archive(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && (bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06"))
}

/// Извлечь человекочитаемый текст ошибки из тела HTTP-ответа Initializr.
/// JSON: {"status":400,"error":"Bad Request","message":"..."} → строка с
/// причиной; иначе — очищенный сырой текст (HTML и т.п.).
fn extract_error_text(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(0);
        let error = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Bad Request");
        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            if !message.trim().is_empty() {
                return format!("HTTP {} {} — {}", status, error, message);
            }
        }
    }
    // HTML или сырой текст: схлопываем пробелы и управляющие символы.
    let cleaned: String = trimmed
        .chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate(&cleaned, 500).to_string()
}

/// Безопасный обрез строки (по символам, не по байтам).
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut end = 0;
    for (i, _) in text.char_indices() {
        if i >= max_chars {
            break;
        }
        end = i;
    }
    format!("{}…", &text[..end])
}

/// Percent-кодирование для query-параметров URL.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Кроссплатформенный запуск команды (как в executor::run_command).
fn spawn_command(command: &str, args: &[String]) -> TokioCommand {
    use std::env::consts::OS;
    match OS {
        "windows" => {
            let is_powershell = command.starts_with("powershell")
                || command.starts_with("pwsh")
                || command.contains("Get-")
                || command.contains("Set-")
                || command.contains("Invoke-")
                || command.contains("New-");
            if is_powershell {
                let mut cmd = TokioCommand::new("powershell");
                cmd.arg("-Command").arg(command).args(args);
                cmd
            } else {
                let mut cmd = TokioCommand::new("cmd");
                cmd.arg("/C").arg(command).args(args);
                cmd
            }
        }
        _ => {
            let mut shell_cmd = String::from(command);
            for arg in args {
                shell_cmd.push(' ');
                if arg.contains(' ') {
                    shell_cmd.push('"');
                    shell_cmd.push_str(arg);
                    shell_cmd.push('"');
                } else {
                    shell_cmd.push_str(arg);
                }
            }
            let mut cmd = TokioCommand::new("sh");
            cmd.arg("-c").arg(shell_cmd);
            cmd
        }
    }
}

/// Читает поток построчно, печатает в консоль и возвращает последние 8 строк.
async fn tail_lines<R: AsyncRead + Unpin>(reader: R) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut tail: VecDeque<String> = VecDeque::new();
    while let Ok(Some(line)) = lines.next_line().await {
        println!("{}", line);
        tail.push_back(line);
        if tail.len() > 8 {
            tail.pop_front();
        }
    }
    tail.into_iter().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_error_text_parses_initializr_json() {
        let body = br#"{"timestamp":"2026-08-13T00:00:00.000+00:00","status":400,"error":"Bad Request","message":"Invalid project id or dependencies","path":"/starter.zip"}"#;
        let text = extract_error_text(body);
        assert!(text.contains("400"), "статус должен быть в тексте: {text}");
        assert!(
            text.contains("Invalid project id or dependencies"),
            "причина ошибки должна быть в тексте: {text}"
        );
    }

    #[test]
    fn extract_error_text_falls_back_to_raw_html() {
        let body = b"<html><body>Spring Initializr is temporarily down</body></html>";
        let text = extract_error_text(body);
        assert!(text.contains("Spring Initializr is temporarily down"), "{text}");
    }

    #[test]
    fn is_zip_archive_detects_real_and_empty_zips() {
        assert!(is_zip_archive(b"PK\x03\x04whatever"));
        assert!(is_zip_archive(b"PK\x05\x06"));
        assert!(!is_zip_archive(b"<html>error page</html>"));
        assert!(!is_zip_archive(b""));
        assert!(!is_zip_archive(b"PK"));
    }

    #[test]
    fn urlencode_keeps_query_safe_chars() {
        assert_eq!(urlencode("my-app"), "my-app");
        assert_eq!(urlencode("a b"), "a%20b");
    }

    #[test]
    fn cli_generator_rejects_config_without_command() {
        let gen = CliGenerator;
        let ctx = WizardContext::default();
        let result = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &serde_json::json!({})));
        assert!(result.is_err(), "конфиг без command обязан падать");
    }

    fn tokio_test_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(fut)
    }
}