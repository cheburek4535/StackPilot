use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::modules::project_creator::engine::content;
use crate::modules::project_creator::engine::process::{
    command_display, ExecutionEventSink, InteractiveRules, ProcessErrorKind, ProcessOutput,
    ProcessRunner, ProcessSpec, StdinMode,
};
use crate::modules::project_creator::models::*;

/// Плейсхолдер в args, на месте которого ScaffoldGenerator подставляет имя
/// создаваемой CLI-папки ("." в dot-режиме или временную папку в temp+move).
pub const SCAFFOLD_TARGET: &str = "__TARGET__";

/// Генератор — исполняет один шаг `Step::Generate` пайплайна.
/// Реализации обязаны быть потокобезопасными и не хранить состояние.
#[async_trait]
pub trait Generator: Send + Sync {
    fn id(&self) -> &str;
#[allow(dead_code)]
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// Без событийного приёмника: вывод CLI-команд не стримится в UI
    /// (используется тестами и внешними вызывающими).
    async fn generate(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        self.generate_with_sink(context, project_path, config, None)
            .await
    }

    /// Основная точка входа движка: `sink` маршрутизирует stdout/stderr
    /// запускаемых процессов в ExecutionEvent — тот же канал, что у
    /// Step::Command. По умолчанию — без приёмника (совместимость).
    async fn generate_with_sink(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        let _ = sink;
        self.generate(context, project_path, config).await
    }
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
        registry.register(Arc::new(ManifestCheckGenerator));
        registry.register(Arc::new(HostToolCheckGenerator));
        registry.register(Arc::new(PythonVenvGenerator));
        registry.register(Arc::new(DotnetMauiEnsureGenerator));
        registry.register(Arc::new(ScaffoldGenerator));
        registry.register(Arc::new(TauriConfigGenerator));
        registry.register(Arc::new(VsCodeMergeGenerator));
        registry.register(Arc::new(VsCodeFoldersMergeGenerator));
        registry
    }

    pub fn register(&mut self, generator: Arc<dyn Generator>) {
        let id = generator.id().to_string();
        self.generators.insert(id, generator);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Generator>> {
        self.generators.get(id).cloned()
    }

#[allow(dead_code)]
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

/// Запустить CLI-команду через общий ProcessRunner (кроссплатформенный
/// cmd/PowerShell/sh-хендлинг, таймаут с kill, хвосты в ошибках) и
/// преобразовать результат в GenerationReport или отформатированную ошибку.
/// stdout/stderr стримятся в UI через `sink`, если он передан.
/// `interactive` переводит stdin в piped-режим с детекцией триггеров.
async fn run_cli_process(
    command: &str,
    args: &[String],
    working_dir: &Path,
    env: Option<&HashMap<String, String>>,
    timeout_secs: u64,
    interactive: &[InteractiveEntry],
    sink: Option<&ExecutionEventSink>,
) -> Result<GenerationReport, String> {
    // Не модифицируем аргументы рецепта: отдельные CLI имеют собственный
    // синтаксис и сами явно объявляют `--yes`, если он им нужен.
    let stdin = if interactive.is_empty() {
        StdinMode::Null
    } else {
        StdinMode::Piped(InteractiveRules {
            entries: interactive
                .iter()
                .map(|e| (e.trigger.clone(), e.response_type.clone()))
                .collect(),
        })
    };
    let spec = ProcessSpec {
        command: command.to_string(),
        args: args.to_vec(),
        working_dir: Some(working_dir.to_path_buf()),
        env: env.cloned(),
        timeout: Some(Duration::from_secs(timeout_secs)),
        stdin,
        // CI=1 заставляет npx/npm/create-* CLI пропускать интерактивные
        // промпты; NPM_CONFIG_YES/npm_config_yes отвечают «да» на
        // подтверждение установки пакета через npx.
        ci_mode: true,
    };

    match ProcessRunner::run(spec, sink).await {
        Ok(_) => Ok(GenerationReport::success(format!(
            "Command '{}' completed successfully",
            command_display(command, args)
        ))),
        Err(error) => Err(error.format_command_error()),
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
//     "timeout_secs": 600,              // опционально, по умолчанию 600
//     "interactive": [                  // опционально: piped-stdin ответы
//       {"trigger": "Which package manager", "response_type": {"Text": "npm"}}
//     ]
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
    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
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
        let env: Option<HashMap<String, String>> =
            config.get("env").and_then(|e| e.as_object()).map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            });
        let timeout_secs = config
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);
        let interactive: Vec<InteractiveEntry> = config
            .get("interactive")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(parse_interactive_entry).collect())
            .unwrap_or_default();

        run_cli_process(
            command,
            &args,
            &working_dir,
            env.as_ref(),
            timeout_secs,
            &interactive,
            sink,
        )
        .await
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
//   2. проверка HTTP-статуса на Rust: project.zip обязан быть настоящим
//      ZIP-архивом (магические байты PK\x03\x04). Если это не ZIP — из тела
//      достаётся реальная причина (JSON-поле message от Initializr) и
//      генерация прерывается с «Spring Initializr отказал: <message>» —
//      сломанный архив не сохраняется и не распаковывается.
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
    async fn generate_with_sink(
        &self,
        context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
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

        let url = spring_starter_url(&project_name, &deps);
        // При сегментации (Strict Subdir Mandate) starter распаковывается
        // ВНУТРИ сегмента (backend/), а не в корне проекта.
        let target_dir = config
            .get("target_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        let target = if target_dir == "." {
            project_path.to_path_buf()
        } else {
            let target = project_path.join(target_dir);
            std::fs::create_dir_all(&target).map_err(|e| {
                format!(
                    "SpringBootGenerator: failed to create target dir '{}': {}",
                    target_dir, e
                )
            })?;
            target
        };
        let zip_path = target.join("project.zip");

        // 1) Скачивание. --fail-with-body: тело HTTP-ошибки сохраняется в файл.
        download_starter(&url, &zip_path, sink).await?;

        // 2) Проверка HTTP-статуса на Rust: настоящий ZIP или тело ошибки
        //    Initializr? Если ответ не архив (JSON с ключом message, HTML) —
        //    генерация прерывается, сломанный архив не сохраняется.
        let bytes = std::fs::read(&zip_path).map_err(|e| {
            format!(
                "Spring Initializr error: failed to read downloaded archive 'project.zip': {}",
                e
            )
        })?;
        if !is_zip_archive(&bytes) {
            let reason = extract_error_text(&bytes);
            let _ = std::fs::remove_file(&zip_path);
            return Err(format!("Spring Initializr error: {}", reason));
        }

        // 3) Распаковка (внутри сегмента, если он есть).
        let zip_str = zip_path.to_string_lossy().to_string();
        let (unzip_cmd, unzip_args) = if cfg!(target_os = "windows") {
            ("tar", vec!["-xf".to_string(), zip_str])
        } else {
            ("unzip", vec!["-o".to_string(), zip_str])
        };
        run_cli_process(unzip_cmd, &unzip_args, &target, None, 300, &[], sink).await?;

        // Spring Initializr archives are commonly wrapped in a single
        // `<artifactId>/` directory.  Our layout contract expects the
        // project files directly in the selected target (backend/ or root),
        // otherwise Docker cannot find pom.xml/mvnw and DevLauncher starts
        // from an empty directory.  Flatten only when the target itself does
        // not already contain a project manifest.
        if !target.join("pom.xml").is_file() {
            let nested = std::fs::read_dir(&target).ok().and_then(|entries| {
                let dirs: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .filter_map(|e| e.file_type().ok().filter(|t| t.is_dir()).map(|_| e.path()))
                    .collect();
                (dirs.len() == 1).then(|| {
                    dirs.into_iter()
                        .next()
                        .expect("dirs.len() == 1 guarantees exactly one element")
                })
            });
            if let Some(nested) = nested {
                for entry in std::fs::read_dir(&nested).map_err(|e| {
                    format!("SpringBootGenerator: failed to read nested archive directory: {e}")
                })? {
                    let entry = entry.map_err(|e| {
                        format!("SpringBootGenerator: failed to inspect archive output: {e}")
                    })?;
                    let destination = target.join(entry.file_name());
                    if destination.exists() {
                        continue;
                    }
                    std::fs::rename(entry.path(), destination).map_err(|e| {
                        format!("SpringBootGenerator: failed to flatten archive: {e}")
                    })?;
                }
                let _ = std::fs::remove_dir(&nested);
            }
        }

        // 4) Уборка.
        let _ = std::fs::remove_file(&zip_path);

        Ok(GenerationReport::success(format!(
            "Spring Boot project '{}' generated (dependencies: {})",
            project_name, deps
        )))
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
            return Ok(GenerationReport::success(
                "Nothing to clean up: no paths configured",
            ));
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
            ..GenerationReport::success(message)
        })
    }
}

// ============================================================================
// ManifestCheckGenerator — пост-валидация манифестов проекта.
//
// Подтверждает, что каркас реально создал валидный манифест с обязательными
// зависимостями фреймворка/инструмента, а не «generic-заглушку». Поддерживает
// package.json (dependencies + devDependencies + optionalDependencies),
// composer.json (require + require-dev), requirements.txt, pyproject.toml
// (поиск имён зависимостей в строковых литералах) и Cargo.toml
// ([dependencies] / [dev-dependencies]).
//
// Конфиг:
//   {
//     "path": "frontend/package.json",   // относительно корня проекта
//     "kind": "package_json",            // package_json | composer_json |
//                                        // requirements_txt | pyproject_toml | cargo_toml
//     "required_dependencies": ["express"]
//   }
// ============================================================================
pub struct ManifestCheckGenerator;

#[async_trait]
impl Generator for ManifestCheckGenerator {
    fn id(&self) -> &str {
        "manifest-check"
    }
    fn name(&self) -> &str {
        "Manifest validation"
    }
    fn description(&self) -> &str {
        "Validates project manifests (package.json, composer.json, requirements.txt, pyproject.toml, Cargo.toml) and required dependencies"
    }
    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        let path = config
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "ManifestCheckGenerator: missing 'path' in config".to_string())?;
        let kind = config
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("package_json");
        let required: Vec<String> = config
            .get("required_dependencies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let full =
            crate::modules::project_creator::engine::paths::resolve_in_root(project_path, path)
                .ok_or_else(|| {
                    format!("ManifestCheckGenerator: path '{path}' escapes the project root")
                })?;
        let content = std::fs::read_to_string(&full)
            .map_err(|e| format!("ManifestCheckGenerator: cannot read '{path}': {e}"))?;

        let declared = manifest_dependencies(kind, &content)
            .map_err(|e| format!("ManifestCheckGenerator: {path}: {e}"))?;

        let missing: Vec<&String> = required
            .iter()
            .filter(|r| !declared.iter().any(|d| d == *r))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "manifest validation failed for {path}: missing required dependencies: {}. Declared: {}",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if declared.is_empty() {
                    "<none>".to_string()
                } else {
                    declared.join(", ")
                }
            ));
        }

        if let Some(sink) = sink {
            sink.emit_stdout(&format!(
                "manifest {path} is valid: declared {}",
                declared.join(", ")
            ))
            .await;
        }
        Ok(GenerationReport::success(format!(
            "Manifest '{path}' validated (required: {})",
            required.join(", ")
        )))
    }
}

/// Имена зависимостей, задекларированных в манифесте по его виду.
fn manifest_dependencies(kind: &str, content: &str) -> Result<Vec<String>, String> {
    match kind {
        "package_json" | "composer_json" => {
            let value: serde_json::Value =
                serde_json::from_str(content).map_err(|e| format!("invalid JSON: {e}"))?;
            let obj = value
                .as_object()
                .ok_or_else(|| "not a JSON object".to_string())?;
            let mut names: Vec<String> = Vec::new();
            let sections: &[&str] = if kind == "package_json" {
                &["dependencies", "devDependencies", "optionalDependencies"]
            } else {
                &["require", "require-dev"]
            };
            for section in sections {
                if let Some(deps) = obj.get(*section).and_then(|d| d.as_object()) {
                    for key in deps.keys() {
                        if !names.contains(key) {
                            names.push(key.clone());
                        }
                    }
                }
            }
            Ok(names)
        }
        "requirements_txt" => {
            let mut names: Vec<String> = Vec::new();
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty()
                    || line.starts_with('#')
                    || line.starts_with('-')
                    || line.contains("://")
                {
                    continue;
                }
                let name = line
                    .split(['[', '=', '<', '>', '~', '!', ';', ' ', '\t'])
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
            Ok(names)
        }
        "pyproject_toml" => {
            // Практичная проверка: имя зависимости должно встречаться как
            // строковый литерал в списке зависимостей [project].
            let mut names: Vec<String> = Vec::new();
            let mut in_deps = false;
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("dependencies") && line.contains('=') {
                    in_deps = true;
                }
                if in_deps {
                    for token in line.split(',').map(|t| t.trim()) {
                        let name = token
                            .trim_matches(|c| c == '"' || c == '\'' || c == '[' || c == ']')
                            .split(['[', '=', '<', '>', '~', '!'])
                            .next()
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        if !name.is_empty()
                            && name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
                            && !names.contains(&name)
                        {
                            names.push(name);
                        }
                    }
                    if line.starts_with(']') {
                        in_deps = false;
                    }
                }
            }
            Ok(names)
        }
        "cargo_toml" => {
            let mut names: Vec<String> = Vec::new();
            let mut in_deps = false;
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_deps = line == "[dependencies]" || line == "[dev-dependencies]";
                    continue;
                }
                if !in_deps || line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let name = line.split('=').next().unwrap_or("").trim();
                let name = name.trim_matches('"');
                if !name.is_empty() && !names.contains(&name.to_string()) {
                    names.push(name.to_string());
                }
            }
            Ok(names)
        }
        // Валидация «файловых» манифестов без структурированного синтаксиса
        // зависимостей (build.gradle.kts, .csproj, dbt_project.yml, main.tf,
        // firebase.json): «декларацией» считается наличие конкретного токена,
        // который вызывающая сторона передаёт в required_dependencies ТОЧНО так,
        // как он выглядит в реальном файле.
        "gradle_kts" => {
            // Строковые литералы деклараций: id("..."), implementation("..."), ...
            // Версия координаты (цифры или $интерполяция) отбрасывается:
            // "io.ktor:ktor-server-core:3.0.3" -> "io.ktor:ktor-server-core".
            let mut names: Vec<String> = Vec::new();
            for line in content.lines() {
                let mut rest = line;
                while let Some(start) = rest.find('"') {
                    let after = &rest[start + 1..];
                    let Some(end) = after.find('"') else { break };
                    let literal = &after[..end];
                    rest = &after[end + 1..];
                    let token: Vec<&str> = literal
                        .split(':')
                        .filter(|part| {
                            !part.is_empty()
                                && !part.starts_with('$')
                                && !part.chars().next().is_some_and(|c| c.is_ascii_digit())
                        })
                        .collect();
                    let token = token.join(":");
                    if !token.is_empty() && !names.contains(&token) {
                        names.push(token);
                    }
                }
            }
            Ok(names)
        }
        "csproj_xml" => {
            // Include-атрибуты PackageReference: <PackageReference Include="X" .../>.
            let mut names: Vec<String> = Vec::new();
            for line in content.lines() {
                let mut rest = line;
                while let Some(start) = rest.find("Include=\"") {
                    let after = &rest[start + "Include=\"".len()..];
                    let Some(end) = after.find('"') else { break };
                    let name = after[..end].to_string();
                    rest = &after[end + 1..];
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            Ok(names)
        }
        "yaml" => {
            // Ключи верхнего уровня: "name: ..." -> "name". Комментарии и
            // пустые строки игнорируются.
            let mut names: Vec<String> = Vec::new();
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some(key) = line.split(':').next() else {
                    continue;
                };
                let key = key.trim();
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    && !names.contains(&key.to_string())
                {
                    names.push(key.to_string());
                }
            }
            Ok(names)
        }
        "terraform" => {
            // Блоки провайдеров: `provider "docker" {` -> `provider "docker"`.
            let mut names: Vec<String> = Vec::new();
            for line in content.lines() {
                let line = line.trim();
                if !line.starts_with("provider ") {
                    continue;
                }
                let Some(start) = line.find('"') else {
                    continue;
                };
                let after = &line[start + 1..];
                let Some(end) = after.find('"') else { continue };
                let name = format!("provider \"{}\"", &after[..end]);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            Ok(names)
        }
        "firebase" => {
            // Ключи верхнего уровня firebase.json.
            let mut names: Vec<String> = Vec::new();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(obj) = value.as_object() {
                    for key in obj.keys() {
                        if !names.contains(key) {
                            names.push(key.clone());
                        }
                    }
                }
            }
            Ok(names)
        }
        _ => Err(format!("unsupported manifest kind '{kind}'")),
    }
}

// ============================================================================
// HostToolCheckGenerator — generic-префлайт инструментов хоста.
//
// Единая проверка обязательного инструментария (dart, cargo, go, dotnet,
// zig, mix, maven, gradle...) ДО любых скаффолдов, вместо пер-фреймворковых
// патчей. Инструмент ищется через `where` (Windows) / `which` (Unix);
// отсутствие обязательного инструмента останавливает пайплайн (Abort) с
// понятной причиной, а не валит скаффолд серединой пайплайна.
//
// Конфиг (Step::Generate.generator_config):
//   {
//     "tools": ["maven", "gradle"],   // имена инструментов в PATH
//     "mode": "any" | "all",          // any: нужен хотя бы один (альтернативы
//                                     // required_tools: maven ИЛИ gradle);
//                                     // all: нужны все (языковые скаффолды)
//   }
// ============================================================================
pub struct HostToolCheckGenerator;

#[async_trait]
impl Generator for HostToolCheckGenerator {
    fn id(&self) -> &str {
        "host-tool-check"
    }
    fn name(&self) -> &str {
        "Host toolchain check"
    }
    fn description(&self) -> &str {
        "Verifies required host tools (dart, cargo, go, dotnet, zig, maven, gradle...) via where/which"
    }
    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        _project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        let tools: Vec<String> = config
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if tools.is_empty() {
            return Err("HostToolCheckGenerator: missing 'tools' in config".to_string());
        }
        let mode_all = config
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|m| m == "all")
            .unwrap_or(false);

        let finder = if std::env::consts::OS == "windows" {
            "where"
        } else {
            "which"
        };
        let mut found: Vec<String> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for tool in &tools {
            let spec = ProcessSpec {
                command: finder.to_string(),
                args: vec![tool.clone()],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(15)),
                stdin: StdinMode::Null,
                ci_mode: false,
            };
            match ProcessRunner::run(spec, None).await {
                Ok(output) if !output.stdout_tail.trim().is_empty() => found.push(tool.clone()),
                _ => missing.push(tool.clone()),
            }
        }
        if mode_all && !missing.is_empty() {
            return Err(format!(
                "Missing required host tools: {}. Install them and add to PATH — generation cannot proceed.",
                missing.join(", ")
            ));
        }
        if !mode_all && found.is_empty() {
            return Err(format!(
                "Missing required host tool: none of [{}] found in PATH (checked via {finder}). Install one of them and retry — generation cannot proceed.",
                tools.join(", ")
            ));
        }
        if let Some(sink) = sink {
            let status = if mode_all {
                format!("all present: {}", found.join(", "))
            } else {
                format!(
                    "found: {} (alternatives: {})",
                    found.join(", "),
                    tools.join(", ")
                )
            };
            sink.emit_stdout(&format!("host toolchain check: {status}"))
                .await;
        }
        Ok(GenerationReport::success(format!(
            "Host toolchain verified ({}): {}",
            if mode_all { "required" } else { "alternative" },
            found.join(", ")
        )))
    }
}

// ============================================================================
// PythonVenvGenerator — каноническое venv-окружение Python-проекта.
//
// Проблема (Debian/Ubuntu и производные): дистрибутивный python3 без пакета
// python3-venv не содержит модуль ensurepip, поэтому `python3 -m venv <dir>`
// падает с «The virtual environment was not created successfully because
// ensurepip is not available», оставляя ПОЛУСОЗДАННОЕ окружение: pyvenv.cfg
// есть, pip нет. Повторный запуск рецепта не исправлялся: условие
// FileNotExists по маркеру пропускало создание, а pip-шаги падали на
// «No module named pip».
//
// Генератор идемпотентен и доводит окружение до рабочего состояния:
//   0. здоровое окружение (маркер + работающий `python -m pip`) —
//      переиспользуется (повторный запуск рецепта ничего не пересоздаёт);
//   1. `python -m venv` — штатный путь (ensurepip доступен, большинство
//      дистрибутивов и python.org-сборки);
//   2. `python -m virtualenv` — если virtualenv установлен: он несёт
//      собственные wheel'ы pip и работает без сети и без python3-venv;
//   3. `python -m venv --without-pip` + bootstrap pip:
//      a. ensurepip интерпретатора venv;
//      b. системный pip: `python -m pip install --python <venv-python> pip`
//         (pip 22.3+ умеет ставить пакеты в чужое окружение);
//      c. колесо pip из каталога дистрибутива (sysconfig WHEEL_PKG_DIR,
//         Debian/Ubuntu: /usr/share/python-wheels) — офлайн-путь;
//      d. get-pip.py (curl/wget) — последний шанс, требует сеть.
//   4. финальная проверка `python -m pip --version`; провал — Err с точными
//      инструкциями (apt install python3-venv) и диагностикой всех попыток.
//
// Каталог venv пересоздаётся с `--clear` ТОЛЬКО когда в нём уже лежит venv
// (pyvenv.cfg): чужая непустая папка с именем venv не очищается.
//
// Конфиг (Step::Generate.generator_config):
//   { "python": "python3", "venv_path": "/abs/path/to/venv" }
// ============================================================================
pub struct PythonVenvGenerator;

/// Таймауты: создание venv, установка/бутстрап pip.
const VENV_CREATE_TIMEOUT_SECS: u64 = 300;
const PIP_BOOTSTRAP_TIMEOUT_SECS: u64 = 600;
const PROBE_TIMEOUT_SECS: u64 = 30;

#[async_trait]
impl Generator for PythonVenvGenerator {
    fn id(&self) -> &str {
        "python-venv"
    }
    fn name(&self) -> &str {
        "Python virtual environment"
    }
    fn description(&self) -> &str {
        "Ensures the project venv exists with a working pip — recreates it and bootstraps pip when the distro python has no ensurepip (python3-venv)"
    }

    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        let python = config
            .get("python")
            .and_then(|v| v.as_str())
            .unwrap_or(if cfg!(target_os = "windows") {
                "python"
            } else {
                "python3"
            })
            .to_string();
        let venv_path = config
            .get("venv_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "PythonVenvGenerator: missing 'venv_path' in config".to_string())?;
        // Относительный project_path допустим (превью/CLI может прислать
        // "."): приводим оба пути к абсолютным от текущего каталога —
        // ровно так же их разрешает executor (working_dir процесса).
        let absolute = |p: &Path| -> Result<PathBuf, String> {
            if p.is_absolute() {
                Ok(p.to_path_buf())
            } else {
                std::path::absolute(p).map_err(|e| {
                    format!("PythonVenvGenerator: cannot resolve '{}': {e}", p.display())
                })
            }
        };
        let project_abs = absolute(project_path)?;
        let venv = absolute(Path::new(venv_path))?;
        // Окружение обязано лежать внутри проекта (venv_abs вычисляется от
        // project_path; проверка — защита от битого конфига).
        if !venv.starts_with(&project_abs) {
            return Err(format!(
                "PythonVenvGenerator: venv_path '{}' is outside the project root '{}'",
                venv.display(),
                project_abs.display()
            ));
        }
        let venv_path = venv.to_string_lossy().into_owned();

        let venv_python = python_venv_binary(&venv);
        let venv_python_str = venv_python.to_string_lossy().into_owned();
        let mut notes: Vec<String> = Vec::new();

        // 0. Здоровое окружение — переиспользуем (идемпотентность рецепта).
        if venv.join("pyvenv.cfg").is_file() {
            if let Some(version) =
                quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path).await
            {
                let message = format!(
                    "Reused the existing virtual environment at {} ({})",
                    venv.display(),
                    version.trim()
                );
                emit_venv_note(sink, &message).await;
                return Ok(GenerationReport::success(message));
            }
            let note = format!(
                "the existing environment at {} is incomplete (no working pip) — recreating it",
                venv.display()
            );
            emit_venv_note(sink, &note).await;
            notes.push(note);
        }

        // 1. Штатный `python -m venv` (у интерпретатора есть ensurepip).
        if quiet_ok(&python, &["-c", "import ensurepip"], project_path).await {
            match run_venv_command(
                &python,
                &venv_create_args(&venv, &venv_path, false),
                project_path,
                None,
                VENV_CREATE_TIMEOUT_SECS,
                sink,
            )
            .await
            {
                Ok(_) => {
                    if let Some(version) =
                        quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path)
                            .await
                    {
                        return Ok(venv_success(sink, &venv, "python -m venv", &version, notes)
                            .await);
                    }
                    notes.push(
                        "`python -m venv` completed but pip is unavailable in the created environment"
                            .to_string(),
                    );
                }
                Err(error) => notes.push(format!("`{python} -m venv` failed: {error}")),
            }
        } else {
            let note = "the interpreter has no ensurepip module (on Debian/Ubuntu the python3-venv package is not installed) — bootstrapping pip without it"
                .to_string();
            if let Some(sink) = sink {
                sink.emit_stdout(&format!(
                    "Python interpreter '{python}': {note}"
                ))
                .await;
            }
            notes.push(note);
        }

        // 2. virtualenv: собственные wheel'ы pip, офлайн, без ensurepip.
        if quiet_ok(&python, &["-m", "virtualenv", "--version"], project_path).await {
            match run_venv_command(
                &python,
                &venv_virtualenv_args(&venv, &venv_path),
                project_path,
                None,
                VENV_CREATE_TIMEOUT_SECS,
                sink,
            )
            .await
            {
                Ok(_) => {
                    if let Some(version) =
                        quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path)
                            .await
                    {
                        return Ok(venv_success(sink, &venv, "virtualenv", &version, notes).await);
                    }
                    notes.push(
                        "`python -m virtualenv` completed but pip is unavailable in the created environment"
                            .to_string(),
                    );
                }
                Err(error) => notes.push(format!("`{python} -m virtualenv` failed: {error}")),
            }
        }

        // 3. venv без pip + bootstrap pip внутрь него.
        run_venv_command(
            &python,
            &venv_create_args(&venv, &venv_path, true),
            project_path,
            None,
            VENV_CREATE_TIMEOUT_SECS,
            sink,
        )
        .await
        .map_err(|error| {
            format!(
                "{}\n\n`{python} -m venv --without-pip` failed: {error}",
                venv_failure_message(&python, &venv, project_path, &notes)
            )
        })?;

        let mut pip_version =
            quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path).await;

        // 3a. ensurepip интерпретатора venv (вдруг он всё-таки доступен).
        if pip_version.is_none() {
            let _ = run_venv_command(
                &venv_python_str,
                &args_vec(&["-m", "ensurepip", "--upgrade"]),
                project_path,
                None,
                PIP_BOOTSTRAP_TIMEOUT_SECS,
                None,
            )
            .await;
            pip_version =
                quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path).await;
        }

        // 3b. Системный pip ставит pip в чужое окружение (pip 22.3+).
        if pip_version.is_none()
            && quiet_ok(&python, &["-m", "pip", "--version"], project_path).await
        {
            let _ = run_venv_command(
                &python,
                &args_vec(&[
                    "-m",
                    "pip",
                    "install",
                    "--python",
                    &venv_python_str,
                    "--upgrade",
                    "pip",
                ]),
                project_path,
                None,
                PIP_BOOTSTRAP_TIMEOUT_SECS,
                sink,
            )
            .await;
            pip_version =
                quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path).await;
        }

        // 3c. Колесо pip из каталога дистрибутива — офлайн (Debian/Ubuntu
        // держат pip/setuptools/wheel в /usr/share/python-wheels).
        if pip_version.is_none() {
            if let Some((wheel_dir, wheel)) = distro_pip_wheel(&python, project_path).await {
                let mut env = HashMap::new();
                env.insert("PYTHONPATH".to_string(), wheel.clone());
                let _ = run_venv_command(
                    &venv_python_str,
                    &args_vec(&[
                        "-m",
                        "pip",
                        "install",
                        "--no-index",
                        "--find-links",
                        &wheel_dir,
                        "--upgrade",
                        "pip",
                    ]),
                    project_path,
                    Some(env),
                    PIP_BOOTSTRAP_TIMEOUT_SECS,
                    sink,
                )
                .await;
                pip_version =
                    quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path).await;
            }
        }

        // 3d. get-pip.py — официальный bootstrap pip, требует сеть.
        if pip_version.is_none() {
            match download_get_pip(project_path, sink).await {
                Ok(script) => {
                    let _ = run_venv_command(
                        &venv_python_str,
                        &args_vec(&[&script]),
                        project_path,
                        None,
                        PIP_BOOTSTRAP_TIMEOUT_SECS,
                        sink,
                    )
                    .await;
                    let _ = std::fs::remove_file(&script);
                    pip_version =
                        quiet_stdout(&venv_python_str, &["-m", "pip", "--version"], project_path)
                            .await;
                }
                Err(error) => notes.push(error),
            }
        }

        match pip_version {
            Some(version) => Ok(venv_success(sink, &venv, "bootstrap", &version, notes).await),
            None => Err(venv_failure_message(&python, &venv, project_path, &notes)),
        }
    }
}

/// `<venv>/bin/python` (Unix) или `<venv>\Scripts\python.exe` (Windows).
fn python_venv_binary(venv: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

fn args_vec(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Аргументы `python -m venv`: `--clear` — только для настоящего venv
/// (pyvenv.cfg), чтобы не удалять содержимое чужой папки с именем venv.
fn venv_create_args(venv: &Path, venv_path: &str, without_pip: bool) -> Vec<String> {
    let mut args = args_vec(&["-m", "venv"]);
    if venv.join("pyvenv.cfg").is_file() {
        args.push("--clear".to_string());
    }
    if without_pip {
        args.push("--without-pip".to_string());
    }
    args.push(venv_path.to_string());
    args
}

/// Аргументы `python -m virtualenv` (--clear по тому же правилу).
fn venv_virtualenv_args(venv: &Path, venv_path: &str) -> Vec<String> {
    let mut args = args_vec(&["-m", "virtualenv"]);
    if venv.join("pyvenv.cfg").is_file() {
        args.push("--clear".to_string());
    }
    args.push(venv_path.to_string());
    args
}

/// Запустить команду венв-пайплайна через общий ProcessRunner; ошибка —
/// форматированный текст с командой, рабочей директорией и хвостами вывода.
async fn run_venv_command(
    command: &str,
    args: &[String],
    working_dir: &Path,
    env: Option<HashMap<String, String>>,
    timeout_secs: u64,
    sink: Option<&ExecutionEventSink>,
) -> Result<ProcessOutput, String> {
    let spec = ProcessSpec {
        command: command.to_string(),
        args: args.to_vec(),
        working_dir: Some(working_dir.to_path_buf()),
        env,
        timeout: Some(Duration::from_secs(timeout_secs)),
        stdin: StdinMode::Null,
        ci_mode: true,
    };
    ProcessRunner::run(spec, sink)
        .await
        .map_err(|error| error.format_command_error())
}

/// Тихая проверка команды (без стриминга в UI): true при exit 0.
async fn quiet_ok(command: &str, args: &[&str], working_dir: &Path) -> bool {
    let spec = ProcessSpec {
        command: command.to_string(),
        args: args_vec(args),
        working_dir: Some(working_dir.to_path_buf()),
        env: None,
        timeout: Some(Duration::from_secs(PROBE_TIMEOUT_SECS)),
        stdin: StdinMode::Null,
        ci_mode: false,
    };
    ProcessRunner::run(spec, None).await.is_ok()
}

/// Тихий запуск с захватом stdout (первая строка хвоста) — для пробников
/// вроде `python -m pip --version`.
async fn quiet_stdout(command: &str, args: &[&str], working_dir: &Path) -> Option<String> {
    let spec = ProcessSpec {
        command: command.to_string(),
        args: args_vec(args),
        working_dir: Some(working_dir.to_path_buf()),
        env: None,
        timeout: Some(Duration::from_secs(PROBE_TIMEOUT_SECS)),
        stdin: StdinMode::Null,
        ci_mode: false,
    };
    match ProcessRunner::run(spec, None).await {
        Ok(output) if !output.stdout_tail.trim().is_empty() => Some(output.stdout_tail),
        _ => None,
    }
}

/// Каталог колёс pip дистрибутива: sysconfig.WHEEL_PKG_DIR (Debian/Ubuntu:
/// /usr/share/python-wheels) + стандартный путь как fallback. Возвращает
/// (каталог, путь к самому свежему колесу pip).
async fn distro_pip_wheel(python: &str, working_dir: &Path) -> Option<(String, String)> {
    let mut dirs: Vec<String> = Vec::new();
    if let Some(raw) = quiet_stdout(
        python,
        &[
            "-c",
            "import sysconfig; print(sysconfig.get_config_var('WHEEL_PKG_DIR') or '')",
        ],
        working_dir,
    )
    .await
    {
        let raw = raw.trim().to_string();
        if !raw.is_empty() {
            dirs.push(raw);
        }
    }
    dirs.push("/usr/share/python-wheels".to_string());

    for dir in dirs {
        let path = Path::new(&dir);
        if !path.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        let mut wheels: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(String::from))
            .filter(|name| name.starts_with("pip-") && name.ends_with(".whl"))
            .collect();
        wheels.sort_by_key(|name| wheel_version_key(name));
        if let Some(best) = wheels.pop() {
            let wheel = path.join(best);
            return Some((dir, wheel.to_string_lossy().into_owned()));
        }
    }
    None
}

/// Числовой ключ версии из имени колеса: `pip-26.2.1-py3-none-any.whl` →
/// [26, 2, 1] (сравнение числовое, а не лексикографическое).
fn wheel_version_key(name: &str) -> Vec<u64> {
    let stem = name
        .trim_start_matches("pip-")
        .split("-py")
        .next()
        .unwrap_or(name);
    let mut out: Vec<u64> = Vec::new();
    let mut num = String::new();
    for c in stem.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else if !num.is_empty() {
            out.push(num.parse().unwrap_or(0));
            num.clear();
        }
    }
    if !num.is_empty() {
        out.push(num.parse().unwrap_or(0));
    }
    out
}

/// Скачать официальный get-pip.py (единственный путь, требующий сети, когда
/// у дистрибутива нет ни ensurepip, ни pip, ни колёс). Возвращает путь к
/// скачанному скрипту в temp-каталоге.
async fn download_get_pip(
    working_dir: &Path,
    sink: Option<&ExecutionEventSink>,
) -> Result<String, String> {
    const URL: &str = "https://bootstrap.pypa.io/get-pip.py";
    let target = std::env::temp_dir().join(format!(
        "stackpilot-get-pip-{}-{}.py",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let target_str = target.to_string_lossy().into_owned();
    let (command, args) = if quiet_ok("curl", &["--version"], working_dir).await {
        (
            "curl",
            args_vec(&["-fsSL", "--retry", "2", "-o", &target_str, URL]),
        )
    } else if quiet_ok("wget", &["--version"], working_dir).await {
        ("wget", args_vec(&["-q", "-O", &target_str, URL]))
    } else {
        return Err(
            "neither curl nor wget is available to download get-pip.py — install the python3-venv package instead"
                .to_string(),
        );
    };
    emit_venv_note(
        sink,
        &format!("downloading {URL} to bootstrap pip (no python3-venv on this system)"),
    )
    .await;
    run_venv_command(command, &args, working_dir, None, 300, sink)
        .await
        .map_err(|error| format!("failed to download get-pip.py: {error}"))?;
    Ok(target_str)
}

async fn emit_venv_note(sink: Option<&ExecutionEventSink>, text: &str) {
    if let Some(sink) = sink {
        sink.emit_stdout(text).await;
    }
}

/// Успешный отчёт генератора (единый формат сообщения + заметки о деградации).
async fn venv_success(
    sink: Option<&ExecutionEventSink>,
    venv: &Path,
    strategy: &str,
    pip_version: &str,
    notes: Vec<String>,
) -> GenerationReport {
    let message = format!(
        "Created the virtual environment at {} ({strategy}; {})",
        venv.display(),
        pip_version.trim()
    );
    emit_venv_note(sink, &message).await;
    GenerationReport {
        message,
        validation_warnings: notes,
        ..GenerationReport::default()
    }
}

/// Ошибка создания venv: точные инструкции (python3-venv/virtualenv/get-pip)
/// + диагностика всех попыток.
fn venv_failure_message(
    python: &str,
    venv: &Path,
    project_path: &Path,
    notes: &[String],
) -> String {
    let interpreter = interpreter_description(python, project_path);
    let venv_python = python_venv_binary(venv);
    let versioned_venv_package = interpreter
        .as_deref()
        .and_then(|text| text.lines().find_map(|line| line.strip_prefix("venv-package: ")))
        .unwrap_or("python3-venv");
    let mut message = format!(
        "Failed to create the Python virtual environment at {}.\n\n\
         The interpreter '{python}' could not create a working environment with pip. \
         On Debian/Ubuntu this usually means the python3-venv package (which provides \
         the ensurepip module) is not installed — `python3 -m venv` then leaves an \
         incomplete environment without pip.\n\n\
         Fix it (pick one):\n\
         \x20 - Debian/Ubuntu: sudo apt install python3-venv   # or: sudo apt install {versioned_venv_package}\n\
         \x20 - Fedora/RHEL:   sudo dnf install python3-pip\n\
         \x20 - no root:       {python} -m pip install --user virtualenv   # then re-run the generation\n\
         \x20 - manual bootstrap: curl -fsSL https://bootstrap.pypa.io/get-pip.py -o get-pip.py \
         && {} get-pip.py\n",
        venv.display(),
        venv_python.display()
    );
    if let Some(text) = interpreter {
        message.push_str(&format!("\nInterpreter: {}\n", text.replace('\n', " | ")));
    }
    if !notes.is_empty() {
        message.push_str("\nWhat was tried:\n");
        for note in notes {
            message.push_str(&format!("  - {note}\n"));
        }
    }
    message
}

/// Строка с путём/версией интерпретатора и именем versioned-пакета venv
/// (python3.14-venv) для сообщений об ошибке.
fn interpreter_description(python: &str, working_dir: &Path) -> Option<String> {
    // Синхронный запуск: ошибка уже формируется как текст, сеть/таймауты не
    // нужны, а stdout тут не стримится.
    let output = std::process::Command::new(python)
        .args([
            "-c",
            "import sys; print(sys.executable); print(sys.version.split()[0]); print('venv-package: python{}.{}-venv'.format(sys.version_info.major, sys.version_info.minor))",
        ])
        .current_dir(working_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

//
// .NET SDK сам по себе НЕ содержит шаблон MAUI («Не найдены шаблоны или
// подкоманды, соответствующие: "maui"», код 103): он появляется только
// после установки workload `dotnet workload install maui`. Toolchain
// ставит workload как пост-шаг явной установки dotnet, но на машине, где
// SDK уже был установлен заранее (без StackPilot), шаблона нет.
//
// Генератор:
//   1. проверяет `dotnet new list maui` (локально, без сети, быстро);
//   2. шаблон уже есть → Success без побочных эффектов (идемпотентно);
//   3. шаблона нет → `dotnet workload install maui` + повторная проверка.
// Сбой установки workload — честная ошибка (Err), а не молчаливый скип:
// без шаблона следующий шаг (maui_new) всё равно упадёт с той же ошибкой.
// ============================================================================
pub struct DotnetMauiEnsureGenerator;

#[async_trait]
impl Generator for DotnetMauiEnsureGenerator {
    fn id(&self) -> &str {
        "dotnet-maui-ensure"
    }
    fn name(&self) -> &str {
        "Ensure .NET MAUI templates"
    }
    fn description(&self) -> &str {
        "Installs the .NET MAUI workload so `dotnet new maui` is available (no-op when already installed)"
    }

    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        _project_path: &Path,
        _config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        const CHECK_TIMEOUT_SECS: u64 = 60;
        const INSTALL_TIMEOUT_SECS: u64 = 900;

        // `dotnet new list maui` печатает найденные шаблоны в stdout и
        // завершается нулём; при отсутствии шаблона — код 103. Любой сбой
        // (нет dotnet, шаблон не найден) трактуется как «нужно обеспечить».
        async fn template_present(sink: Option<&ExecutionEventSink>) -> bool {
            let spec = ProcessSpec {
                command: "dotnet".to_string(),
                args: vec!["new".to_string(), "list".to_string(), "maui".to_string()],
                working_dir: None,
                env: None,
                timeout: Some(Duration::from_secs(CHECK_TIMEOUT_SECS)),
                stdin: StdinMode::Null,
                ci_mode: false,
            };
            match ProcessRunner::run(spec, sink).await {
                Ok(output) => !output.stdout_tail.trim().is_empty(),
                Err(_) => false,
            }
        }

        if template_present(sink).await {
            if let Some(sink) = sink {
                sink.emit_stdout(
                    "dotnet new maui: template already installed — workload not needed",
                )
                .await;
            }
            return Ok(GenerationReport::success(
                "The .NET MAUI template is already installed — no workload install needed",
            ));
        }

        if let Some(sink) = sink {
            sink.emit_stdout(
                "dotnet new maui: template missing — installing the .NET MAUI workload (`dotnet workload install maui`)",
            )
            .await;
        }
        let install_spec = ProcessSpec {
            command: "dotnet".to_string(),
            args: vec![
                "workload".to_string(),
                "install".to_string(),
                "maui".to_string(),
            ],
            working_dir: None,
            env: None,
            timeout: Some(Duration::from_secs(INSTALL_TIMEOUT_SECS)),
            stdin: StdinMode::Null,
            ci_mode: false,
        };
        if let Err(error) = ProcessRunner::run(install_spec, sink).await {
            return Err(format!(
                "Failed to install the .NET MAUI workload (required for the `maui` template): {}",
                error.format_command_error()
            ));
        }

        // Повторная проверка: workload установлен — шаблон обязан появиться.
        if !template_present(sink).await {
            return Err(
                "The .NET MAUI workload was installed, but `dotnet new maui` is still unavailable — restart the app or check the .NET SDK installation".to_string(),
            );
        }
        Ok(GenerationReport::success(
            "Installed the .NET MAUI workload; the `maui` template is now available",
        ))
    }
}

// ============================================================================
// ScaffoldGenerator — выполнение CLI-скаффолдеров по ЯВНЫМ способностям.
//
// МОДЕЛЬ СПОСОБНОСТЕЙ (ScaffoldCapability)
// ----------------------------------------
// Раньше движок считал каждый CLI-скаффолдер «create-vite»: принимает
// каталог позиционным аргументом, создаёт именно его, работает без
// интерактива, понимает одни и те же yes/no-флаги и безопасен во временной
// папке с последующим переносом. Это неверно для Nest, Nuxt, Tauri,
// SolidStart, Flutter, Zig и других инструментов. Универсальный temp+move
// применяется ТОЛЬКО там, где рецепт явно разрешил временную папку.
//
// Рецепт обязан объявить способность каждого scaffold-шага:
//
//   | Способность                      | Примеры                                 |
//   |----------------------------------|-----------------------------------------|
//   | creates_named_directory          | create-vite app, create-next-app app,   |
//   |                                  | nuxi init app, composer create-project, |
//   |                                  | create-expo-app                         |
//   | creates_in_current_directory     | nest new ., flutter create .,           |
//   |                                  | django-admin startproject x .,          |
//   |                                  | dotnet new -o .                         |
//   | creates_project_and_may_prompt   | create-solid, flutter create,           |
//   |                                  | electron-forge, RN CLI, plasmo init     |
//   | generates_root_shell             | cargo tauri init, zig init              |
//   | does_not_create_a_project        | npm install, prisma init                |
//
// Каждый шаг ЯВНО определяет:
//   - command / args                       — что выполняется;
//   - working_dir                          — рабочая директория CLI;
//   - target_dir                           — поведение назначения ("." или подкаталог);
//   - temp_dir_allowed                     — разрешена ли временная папка;
//   - expected_outputs                     — ожидаемые пути ПОСЛЕ завершения
//                                            (пост-условия: package.json,
//                                            src-tauri/tauri.conf.json,
//                                            pubspec.yaml, build.zig...);
//   - interactive                          — обязательные интерактивные ответы
//                                            (для may-prompt способностей).
//
// ПОСТ-УСЛОВИЯ: после успешного выхода CLI каждый expected_output обязан
// существовать в каталоге назначения. Провал — ОДНА ошибка шага (все
// недостающие пути перечислены), зависимые шаги рецепта (патчи package.json,
// tauri-config и т.п.) не выполняются: движок пропускает их по
// FileExists-условию, а не выдаёт вторичные ENOENT-ошибки.
//
// МИГРАЦИЯ РЕЦЕПТОВ (движок engine/mod.rs — см. scaffold_step):
//   vite (react/vue/svelte) → creates_named_directory + temp, ожидает package.json
//   nextjs                   → creates_named_directory + temp, ожидает package.json
//   sveltekit                → creates_named_directory + temp, ожидает package.json
//   nuxt                     → creates_named_directory + temp, ожидает package.json
//   expo                     → creates_named_directory + temp, ожидает package.json
//   laravel/symfony          → creates_named_directory + temp (composer), ожидает composer.json
//   solidjs                  → creates_project_and_may_prompt + interactive, ожидает package.json
//   flutter                  → creates_project_and_may_prompt + interactive, ожидает pubspec.yaml
//   tauri_web_scaffold       → creates_named_directory + temp, ожидает package.json
//   tauri_init               → generates_root_shell, ожидает src-tauri/tauri.conf.json
//
// Конфиг (Step::Generate.generator_config):
//   {
//     "command": "npx",
//     "args": ["create-vite@latest", "__TARGET__", "--template", "react-ts"],
//     "capability": "creates_named_directory",   // ОБЯЗАТЕЛЬНО
//     "target_dir": "frontend",                  // "." = корень проекта
//     "working_dir": ".",                        // относительно корня (опционально)
//     "temp_dir_allowed": true,                  // опционально, по умолчанию — по способности
//     "expected_outputs": ["package.json"],      // пост-условия (опционально)
//     "interactive": [{"trigger": "...", "response_type": {"Text": "npm"}}],
//     "timeout_secs": 600
//   }
// ============================================================================
pub struct ScaffoldGenerator;

/// Полное описание одного scaffold-шага — рецепт обязан заполнить все поля.
#[derive(Debug, Clone)]
pub struct ScaffoldStepConfig {
    pub command: String,
    pub args: Vec<String>,
    /// Способность CLI (см. ScaffoldCapability).
    pub capability: ScaffoldCapability,
    /// Куда проект обязан попасть: "." (корень) или относительный подкаталог.
    pub target_dir: String,
    /// Рабочая директория CLI относительно корня проекта. По умолчанию:
    /// корень для named-directory, target_dir для in-place/root-shell.
    pub working_dir: Option<String>,
    /// Разрешена ли временная папка (temp+move). По умолчанию — по способности.
    pub temp_dir_allowed: Option<bool>,
    /// Пост-условия: ожидаемые файлы/каталоги после завершения (относительно
    /// каталога назначения). Провал любого — ошибка шага.
    pub expected_outputs: Vec<String>,
    /// Интерактивные ответы (piped stdin) — обязательны для may-prompt.
    pub interactive: Vec<InteractiveEntry>,
    pub timeout_secs: u64,
}

impl ScaffoldStepConfig {
    pub fn from_json(config: &serde_json::Value) -> Result<Self, String> {
        let command = config
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "ScaffoldGenerator: missing 'command' in config".to_string())?
            .to_string();
        let args: Vec<String> = config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .ok_or_else(|| {
                "ScaffoldGenerator: missing 'args' (array of strings) in config".to_string()
            })?;
        let capability_str = config
            .get("capability")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                "ScaffoldGenerator: missing 'capability' in config — recipe must declare the \
                 scaffold capability explicitly (creates_named_directory | \
                 creates_in_current_directory | creates_project_and_may_prompt | \
                 generates_root_shell | does_not_create_a_project)"
                    .to_string()
            })?;
        let capability = match capability_str {
            "creates_named_directory" => ScaffoldCapability::CreatesNamedDirectory,
            "creates_in_current_directory" => ScaffoldCapability::CreatesInCurrentDirectory,
            "creates_project_and_may_prompt" => ScaffoldCapability::CreatesProjectAndMayPrompt,
            "generates_root_shell" => ScaffoldCapability::GeneratesRootShell,
            "does_not_create_a_project" => ScaffoldCapability::DoesNotCreateAProject,
            other => {
                return Err(format!(
                    "ScaffoldGenerator: unknown 'capability' '{}' (expected: \
                     creates_named_directory | creates_in_current_directory | \
                     creates_project_and_may_prompt | generates_root_shell | \
                     does_not_create_a_project)",
                    other
                ));
            }
        };
        let target_dir = config
            .get("target_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();
        let working_dir = config
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(String::from);
        let temp_dir_allowed = config.get("temp_dir_allowed").and_then(|v| v.as_bool());
        let expected_outputs: Vec<String> = config
            .get("expected_outputs")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let interactive: Vec<InteractiveEntry> = config
            .get("interactive")
            .and_then(|a| a.as_array())
            .map(|arr| arr.iter().filter_map(parse_interactive_entry).collect())
            .unwrap_or_default();
        let timeout_secs = config
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);
        Ok(Self {
            command,
            args,
            capability,
            target_dir,
            working_dir,
            temp_dir_allowed,
            expected_outputs,
            interactive,
            timeout_secs,
        })
    }
}

/// Разобрать {"trigger": "...", "response_type": ...} в InteractiveEntry.
fn parse_interactive_entry(value: &serde_json::Value) -> Option<InteractiveEntry> {
    let trigger = value.get("trigger")?.as_str()?.to_string();
    let response_type = parse_response_type(value.get("response_type")?)?;
    Some(InteractiveEntry {
        trigger,
        response_type,
    })
}

/// Разобрать response_type: строка-шорткат ("y"/"n"/"yes"/текст) или
/// сериализованный вариант ResponseType ({"Text": "npm"} и т.п.).
fn parse_response_type(value: &serde_json::Value) -> Option<ResponseType> {
    if let Some(text) = value.as_str() {
        return match text.to_ascii_lowercase().as_str() {
            "y" | "yes" | "confirm" => Some(ResponseType::Confirm(true)),
            "n" | "no" | "decline" => Some(ResponseType::Confirm(false)),
            _ => Some(ResponseType::Text(text.to_string())),
        };
    }
    serde_json::from_value::<ResponseType>(value.clone()).ok()
}

#[async_trait]
impl Generator for ScaffoldGenerator {
    fn id(&self) -> &str {
        "scaffold"
    }
    fn name(&self) -> &str {
        "Smart Scaffold"
    }
    fn description(&self) -> &str {
        "Runs a scaffolding CLI according to its declared capability, then validates expected outputs"
    }
    async fn generate_with_sink(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
        sink: Option<&ExecutionEventSink>,
    ) -> Result<GenerationReport, String> {
        let cfg = ScaffoldStepConfig::from_json(config)?;
        let mut report = GenerationReport::success(format!(
            "Scaffold '{}' ({}) completed",
            cfg.command,
            cfg.capability.as_str()
        ));

        // may-prompt способность без ответов — CLI может повиснуть на вопросе.
        if cfg.capability == ScaffoldCapability::CreatesProjectAndMayPrompt
            && cfg.interactive.is_empty()
        {
            report.validation_warnings.push(
                "capability 'creates_project_and_may_prompt' has no interactive answers \
                 configured — the CLI may prompt and hang"
                    .to_string(),
            );
        }

        // Каталог назначения: куда проект обязан попасть.
        let target = resolve_target(project_path, &cfg.target_dir)?;
        if cfg.target_dir != "." && !target.exists() {
            std::fs::create_dir_all(&target).map_err(|e| {
                format!(
                    "ScaffoldGenerator: failed to create target dir '{}': {}",
                    cfg.target_dir, e
                )
            })?;
            report.created_directories.push(cfg.target_dir.clone());
        }
        let work_dir = resolve_working_dir(project_path, &cfg);
        if !work_dir.exists() {
            // Рабочая директория CLI обязана существовать (например,
            // does_not_create_a_project с working_dir="backend").
            std::fs::create_dir_all(&work_dir).map_err(|e| {
                format!(
                    "ScaffoldGenerator: failed to create working dir '{}': {}",
                    work_dir.display(),
                    e
                )
            })?;
        }
        let mut args = cfg.args.clone();

        match cfg.capability {
            ScaffoldCapability::CreatesNamedDirectory
            | ScaffoldCapability::CreatesProjectAndMayPrompt => {
                run_named_directory_scaffold(
                    &cfg,
                    &mut args,
                    project_path,
                    &target,
                    &work_dir,
                    sink,
                    &mut report,
                )
                .await?;
            }
            ScaffoldCapability::CreatesInCurrentDirectory => {
                // CLI работает ВНУТРИ каталога назначения с "." (или уже с
                // фиксированным именем в args — плейсхолдер заменяется).
                replace_placeholder(&mut args, ".");
                run_cli_process(
                    &cfg.command,
                    &args,
                    &target,
                    None,
                    cfg.timeout_secs,
                    &cfg.interactive,
                    sink,
                )
                .await?;
            }
            ScaffoldCapability::GeneratesRootShell | ScaffoldCapability::DoesNotCreateAProject => {
                if args.iter().any(|a| a == SCAFFOLD_TARGET) {
                    report.validation_warnings.push(format!(
                        "capability '{}' does not take the '{}' placeholder — left as-is",
                        cfg.capability.as_str(),
                        SCAFFOLD_TARGET
                    ));
                }
                run_cli_process(
                    &cfg.command,
                    &args,
                    &work_dir,
                    None,
                    cfg.timeout_secs,
                    &cfg.interactive,
                    sink,
                )
                .await?;
            }
        }

        // ПОСТ-УСЛОВИЯ: ожидаемые выходные пути обязаны существовать. Провал —
        // одна ошибка шага; зависимые шаги рецепта не выполняются.
        validate_outputs(&cfg, &target, &mut report)?;

        Ok(report)
    }
}

/// Заменить плейсхолдер в args (если он есть).
fn replace_placeholder(args: &mut [String], replacement: &str) {
    for arg in args.iter_mut() {
        if arg == SCAFFOLD_TARGET {
            *arg = replacement.to_string();
        }
    }
}

/// Каталог назначения (абсолютный), создаётся при необходимости.
fn resolve_target(project_path: &Path, target_dir: &str) -> Result<std::path::PathBuf, String> {
    if target_dir == "." {
        Ok(project_path.to_path_buf())
    } else {
        Ok(project_path.join(target_dir))
    }
}

/// Рабочая директория CLI: явный working_dir, иначе по способности —
/// корень (named-directory) или каталог назначения (in-place/root-shell).
fn resolve_working_dir(project_path: &Path, cfg: &ScaffoldStepConfig) -> std::path::PathBuf {
    match (&cfg.working_dir, cfg.capability) {
        (Some(wd), _) => project_path.join(wd),
        (
            None,
            ScaffoldCapability::CreatesInCurrentDirectory | ScaffoldCapability::GeneratesRootShell,
        ) => {
            if cfg.target_dir == "." {
                project_path.to_path_buf()
            } else {
                project_path.join(&cfg.target_dir)
            }
        }
        (None, _) => project_path.to_path_buf(),
    }
}

/// Выполнение named-directory / may-prompt скаффолдера:
///   1. имя создаваемой папки подставляется на место плейсхолдера
///      (временная папка при temp_dir_allowed, иначе — имя каталога
///      назначения рядом с ним);
///   2. CLI запускается в рабочей директории;
///   3. созданная папка переносится в target (merge, если это не сам target);
///   4. матрёшка <target>/<name> нормализуется (содержимое поднимается вверх).
async fn run_named_directory_scaffold(
    cfg: &ScaffoldStepConfig,
    args: &mut Vec<String>,
    project_path: &Path,
    target: &Path,
    work_dir: &Path,
    sink: Option<&ExecutionEventSink>,
    report: &mut GenerationReport,
) -> Result<(), String> {
    let placeholder = args
        .iter()
        .position(|a| a == SCAFFOLD_TARGET)
        .ok_or_else(|| {
            format!(
                "ScaffoldGenerator: capability '{}' requires the '{}' placeholder in args",
                cfg.capability.as_str(),
                SCAFFOLD_TARGET
            )
        })?;
    let temp_allowed = cfg
        .temp_dir_allowed
        .unwrap_or_else(|| cfg.capability.supports_temp_dir());

    // Куда CLI кладёт проект: уникальная временная папка в корне проекта
    // (temp+move) или папка с именем каталога назначения рядом с ним.
    // `run_dir` — фактическая рабочая директория CLI; `staging_root` задан,
    // когда CLI пришлось увести в staging-родитель (коллизия с target).
    let (run_dir, created_dir, created_name, needs_merge, staging_root) = if temp_allowed {
        let temp_name = unique_temp_name(project_path, &cfg.target_dir);
        args[placeholder] = temp_name.clone();
        (
            work_dir.to_path_buf(),
            work_dir.join(&temp_name),
            temp_name,
            true,
            None,
        )
    } else {
        // target=".": имя = имя папки проекта (как поступил бы пользователь);
        // иначе — имя каталога назначения (CLI создаёт его рядом с самим собой).
        let name = if cfg.target_dir == "." {
            project_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("app")
                .to_string()
        } else {
            Path::new(&cfg.target_dir)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("app")
                .to_string()
        };
        args[placeholder] = name.clone();
        let created = work_dir.join(&name);
        // Коллизия: каталог назначения уже существует и не пуст (повторный
        // запуск, чужой каркас в frontend/ и т.п.). RN CLI и подобные
        // скаффолдеры отказываются писать в непустой каталог. Запускаем CLI
        // в свежем staging-родителе, сохраняя ИМЯ проекта (app.json/package
        // name остаются правильными), и программно сливаем результат в target.
        if dir_has_entries(&created) {
            let staging = work_dir.join(unique_temp_name(work_dir, &cfg.target_dir));
            std::fs::create_dir_all(&staging).map_err(|e| {
                format!(
                    "ScaffoldGenerator: failed to create staging dir '{}': {}",
                    staging.display(),
                    e
                )
            })?;
            report.validation_warnings.push(format!(
                "target '{}' already contained files — ran the scaffold in a staging directory and merged the result",
                cfg.target_dir
            ));
            (
                staging.clone(),
                staging.join(&name),
                name,
                true,
                Some(staging),
            )
        } else {
            // CLI создал сам каталог назначения — слияние не нужно.
            let needs_merge = created != target;
            (work_dir.to_path_buf(), created, name, needs_merge, None)
        }
    };

    // Перенос + нормализация матрёшки. При ЛЮБОЙ ошибке (включая провал
    // самого CLI, создавшего временную папку) временная папка (temp+move)
    // удаляется целиком — повторный запуск рецепта не натыкается на хвосты
    // проваленного скаффолда, а postcondition-валидация работает по честному
    // «файл есть/нет».
    let result = async {
        run_cli_process(
            &cfg.command,
            args,
            &run_dir,
            None,
            cfg.timeout_secs,
            &cfg.interactive,
            sink,
        )
        .await?;

        if !created_dir.is_dir() {
            return Err(format!(
                "Scaffold '{}' failed: the CLI exited successfully but did not create the \
                 expected directory '{}' in {}",
                cfg.command,
                created_name,
                run_dir.display()
            ));
        }

        if needs_merge {
            // Перенос содержимого (node_modules-масштаба деревья) — тяжёлая
            // синхронная работа: выносим с async-рантайма в блокирующий поток.
            let merge_from = created_dir.clone();
            let merge_to = target.to_path_buf();
            let merge_result: Result<(), String> =
                tokio::task::spawn_blocking(move || merge_dir_contents(&merge_from, &merge_to))
                    .await
                    .map_err(|e| {
                        format!("Scaffold '{}': merge task panicked: {e}", cfg.command)
                    })?;
            merge_result.map_err(|e| {
                format!(
                    "Scaffold '{}': failed to move '{}' into '{}': {}",
                    cfg.command, created_name, cfg.target_dir, e
                )
            })?;
            std::fs::remove_dir_all(&created_dir).map_err(|e| {
                format!(
                    "Scaffold '{}': failed to remove '{}': {}",
                    cfg.command, created_name, e
                )
            })?;
        } else {
            report.skipped_dependent_steps.push(format!(
                "merge of '{}' into '{}' skipped (CLI created the target itself)",
                created_name, cfg.target_dir
            ));
        }

        // Матрёшка: CLI положил проект во вложенную <target>/<name> — содержимое
        // поднимается в target, папка удаляется, факт фиксируется предупреждением.
        let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let nested = target.join(&created_name);
        if created_name != target_name && nested.is_dir() {
            let flatten_from = nested.clone();
            let flatten_to = target.to_path_buf();
            let flatten_result: Result<(), String> =
                tokio::task::spawn_blocking(move || merge_dir_contents(&flatten_from, &flatten_to))
                    .await
                    .map_err(|e| {
                        format!("Scaffold '{}': flatten task panicked: {e}", cfg.command)
                    })?;
            flatten_result.map_err(|e| {
                format!(
                    "Scaffold '{}': failed to flatten nested '{}' into '{}': {}",
                    cfg.command, created_name, cfg.target_dir, e
                )
            })?;
            let _ = std::fs::remove_dir_all(&nested);
            report.validation_warnings.push(format!(
                "CLI created a nested '{}/' folder — its contents were flattened into '{}'",
                created_name, cfg.target_dir
            ));
        } else if needs_merge {
            report.skipped_dependent_steps.push(format!(
                "nested-folder flatten skipped (no nested '{}' detected in '{}')",
                created_name, cfg.target_dir
            ));
        }

        Ok(())
    }
    .await;
    if result.is_err() {
        // Провал: хвосты проваленного скаффолда не должны ломать повторный
        // запуск — чистим созданный каталог (temp+move) и staging-родитель.
        if temp_allowed {
            let _ = std::fs::remove_dir_all(&created_dir);
        }
        if let Some(staging) = &staging_root {
            let _ = std::fs::remove_dir_all(staging);
        }
    } else if let Some(staging) = &staging_root {
        // Успех: содержимое перенесено в target — staging-родитель пуст.
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

/// Проверка пост-условий: каждый expected_output обязан существовать в
/// каталоге назначения. Провал — ОДНА ошибка со списком недостающих путей.
/// Файлы/каталоги, подтверждённые валидацией, попадают в отчёт.
fn validate_outputs(
    cfg: &ScaffoldStepConfig,
    target: &Path,
    report: &mut GenerationReport,
) -> Result<(), String> {
    if cfg.expected_outputs.is_empty() {
        return Ok(());
    }
    let mut missing: Vec<String> = Vec::new();
    for output in &cfg.expected_outputs {
        let full = target.join(output);
        if !full.exists() {
            missing.push(output.clone());
            continue;
        }
        let reported = report_path(&cfg.target_dir, output);
        if full.is_dir() {
            report.created_directories.push(reported);
        } else {
            report.created_files.push(reported.clone());
            // Пустой файл — подозрительный результат (package.json без
            // содержимого и т.п.) — предупреждение, не ошибка.
            if full.metadata().map(|m| m.len() == 0).unwrap_or(false) {
                report.validation_warnings.push(format!(
                    "expected output '{}' exists but is empty",
                    reported
                ));
            }
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Scaffold '{}' validation failed: expected output(s) missing in '{}': {}",
            cfg.command,
            cfg.target_dir,
            missing.join(", ")
        ));
    }
    Ok(())
}

/// Путь для отчёта: с префиксом каталога назначения (кроме корня ".").
fn report_path(target_dir: &str, output: &str) -> String {
    if target_dir == "." {
        output.to_string()
    } else {
        format!("{}/{}", target_dir, output)
    }
}

// ============================================================================
// TauriConfigGenerator — Rust-патч src-tauri/tauri.conf.json после tauri init.
//
// `cargo tauri init --ci` создаёт конфиг под фронтенд в корне приложения.
// Наш фронтенд живёт в frontend/, поэтому конфиг правится ПРОГРАММНО (не
// текстовыми заменами): build.beforeDevCommand / beforeBuildCommand / devUrl /
// frontendDist указывают на frontend/, legacy build.distDir (v1) тоже
// переписывается, если присутствует; identifier берётся из конфига.
//
// Strict Subdir Mandate: при сегментации tauri живёт в backend/, поэтому
// конфиг лежит в backend/src-tauri/tauri.conf.json (tauri_dir="backend") и
// frontendDist = "../../frontend/dist" (передаётся движком явно).
//
// Конфиг:
//   {
//     "frontend_dir": "frontend",
//     "tauri_dir": "",            // каталог tauri-проекта ("backend" в моно-репо)
//     "frontend_dist": "../frontend/dist",  // готовый путь (опционально)
//     "dev_url": "http://localhost:5173",
//     "before_dev_command": "npm --prefix frontend run dev",
//     "before_build_command": "npm --prefix frontend run build",
//     "identifier": "com.myapp"
//   }
// ============================================================================
pub struct TauriConfigGenerator;

#[async_trait]
impl Generator for TauriConfigGenerator {
    fn id(&self) -> &str {
        "tauri-config"
    }
    fn name(&self) -> &str {
        "Tauri config patch"
    }
    fn description(&self) -> &str {
        "Adapts src-tauri/tauri.conf.json to the frontend/ layout"
    }
    async fn generate(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        let frontend_dir = config
            .get("frontend_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("frontend");
        let tauri_dir = config
            .get("tauri_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dev_url = config
            .get("dev_url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:5173");
        let before_dev_command = config
            .get("before_dev_command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                "TauriConfigGenerator: missing 'before_dev_command' in config".to_string()
            })?;
        let before_build_command = config
            .get("before_build_command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                "TauriConfigGenerator: missing 'before_build_command' in config".to_string()
            })?;
        let identifier = config
            .get("identifier")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "TauriConfigGenerator: missing 'identifier' in config".to_string())?;
        // Путь на dist передаёт движок целиком (отличается в root-режиме и
        // при сегментации: ../frontend/dist против ../../frontend/dist).
        let dist = config
            .get("frontend_dist")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("../{}/dist", frontend_dir));

        // Валидация итоговых путей: каталог фронтенда обязан существовать
        // (его создал фронтенд-скаффолд ДО tauri init). Отсутствие означает
        // оборванную цепочку «vite/компаньон → frontend/» — патчить конфиг
        // на пустой каталог бессмысленно.
        let frontend_path = project_path.join(frontend_dir);
        if !frontend_path.is_dir() {
            return Err(format!(
                "TauriConfigGenerator: frontend dir '{}' does not exist — the frontend scaffold did not run (no '{}' next to src-tauri/). Aborting the tauri.conf.json patch.",
                frontend_dir,
                frontend_path.join("package.json").display()
            ));
        }

        let tauri_root = if tauri_dir.is_empty() {
            project_path.to_path_buf()
        } else {
            project_path.join(tauri_dir)
        };
        let config_path = tauri_root.join("src-tauri").join("tauri.conf.json");
        let mut value = read_json(&config_path)
            .map_err(|e| format!("TauriConfigGenerator: {} (tauri init не создал конфиг?)", e))?;

        if !value.get("build").is_some_and(|v| v.is_object()) {
            value["build"] = serde_json::json!({});
        }
        let build = value
            .get_mut("build")
            .and_then(|v| v.as_object_mut())
            .expect("build должен быть объектом");
        build.insert(
            "beforeDevCommand".into(),
            serde_json::Value::String(before_dev_command.to_string()),
        );
        build.insert(
            "beforeBuildCommand".into(),
            serde_json::Value::String(before_build_command.to_string()),
        );
        build.insert(
            "devUrl".into(),
            serde_json::Value::String(dev_url.to_string()),
        );
        build.insert(
            "frontendDist".into(),
            serde_json::Value::String(dist.clone()),
        );
        // v1 (legacy): ключ distDir — переписываем, только если присутствует
        // (в v2-конфиге неизвестный ключ distDir сломал бы валидацию tauri).
        if build.contains_key("distDir") {
            build.insert("distDir".into(), serde_json::Value::String(dist));
        }

        value["identifier"] = serde_json::Value::String(identifier.to_string());

        write_json(&config_path, &value)?;
        let reported_path = if tauri_dir.is_empty() {
            "src-tauri/tauri.conf.json".to_string()
        } else {
            format!("{}/src-tauri/tauri.conf.json", tauri_dir)
        };
        Ok(GenerationReport {
            modified_files: vec![reported_path],
            ..GenerationReport::success(
                "Tauri configuration adapted to the frontend/ layout".to_string(),
            )
        })
    }
}

// ============================================================================
// VsCodeMergeGenerator — слияние .vscode-конфигов CLI с нашими.
//
// Проблема: часть скаффолдеров (create-next-app и т.п.) сами создают
// .vscode/settings.json. Старый пайплайн писал наш settings.json с
// overwrite=true и ЗАТИРАЛ конфиг CLI (typescript.tsdk и т.п.), а рядом
// появлялись лишние .vscode-папки в подкаталогах.
//
// Теперь: наш конфиг СЛИВАЕТСЯ с существующим — ключи CLI побеждают при
// конфликте (вложенные объекты сливаются рекурсивно), extensions.json
// объединяет списки рекомендаций. В не-корневых каталогах (frontend/ и т.п.)
// .vscode НЕ создаётся заново — конфиг примешивается только если его уже
// создал сам CLI.
//
// Конфиг:
//   { "lang": "typescript", "dirs": [".", "frontend"] }
// ============================================================================
pub struct VsCodeMergeGenerator;

#[async_trait]
impl Generator for VsCodeMergeGenerator {
    fn id(&self) -> &str {
        "vscode-merge"
    }
    fn name(&self) -> &str {
        "VS Code config merge"
    }
    fn description(&self) -> &str {
        "Merges StackPilot VS Code settings with the ones created by scaffolding CLIs"
    }
    async fn generate(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        let lang = config
            .get("lang")
            .and_then(|v| v.as_str())
            .unwrap_or("python");
        // Интерпретатор Python из канонической раскладки (venv в корне или
        // backend/venv) — движок передаёт его в конфиге. Если python не
        // главный язык, ключ всё равно добавляется к настройкам.
        let python_interpreter = config
            .get("python_interpreter")
            .and_then(|v| v.as_str());
        let dirs: Vec<String> = config
            .get("dirs")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec![".".to_string()]);

        let settings_ours = if lang == "python" {
            parse_json(&content::generate_vscode_settings_with_interpreter(
                lang,
                python_interpreter,
            ))?
        } else {
            let mut value = parse_json(&content::generate_vscode_settings(lang))?;
            if let Some(interpreter) = python_interpreter {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "python.defaultInterpreterPath".to_string(),
                        serde_json::json!(interpreter),
                    );
                }
            }
            value
        };
        let extensions_ours = parse_json(&content::generate_vscode_extensions(lang))?;

        let mut created_files: Vec<String> = Vec::new();
        let mut modified_files: Vec<String> = Vec::new();
        let mut seen_dirs: Vec<String> = Vec::new();

        for dir in &dirs {
            if dir.is_empty() || seen_dirs.contains(dir) {
                continue;
            }
            seen_dirs.push(dir.clone());
            let base = if dir == "." {
                project_path.to_path_buf()
            } else {
                project_path.join(dir)
            };
            let is_root = dir == ".";
            let vscode_dir = base.join(".vscode");
            let settings_path = vscode_dir.join("settings.json");
            let extensions_path = vscode_dir.join("extensions.json");

            // В не-корневых каталогах .vscode создаётся только самим CLI —
            // лишние папки не дублируем.
            if !is_root && !settings_path.exists() {
                continue;
            }

            std::fs::create_dir_all(&vscode_dir).map_err(|e| {
                format!(
                    "VsCodeMergeGenerator: failed to create '{}': {}",
                    vscode_dir.display(),
                    e
                )
            })?;

            let settings_existing =
                read_json(&settings_path).unwrap_or_else(|_| serde_json::json!({}));
            let merged_settings = merge_json(&settings_existing, &settings_ours);
            let was_created = !settings_path.exists();
            write_json(&settings_path, &merged_settings)?;
            if was_created {
                created_files.push(format!("{}/.vscode/settings.json", dir));
            } else {
                modified_files.push(format!("{}/.vscode/settings.json", dir));
            }

            let extensions_existing =
                read_json(&extensions_path).unwrap_or_else(|_| serde_json::json!({}));
            let merged_extensions = merge_extensions(&extensions_existing, &extensions_ours);
            let was_created = !extensions_path.exists();
            write_json(&extensions_path, &merged_extensions)?;
            if was_created {
                created_files.push(format!("{}/.vscode/extensions.json", dir));
            } else {
                modified_files.push(format!("{}/.vscode/extensions.json", dir));
            }
        }

        Ok(GenerationReport {
            created_files,
            modified_files,
            ..GenerationReport::success(
                "VS Code settings merged with scaffolding CLI configs".to_string(),
            )
        })
    }
}

// ============================================================================
// VsCodeFoldersMergeGenerator — слияние вложенных .vscode в корневой.
//
// CLI-скаффолдеры (create-next-app и т.п.) создают .vscode/ внутри своих
// папок: frontend/.vscode, backend/.vscode. После того как все каркасы
// собраны, вложенные .vscode СЛИВАЮТСЯ в корневой .vscode/ (settings.json —
// глубокое слияние, extensions.json — объединение списков), а вложенные
// папки удаляются. Конфиги CLI (typescript.tsdk и т.п.) побеждают при
// конфликте — как в VsCodeMergeGenerator.
//
// Конфиг не требуется: каталоги frontend/ и backend/ сканируются
// автоматически. No-op, если вложенных .vscode нет.
// ============================================================================
pub struct VsCodeFoldersMergeGenerator;

#[async_trait]
impl Generator for VsCodeFoldersMergeGenerator {
    fn id(&self) -> &str {
        "vscode-folders"
    }
    fn name(&self) -> &str {
        "VS Code folders merge"
    }
    fn description(&self) -> &str {
        "Merges frontend/.vscode and backend/.vscode into the root .vscode/"
    }
    async fn generate(
        &self,
        _context: &WizardContext,
        project_path: &Path,
        _config: &serde_json::Value,
    ) -> Result<GenerationReport, String> {
        merge_vscode_folders(project_path)
    }
}

// ============================================================================
// Общие помощники
// ============================================================================

/// Слить .vscode из frontend/ и backend/ в корневой .vscode/ и удалить
/// вложенные папки (см. VsCodeFoldersMergeGenerator).
///
/// settings.json сливается глубоко (ключи вложенного .vscode побеждают —
/// конфиги CLI не затираются), extensions.json объединяет recommendations
/// без дубликатов. Папка с .vscode, но без файлов — просто удаляется.
fn merge_vscode_folders(project_path: &Path) -> Result<GenerationReport, String> {
    let mut merged_files: Vec<String> = Vec::new();
    for dir in ["frontend", "backend"] {
        let inner = project_path.join(dir).join(".vscode");
        if !inner.exists() {
            continue;
        }
        let root_vscode = project_path.join(".vscode");
        std::fs::create_dir_all(&root_vscode).map_err(|e| {
            format!(
                "VsCodeFoldersMergeGenerator: failed to create root '.vscode': {}",
                e
            )
        })?;

        for file in ["settings.json", "extensions.json"] {
            let inner_file = inner.join(file);
            if !inner_file.exists() {
                continue;
            }
            let root_file = root_vscode.join(file);
            if root_file.exists() {
                let root_value = read_json(&root_file)?;
                let inner_value = read_json(&inner_file)?;
                let merged = if file == "extensions.json" {
                    merge_extensions(&inner_value, &root_value)
                } else {
                    merge_json(&inner_value, &root_value)
                };
                write_json(&root_file, &merged)?;
            } else {
                std::fs::copy(&inner_file, &root_file).map_err(|e| {
                    format!(
                        "VsCodeFoldersMergeGenerator: failed to copy {} → {}: {}",
                        inner_file.display(),
                        root_file.display(),
                        e
                    )
                })?;
            }
            merged_files.push(format!("{}/.vscode/{}", dir, file));
        }

        std::fs::remove_dir_all(&inner).map_err(|e| {
            format!(
                "VsCodeFoldersMergeGenerator: failed to remove '{}': {}",
                inner.display(),
                e
            )
        })?;
    }

    let message = if merged_files.is_empty() {
        "No inner .vscode folders found — nothing to merge".to_string()
    } else {
        format!(
            "Merged inner .vscode into root: {}",
            merged_files.join(", ")
        )
    };
    Ok(GenerationReport {
        modified_files: merged_files,
        ..GenerationReport::success(message)
    })
}

/// Уникальное имя временной папки для temp+move: `temp_<target>` + числовой
/// суффикс, если папка с таким именем уже занята.
fn unique_temp_name(project_path: &Path, target_dir: &str) -> String {
    let hint: String = target_dir
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let hint = if hint.is_empty() {
        "scaffold".to_string()
    } else {
        hint
    };
    let mut name = format!("temp_{}", hint);
    let mut counter = 2;
    while project_path.join(&name).exists() {
        name = format!("temp_{}_{}", hint, counter);
        counter += 1;
    }
    name
}

/// Каталог существует и содержит хотя бы один элемент (файл или папку)?
/// Несуществующий каталог и каталог, который не удалось прочитать, — false.
fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Рекурсивно перенести содержимое src в dst: каталоги сливаются, файлы
/// перезаписываются. Скрытые файлы включены. Исходные файлы после
/// копирования удаляются (перенос, а не копирование); опустевшие каталоги
/// тоже убираются.
fn merge_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(src).map_err(|e| format!("read_dir {}: {}", src.display(), e))? {
        let entry = entry.map_err(|e| format!("read_dir entry: {}", e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            std::fs::create_dir_all(&to)
                .map_err(|e| format!("create_dir_all {}: {}", to.display(), e))?;
            merge_dir_contents(&from, &to)?;
            // Опустевший после переноса каталог удаляем (не ошибка, если
            // внутри остались чужие файлы).
            let _ = std::fs::remove_dir(&from);
        } else {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create_dir_all {}: {}", parent.display(), e))?;
            }
            std::fs::copy(&from, &to)
                .map_err(|e| format!("copy {} → {}: {}", from.display(), to.display(), e))?;
            std::fs::remove_file(&from).map_err(|e| format!("remove {}: {}", from.display(), e))?;
        }
    }
    Ok(())
}

/// Глубокое слияние JSON: ключи base побеждают (существующие настройки CLI
/// не затираются нашими), недостающие ключи берутся из extra; вложенные
/// объекты сливаются рекурсивно.
fn merge_json(base: &serde_json::Value, extra: &serde_json::Value) -> serde_json::Value {
    match (base, extra) {
        (serde_json::Value::Object(b), serde_json::Value::Object(e)) => {
            let mut out = b.clone();
            for (k, v) in e {
                out.entry(k.clone())
                    .and_modify(|existing| {
                        *existing = merge_json(existing, v);
                    })
                    .or_insert_with(|| v.clone());
            }
            serde_json::Value::Object(out)
        }
        (base, _) => base.clone(),
    }
}

/// Слияние extensions.json: список recommendations объединяется (базовый
/// список первым, без дубликатов).
fn merge_extensions(base: &serde_json::Value, extra: &serde_json::Value) -> serde_json::Value {
    let mut out = base.clone();
    if !out.is_object() {
        out = serde_json::json!({});
    }
    let obj = out.as_object_mut().expect("out должен быть объектом");
    let base_recs = obj
        .get("recommendations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let extra_recs = extra
        .get("recommendations")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    let mut merged = base_recs;
    for rec in extra_recs {
        if !merged.contains(&rec) {
            merged.push(rec);
        }
    }
    obj.insert("recommendations".into(), serde_json::Value::Array(merged));
    out
}

/// Разобрать JSON-текст (наши generated-шаблоны и файлы на диске).
fn parse_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("invalid JSON: {}", e))
}

fn read_json(path: &Path) -> Result<serde_json::Value, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_json(&text)
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("serialize JSON: {}", e))?;
    std::fs::write(path, format!("{}\n", text))
        .map_err(|e| format!("write {}: {}", path.display(), e))
}

/// Скачать starter.zip с start.spring.io через curl.
///
/// `--fail-with-body` (curl >= 7.76): при HTTP >= 400 процесс выходит с
/// кодом 22, но тело ответа (JSON/HTML ошибки) сохраняется в файл — его
/// потом разбирает Rust-сторона и показывает реальную причину.
///
/// Любой сбой (HTTP 400 от Initializr, таймаут, недоступный хост) даёт
/// ошибку вида «Spring Initializr отказал: <причина>» — генерация
/// прерывается до попытки распаковать сломанный архив.
///
/// curl запускается НАПРЯМУЮ (без cmd/sh-обёртки): query-параметры URL
/// percent-кодируются (urlencode — пробелы, кавычки, &, %), а шелл
/// (особенно cmd) раскрывает %VAR%-пары и пережёвывает спецсимволы —
/// строка `?name=a%20b&deps=web` в кавычках cmd даст `?name=a20b`. Прямой
/// spawn передаёт аргумент curl'у байт-в-байт (build_command не оборачивает
/// команду без метасимволов, поэтому curl идёт напрямую).
async fn download_starter(
    url: &str,
    zip_path: &Path,
    sink: Option<&ExecutionEventSink>,
) -> Result<(), String> {
    let zip_str = zip_path.to_string_lossy().to_string();
    let args: Vec<String> = vec![
        "-sSL".into(),
        "--max-time".into(),
        "120".into(),
        // HTTP-статус пишется в stdout (единственный вывод -w), тело
        // ответа — в project.zip (при ошибке там лежит JSON/HTML причины).
        "-w".into(),
        "%{http_code}".into(),
        url.to_string(),
        "-o".into(),
        zip_str,
    ];
    let spec = ProcessSpec {
        command: "curl".into(),
        args,
        working_dir: zip_path.parent().map(|p| p.to_path_buf()),
        env: None,
        timeout: Some(Duration::from_secs(180)),
        stdin: StdinMode::Null,
        // curl не нуждается в CI-переменных npm — их не инжектируем.
        ci_mode: false,
    };

    let (http_code, exit_code, stderr_tail) = match ProcessRunner::run(spec, sink).await {
        Ok(output) => (
            output.stdout_tail.trim().parse::<u32>().unwrap_or(0),
            None,
            output.stderr_tail,
        ),
        Err(error) => {
            let http_code = error.stdout_tail.trim().parse::<u32>().unwrap_or(0);
            let stderr_tail = error.stderr_tail;
            match &error.kind {
                ProcessErrorKind::Spawn { source } => {
                    return Err(format!(
                        "Spring Initializr отказал: failed to spawn curl: {}",
                        source
                    ));
                }
                ProcessErrorKind::Wait { source } => {
                    return Err(format!(
                        "Spring Initializr отказал: failed to run curl: {}",
                        source
                    ));
                }
                ProcessErrorKind::Timeout { timeout_secs } => {
                    return Err(format!(
                        "Spring Initializr отказал: timed out after {} seconds",
                        timeout_secs
                    ));
                }
                ProcessErrorKind::Exit { code } => (http_code, Some(code.clone()), stderr_tail),
                ProcessErrorKind::ReadOutput { stream, source } => {
                    return Err(format!(
                        "Spring Initializr отказал: failed to read {stream}: {}",
                        source
                    ));
                }
            }
        }
    };

    // HTTP-статус от curl (-w "%{http_code}"): при любом HTTP-ответе
    // (включая 4xx/5xx) процесс завершается успешно, статус — в stdout.
    if http_code != 0 {
        if http_code != 200 {
            // Статус != 200: в project.zip лежит тело ошибки Initializr
            // (JSON с полем message или HTML). Извлекаем причину, НЕ сохраняем
            // сломанный архив (удаляем) и прерываем генерацию.
            let text = match std::fs::read(zip_path) {
                Ok(bytes) if !bytes.is_empty() => extract_error_text(&bytes),
                _ => format!("HTTP request failed with status {}", http_code),
            };
            let _ = std::fs::remove_file(zip_path);
            return Err(format!("Spring Initializr error: {}", text));
        }
        return Ok(());
    }

    // curl не дождался HTTP-ответа (недоступный хост, обрыв соединения).
    let tail = stderr_tail.trim();
    let reason = if tail.is_empty() {
        format!(
            "HTTP request failed (exit code {})",
            exit_code.unwrap_or_else(|| "-1".into())
        )
    } else {
        truncate(tail, 500)
    };
    Err(format!("Spring Initializr отказал: {}", reason))
}

/// Настоящий ZIP-архив? Проверяем магические байты (PK\x03\x04 — обычный
/// архив, PK\x05\x06 — пустой архив). Тело HTTP-ошибки (JSON/HTML) сюда
/// не подходит — это и есть детектор «ответ был не архивом».
fn is_zip_archive(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && (bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06"))
}

/// Извлечь человекочитаемый текст ошибки из тела HTTP-ответа Initializr.
/// JSON: {"status":400,"error":"Bad Request","message":"Invalid dependency..."}
/// → возвращается именно message (подставляется в «Spring Initializr
/// отказал: <message>»); иначе — очищенный сырой текст (HTML и т.п.).
fn extract_error_text(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(message) = value.get("message").and_then(|v| v.as_str()) {
            let message = message.trim();
            if !message.is_empty() {
                return message.to_string();
            }
        }
    }
    // HTML или сырой текст: схлопываем пробелы и управляющие символы.
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
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

/// URL к Spring Initializr. ВСЕ query-параметры (name, groupId, artifactId,
/// dependencies) percent-кодируются через urlencode: пробелы, кавычки и `&`
/// внутри значений не ломают ни сам запрос, ни передачу URL в командную
/// строку curl.
fn spring_starter_url(project_name: &str, deps: &str) -> String {
    format!(
        "https://start.spring.io/starter.zip?name={}&groupId={}&artifactId={}&dependencies={}",
        urlencode(project_name),
        urlencode("com.example"),
        urlencode(project_name),
        urlencode(deps)
    )
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

/// Кроссплатформенный запуск команды реализован в engine::process
/// (ProcessRunner::run) — генераторы используют его через run_cli_process
/// и download_starter. Отдельной логики спавна здесь нет.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_error_text_parses_initializr_json_message() {
        let body = br#"{"timestamp":"2026-08-13T00:00:00.000+00:00","status":400,"error":"Bad Request","message":"Invalid dependency 'web' for type 'maven-project'","path":"/starter.zip"}"#;
        // Из тела достаётся именно JSON-поле message — оно подставляется в
        // «Spring Initializr отказал: <message>».
        let text = extract_error_text(body);
        assert_eq!(
            text, "Invalid dependency 'web' for type 'maven-project'",
            "message извлекается без статуса/error: {text}"
        );
    }

    #[test]
    fn extract_error_text_falls_back_to_raw_html() {
        let body = b"<html><body>Spring Initializr is temporarily down</body></html>";
        let text = extract_error_text(body);
        assert!(
            text.contains("Spring Initializr is temporarily down"),
            "{text}"
        );
    }

    #[test]
    fn extract_error_text_ignores_empty_message() {
        // message пустой/отсутствует — в дело идёт сырое тело (статус не
        // выводится как «причина»).
        let body = br#"{"status":400,"error":"Bad Request","message":""}"#;
        let text = extract_error_text(body);
        assert!(
            !text.contains("HTTP 400"),
            "пустой message не превращается в заглушку со статусом: {text}"
        );
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
    fn spring_url_escapes_all_query_params() {
        // Пробелы, запятые и спецсимволы в name/dependencies не попадают в
        // URL сырыми: каждый параметр percent-кодируется.
        let url = spring_starter_url("my app", "web,dev-tools");
        assert!(url.contains("name=my%20app"), "{url}");
        assert!(url.contains("artifactId=my%20app"), "{url}");
        assert!(url.contains("dependencies=web%2Cdev-tools"), "{url}");
        assert!(url.contains("groupId=com.example"), "{url}");
        assert!(!url.contains(' '), "URL без сырых пробелов: {url}");
        // Кавычки и амперсанд в имени проекта — тоже под urlencode.
        let url2 = spring_starter_url("a\"b&c", "web");
        assert!(url2.contains("name=a%22b%26c"), "{url2}");
    }

    #[test]
    fn cli_generator_rejects_config_without_command() {
        let gen = CliGenerator;
        let ctx = WizardContext::default();
        let result =
            tokio_test_block_on(gen.generate(&ctx, Path::new("."), &serde_json::json!({})));
        assert!(result.is_err(), "конфиг без command обязан падать");
    }

    #[test]
    fn host_tool_check_reports_missing_tools() {
        // Отсутствующий инструмент обязателен → Err с именем инструмента
        // (где/которые-поиск в PATH детерминированно не находит).
        let gen = HostToolCheckGenerator;
        let ctx = WizardContext::default();
        let result = tokio_test_block_on(gen.generate(
            &ctx,
            Path::new("."),
            &serde_json::json!({
                "tools": ["definitely-not-a-real-tool-xyz"],
                "mode": "all"
            }),
        ));
        let err = result.expect_err("инструмент отсутствует — проверка обязана упасть");
        assert!(
            err.contains("definitely-not-a-real-tool-xyz"),
            "ошибка называет инструмент: {err}"
        );
    }

    #[test]
    fn host_tool_check_rejects_empty_config() {
        let gen = HostToolCheckGenerator;
        let ctx = WizardContext::default();
        let result =
            tokio_test_block_on(gen.generate(&ctx, Path::new("."), &serde_json::json!({})));
        assert!(result.is_err(), "конфиг без tools обязан падать");
    }

    #[test]
    fn python_venv_generator_rejects_relative_and_outside_paths() {
        // venv обязан лежать внутри проекта: относительный путь разрешается
        // от текущего каталога (как это делает executor), поэтому "venv" у
        // временного проекта — выход за корень; абсолютный путь вне проекта
        // отклоняется явно.
        let gen = PythonVenvGenerator;
        let ctx = WizardContext::default();
        let project = temp_test_dir("pyvenv_guard");

        let relative = tokio_test_block_on(gen.generate(
            &ctx,
            &project,
            &serde_json::json!({ "python": "python3", "venv_path": "venv" }),
        ));
        assert!(
            relative
                .expect_err("относительный путь вне проекта обязан отклоняться")
                .contains("outside the project root"),
            "ошибка про выход за корень проекта"
        );

        let outside = project.parent().unwrap().join("elsewhere").join("venv");
        let result = tokio_test_block_on(gen.generate(
            &ctx,
            &project,
            &serde_json::json!({
                "python": "python3",
                "venv_path": outside.to_string_lossy(),
            }),
        ));
        assert!(
            result
                .expect_err("путь вне проекта обязан отклоняться")
                .contains("outside the project root"),
            "ошибка про выход за корень проекта"
        );

        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn python_venv_create_args_clear_only_for_existing_venv() {
        // --clear удаляет содержимое каталога: применяем его ТОЛЬКО когда
        // там уже лежит venv (pyvenv.cfg), чтобы не стереть чужую папку.
        let dir = temp_test_dir("pyvenv_clear");
        let venv = dir.join("venv");
        std::fs::create_dir_all(&venv).unwrap();

        let args = venv_create_args(&venv, venv.to_string_lossy().as_ref(), true);
        assert!(!args.iter().any(|a| a == "--clear"), "{args:?}");
        assert!(args.iter().any(|a| a == "--without-pip"), "{args:?}");
        assert_eq!(args.last().unwrap(), venv.to_string_lossy().as_ref());
        assert_eq!(&args[..2], &["-m".to_string(), "venv".to_string()]);

        std::fs::write(venv.join("pyvenv.cfg"), "home = /usr/bin").unwrap();
        let args = venv_create_args(&venv, venv.to_string_lossy().as_ref(), false);
        assert!(args.iter().any(|a| a == "--clear"), "{args:?}");
        assert!(!args.iter().any(|a| a == "--without-pip"), "{args:?}");

        let vargs = venv_virtualenv_args(&venv, venv.to_string_lossy().as_ref());
        assert_eq!(&vargs[..2], &["-m".to_string(), "virtualenv".to_string()]);
        assert!(vargs.iter().any(|a| a == "--clear"), "{vargs:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wheel_version_key_is_numeric_not_lexicographic() {
        // pip-26.10 старше pip-26.2 лексикографически, но НОВЕЕ по версии.
        assert!(wheel_version_key("pip-26.10-py3-none-any.whl") > wheel_version_key("pip-26.2-py3-none-any.whl"));
        assert!(wheel_version_key("pip-26.2.1-py3-none-any.whl") > wheel_version_key("pip-26.2-py3-none-any.whl"));
        assert!(wheel_version_key("pip-24.0-py3-none-any.whl") > wheel_version_key("pip-9.0.3-py3-none-any.whl"));
    }

    #[test]
    fn python_venv_failure_message_has_instructions_and_notes() {
        // Сообщение об ошибке обязано содержать точную команду установки
        // python3-venv, путь venv и диагностику попыток.
        let venv = Path::new("/tmp/proj/venv");
        let notes = vec!["`python3 -m venv` failed: exit 1".to_string()];
        let message = venv_failure_message(
            "definitely-not-a-real-python-xyz",
            venv,
            Path::new("/tmp/proj"),
            &notes,
        );
        assert!(message.contains("apt install python3-venv"), "{message}");
        assert!(message.contains("get-pip.py"), "{message}");
        assert!(message.contains("/tmp/proj/venv"), "{message}");
        assert!(message.contains("`python3 -m venv` failed"), "{message}");
    }

    /// Сквозная проверка на реальном интерпретаторе: генератор обязан создать
    /// venv с рабочим pip даже на дистрибутиве без python3-venv (Debian/Ubuntu
    /// без ensurepip — путь через get-pip.py, нужна сеть), а повторный прогон
    /// — переиспользовать здоровое окружение. Запускать вручную:
    /// `cargo test --lib python_venv_generator_end_to_end -- --ignored`.
    #[test]
    #[ignore = "requires a real python3 and network access (pip bootstrap)"]
    fn python_venv_generator_end_to_end_bootstraps_pip() {
        let dir = temp_test_dir("pyvenv_e2e");
        let venv = dir.join("venv");
        let python = if cfg!(target_os = "windows") {
            "python"
        } else {
            "python3"
        };
        let gen = PythonVenvGenerator;
        let ctx = WizardContext::default();
        let config = serde_json::json!({
            "python": python,
            "venv_path": venv.to_string_lossy(),
        });

        tokio_test_block_on(gen.generate(&ctx, &dir, &config))
            .expect("venv обязан создаться с рабочим pip");

        let pip = std::process::Command::new(python_venv_binary(&venv))
            .args(["-m", "pip", "--version"])
            .output()
            .expect("интерпретатор venv обязан запускаться");
        assert!(
            pip.status.success(),
            "pip недоступен: {}{}",
            String::from_utf8_lossy(&pip.stdout),
            String::from_utf8_lossy(&pip.stderr)
        );

        // Идемпотентность: здоровое окружение переиспользуется.
        let again = tokio_test_block_on(gen.generate(&ctx, &dir, &config))
            .expect("повторный прогон обязан переиспользовать venv");
        assert!(
            again.message.contains("Reused"),
            "повторный прогон не пересоздаёт venv: {}",
            again.message
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tokio_test_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(fut)
    }

    /// Уникальная временная папка для тестов (по имени тега).
    fn temp_test_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_gen_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn scaffold_step_config_parses_full_config() {
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "__TARGET__", "--template", "react-ts"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "working_dir": ".",
            "temp_dir_allowed": false,
            "expected_outputs": ["package.json", "src"],
            "interactive": [
                {"trigger": "pm?", "response_type": {"Text": "npm"}},
                {"trigger": "git?", "response_type": "n"},
            ],
            "timeout_secs": 90,
        });
        let parsed = ScaffoldStepConfig::from_json(&cfg).expect("конфиг должен парситься");
        assert_eq!(parsed.command, "npx");
        assert_eq!(
            parsed.args,
            vec!["create-vite@latest", "__TARGET__", "--template", "react-ts"]
        );
        assert_eq!(parsed.capability, ScaffoldCapability::CreatesNamedDirectory);
        assert_eq!(parsed.target_dir, "frontend");
        assert_eq!(parsed.working_dir.as_deref(), Some("."));
        assert_eq!(parsed.temp_dir_allowed, Some(false));
        assert_eq!(parsed.expected_outputs, vec!["package.json", "src"]);
        assert_eq!(parsed.interactive.len(), 2, "интерактивные ответы парсятся");
        assert!(
            matches!(parsed.interactive[0].response_type, ResponseType::Text(ref t) if t == "npm")
        );
        assert!(matches!(
            parsed.interactive[1].response_type,
            ResponseType::Confirm(false)
        ));
        assert_eq!(parsed.timeout_secs, 90);
    }

    #[test]
    fn scaffold_step_config_applies_defaults() {
        let cfg = serde_json::json!({
            "command": "flutter",
            "args": ["create", "__TARGET__"],
            "capability": "creates_project_and_may_prompt",
        });
        let parsed = ScaffoldStepConfig::from_json(&cfg).expect("конфиг должен парситься");
        assert_eq!(parsed.target_dir, ".");
        assert_eq!(parsed.working_dir, None);
        assert_eq!(
            parsed.temp_dir_allowed, None,
            "по умолчанию — по способности"
        );
        assert!(parsed.expected_outputs.is_empty());
        assert!(parsed.interactive.is_empty());
        assert_eq!(parsed.timeout_secs, 600);
    }

    #[test]
    fn scaffold_step_config_requires_capability() {
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "__TARGET__"],
        });
        let err = ScaffoldStepConfig::from_json(&cfg).unwrap_err();
        assert!(err.contains("missing 'capability'"), "{err}");
    }

    #[test]
    fn scaffold_step_config_rejects_unknown_capability() {
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "__TARGET__"],
            "capability": "creates_magic_dir",
        });
        let err = ScaffoldStepConfig::from_json(&cfg).unwrap_err();
        assert!(
            err.contains("unknown 'capability'") && err.contains("creates_magic_dir"),
            "{err}"
        );
    }

    #[test]
    fn scaffold_step_config_parses_all_capabilities() {
        for (name, expected) in [
            (
                "creates_named_directory",
                ScaffoldCapability::CreatesNamedDirectory,
            ),
            (
                "creates_in_current_directory",
                ScaffoldCapability::CreatesInCurrentDirectory,
            ),
            (
                "creates_project_and_may_prompt",
                ScaffoldCapability::CreatesProjectAndMayPrompt,
            ),
            (
                "generates_root_shell",
                ScaffoldCapability::GeneratesRootShell,
            ),
            (
                "does_not_create_a_project",
                ScaffoldCapability::DoesNotCreateAProject,
            ),
        ] {
            let cfg = serde_json::json!({
                "command": "x",
                "args": [],
                "capability": name,
            });
            let parsed =
                ScaffoldStepConfig::from_json(&cfg).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(parsed.capability, expected, "{name}");
        }
    }

    #[test]
    fn scaffold_capability_supports_temp_dir() {
        // Временная папка (temp+move) разрешена только там, где CLI сам
        // создаёт именованную папку и её содержимое можно перенести.
        assert!(ScaffoldCapability::CreatesNamedDirectory.supports_temp_dir());
        assert!(ScaffoldCapability::CreatesProjectAndMayPrompt.supports_temp_dir());
        assert!(!ScaffoldCapability::CreatesInCurrentDirectory.supports_temp_dir());
        assert!(!ScaffoldCapability::GeneratesRootShell.supports_temp_dir());
        assert!(!ScaffoldCapability::DoesNotCreateAProject.supports_temp_dir());
    }

    #[test]
    fn scaffold_generator_rejects_config_without_capability() {
        let gen = ScaffoldGenerator;
        let ctx = WizardContext::default();
        // нет capability — конфиг обязан падать без запуска CLI
        let cfg = serde_json::json!({ "command": "npx", "args": ["create-vite@latest", "app"] });
        let res = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &cfg));
        assert!(res.is_err(), "конфиг без capability обязан падать");

        // named-directory способность без плейсхолдера в args
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "app"],
            "capability": "creates_named_directory",
            "target_dir": ".",
        });
        let res = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &cfg));
        assert!(
            res.is_err(),
            "named-directory без плейсхолдера обязан падать"
        );
    }

    // ==================== fake CLI: выполнение по способностям =============

    /// Фейковые CLI-скаффолдеры (.cmd на Windows, sh на Unix) в temp-папке:
    ///   - fake-create: создаёт ПОДПАПКУ <name> с package.json внутри;
    ///   - fake-matroshka: создаёт <name>/<name>/package.json (вложенная папка);
    ///   - fake-create-empty: создаёт только <name>, без package.json;
    ///   - fake-inplace: пишет package.json в ТЕКУЩИЙ каталог;
    ///   - fake-fail: завершается с exit 1;
    ///   - fake-nothing: exit 0 без побочных эффектов.
    fn fake_cli_dir(tag: &str) -> std::path::PathBuf {
        let dir = temp_test_dir(&format!("{}_clis", tag));
        if cfg!(target_os = "windows") {
            std::fs::write(
                dir.join("fake-create.cmd"),
                "@echo off\r\nmkdir \"%1\"\r\necho {}> \"%1\\package.json\"\r\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-matroshka.cmd"),
                "@echo off\r\nmkdir \"%1\\%1\"\r\necho {}> \"%1\\%1\\package.json\"\r\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-create-empty.cmd"),
                "@echo off\r\nmkdir \"%1\"\r\n",
            )
            .unwrap();
            // Ведёт себя как RN CLI: существующий каталог — отказ (exit 1).
            std::fs::write(
                dir.join("fake-strict-create.cmd"),
                "@echo off\r\nif exist \"%1\" exit /b 1\r\nmkdir \"%1\"\r\necho {}> \"%1\\package.json\"\r\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-inplace.cmd"),
                "@echo off\r\necho {}> package.json\r\n",
            )
            .unwrap();
            std::fs::write(dir.join("fake-fail.cmd"), "@echo off\r\nexit /b 1\r\n").unwrap();
            std::fs::write(dir.join("fake-nothing.cmd"), "@echo off\r\n").unwrap();
        } else {
            std::fs::write(
                dir.join("fake-create.sh"),
                "#!/bin/sh\nmkdir \"$1\"\necho '{}' > \"$1/package.json\"\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-matroshka.sh"),
                "#!/bin/sh\nmkdir -p \"$1/$1\"\necho '{}' > \"$1/$1/package.json\"\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-create-empty.sh"),
                "#!/bin/sh\nmkdir \"$1\"\n",
            )
            .unwrap();
            // Ведёт себя как RN CLI: существующий каталог — отказ (exit 1).
            std::fs::write(
                dir.join("fake-strict-create.sh"),
                "#!/bin/sh\nif [ -e \"$1\" ]; then exit 1; fi\nmkdir \"$1\"\necho '{}' > \"$1/package.json\"\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("fake-inplace.sh"),
                "#!/bin/sh\necho '{}' > package.json\n",
            )
            .unwrap();
            std::fs::write(dir.join("fake-fail.sh"), "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::write(dir.join("fake-nothing.sh"), "#!/bin/sh\n").unwrap();
            // CLI запускается через `sh -c '<путь> <args>'`: exec требует
            // права на исполнение, иначе скрипт умирает с "Permission
            // denied" ещё до запуска (на Windows .cmd это не важно).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for entry in std::fs::read_dir(&dir).unwrap() {
                    let path = entry.unwrap().path();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                }
            }
        }
        dir
    }

    fn fake_cli(dir: &Path, name: &str) -> String {
        let ext = if cfg!(target_os = "windows") {
            ".cmd"
        } else {
            ".sh"
        };
        dir.join(format!("{}{}", name, ext))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn scaffold_named_directory_uses_temp_and_merges() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_named");
        let clis = fake_cli_dir("scaffold_named");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-create"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("scaffold должен пройти");

        // package.json лежит В frontend/ — без матрёшки frontend/<name>/
        assert!(
            project.join("frontend/package.json").exists(),
            "проект в frontend/"
        );
        assert_eq!(report.created_files, vec!["frontend/package.json"]);
        // временная папка удалена, матрёшки нет
        assert!(!project.join("frontend/frontend").exists(), "нет матрёшки");
        let leftovers: Vec<_> = std::fs::read_dir(&project)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("temp_"))
            .collect();
        assert!(leftovers.is_empty(), "temp-папки удаляются: {leftovers:?}");
        assert!(
            report
                .skipped_dependent_steps
                .iter()
                .any(|s| s.contains("nested")),
            "flatten-скип фиксируется в отчёте: {:?}",
            report.skipped_dependent_steps
        );
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_named_directory_without_temp_lets_cli_create_target() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_notemp");
        let clis = fake_cli_dir("scaffold_notemp");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-create"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "temp_dir_allowed": false,
            "expected_outputs": ["package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("scaffold должен пройти");
        assert!(project.join("frontend/package.json").exists());
        // CLI создал сам target — merge пропущен и это зафиксировано
        assert!(
            report
                .skipped_dependent_steps
                .iter()
                .any(|s| s.contains("merge")),
            "{:?}",
            report.skipped_dependent_steps
        );
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_without_temp_stages_around_existing_target_collision() {
        // Регрессия RN init: каталог назначения уже существует и не пуст
        // (повторный запуск, чужой каркас в frontend/). CLI, который
        // отказывается писать в существующий каталог (RN CLI), обязан
        // получить свежий staging-родитель с правильным именем проекта,
        // а результат — слиться в target без потери чужих файлов.
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_collision");
        let clis = fake_cli_dir("scaffold_collision");
        let ctx = WizardContext::default();
        // Чужой файл в целевом каталоге — обязан выжить после merge.
        std::fs::create_dir_all(project.join("frontend")).unwrap();
        std::fs::write(project.join("frontend").join("keep.txt"), "keep").unwrap();

        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-strict-create"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "temp_dir_allowed": false,
            "expected_outputs": ["package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("коллизия не должна валить скаффолд");

        assert!(
            project.join("frontend/package.json").exists(),
            "результат staging-скаффолда переносится в frontend/"
        );
        assert!(
            project.join("frontend/keep.txt").exists(),
            "существующие файлы target не теряются (merge)"
        );
        assert!(
            !project.join("frontend/frontend").exists(),
            "матрёшки быть не должно"
        );
        assert!(
            report
                .validation_warnings
                .iter()
                .any(|w| w.contains("staging")),
            "факт staging-запуска фиксируется: {:?}",
            report.validation_warnings
        );
        let leftovers: Vec<_> = std::fs::read_dir(&project)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("temp_"))
            .collect();
        assert!(leftovers.is_empty(), "staging-папки удаляются: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_flattens_nested_folder_with_warning() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_matroshka");
        let clis = fake_cli_dir("scaffold_matroshka");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-matroshka"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("scaffold должен пройти");
        assert!(
            project.join("frontend/package.json").exists(),
            "вложенная папка распрямлена"
        );
        assert!(
            report
                .validation_warnings
                .iter()
                .any(|w| w.contains("nested")),
            "факт матрёшки фиксируется предупреждением: {:?}",
            report.validation_warnings
        );
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_in_current_directory_writes_inside_target() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_inplace");
        let clis = fake_cli_dir("scaffold_inplace");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-inplace"),
            "args": ["__TARGET__"],
            "capability": "creates_in_current_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("scaffold должен пройти");
        assert!(
            project.join("frontend/package.json").exists(),
            "CLI пишет ВНУТРИ target"
        );
        assert_eq!(report.created_files, vec!["frontend/package.json"]);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_missing_postcondition_fails_with_all_paths() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_postcond");
        let clis = fake_cli_dir("scaffold_postcond");
        let ctx = WizardContext::default();
        // CLI создаёт папку, но не кладёт ожидаемые файлы
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-create-empty"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json", "src/index.ts"],
        });
        let err = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect_err("недостающие пост-условия обязаны падать");
        assert!(
            err.contains("package.json"),
            "в ошибке все недостающие пути: {err}"
        );
        assert!(
            err.contains("src/index.ts"),
            "в ошибке все недостающие пути: {err}"
        );
        assert!(
            err.contains("frontend"),
            "ошибка указывает каталог назначения: {err}"
        );
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_cli_exit_zero_without_creating_dir_fails() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_nodir");
        let clis = fake_cli_dir("scaffold_nodir");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-nothing"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json"],
        });
        let err = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect_err("exit 0 без созданной папки обязан падать");
        assert!(
            err.contains("did not create the expected directory"),
            "{err}"
        );
        assert!(
            err.contains("frontend"),
            "ошибка называет ожидаемую папку: {err}"
        );
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_cli_failure_is_reported() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_cli_fail");
        let clis = fake_cli_dir("scaffold_cli_fail");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-fail"),
            "args": ["__TARGET__"],
            "capability": "creates_named_directory",
            "target_dir": "frontend",
            "expected_outputs": ["package.json"],
        });
        let err = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect_err("exit 1 обязан падать");
        assert!(err.to_lowercase().contains("exit"), "{err}");
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn scaffold_does_not_create_project_runs_in_working_dir() {
        let gen = ScaffoldGenerator;
        let project = temp_test_dir("scaffold_doesnot");
        let clis = fake_cli_dir("scaffold_doesnot");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "command": fake_cli(&clis, "fake-inplace"),
            "args": ["init"],
            "capability": "does_not_create_a_project",
            "working_dir": "backend",
            "expected_outputs": ["backend/package.json"],
        });
        let report = tokio_test_block_on(gen.generate(&ctx, &project, &cfg))
            .expect("scaffold должен пройти");
        assert!(
            project.join("backend/package.json").exists(),
            "CLI работает в рабочей директории, а не в корне"
        );
        assert_eq!(report.created_files, vec!["backend/package.json"]);
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&clis);
    }

    #[test]
    fn unique_temp_name_appends_suffix_on_collision() {
        let dir = temp_test_dir("temp_name");
        assert_eq!(unique_temp_name(&dir, "frontend"), "temp_frontend");
        std::fs::create_dir_all(dir.join("temp_frontend")).unwrap();
        assert_eq!(unique_temp_name(&dir, "frontend"), "temp_frontend_2");
        std::fs::create_dir_all(dir.join("temp_frontend_2")).unwrap();
        assert_eq!(unique_temp_name(&dir, "frontend"), "temp_frontend_3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_dir_contents_moves_nested_hidden_tree() {
        let src = temp_test_dir("merge_src");
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join("package.json"), "{}").unwrap();
        std::fs::write(src.join(".git/config"), "src-config").unwrap();
        let dst = temp_test_dir("merge_dst");
        std::fs::write(dst.join("keep.txt"), "keep").unwrap();

        merge_dir_contents(&src, &dst).expect("merge должен пройти");

        assert!(dst.join("package.json").exists(), "файл из src переносится");
        assert_eq!(
            std::fs::read_to_string(dst.join(".git/config")).unwrap(),
            "src-config",
            "скрытый файл переносится (перезапись)"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("keep.txt")).unwrap(),
            "keep"
        );
        assert!(
            !src.join("package.json").exists(),
            "исходный файл удаляется после копирования"
        );
        assert!(!src.join(".git").exists(), "каталог переносится целиком");
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn merge_json_keeps_existing_keys_and_merges_objects() {
        let base = serde_json::json!({
            "typescript.tsdk": "node_modules/typescript/lib",
            "[typescript]": { "editor.formatOnSave": false }
        });
        let extra = serde_json::json!({
            "editor.formatOnSave": true,
            "[typescript]": { "editor.defaultFormatter": "esbenp.prettier-vscode" }
        });
        let merged = merge_json(&base, &extra);
        assert_eq!(
            merged["typescript.tsdk"], "node_modules/typescript/lib",
            "ключ CLI побеждает"
        );
        assert_eq!(
            merged["editor.formatOnSave"], true,
            "наши ключи добавляются"
        );
        assert_eq!(
            merged["[typescript]"]["editor.formatOnSave"], false,
            "вложенный ключ CLI побеждает"
        );
        assert_eq!(
            merged["[typescript]"]["editor.defaultFormatter"],
            "esbenp.prettier-vscode"
        );
    }

    #[test]
    fn vscode_merge_keeps_cli_settings_and_adds_ours() {
        let gen = VsCodeMergeGenerator;
        let dir = temp_test_dir("vscode_merge");
        let vscode = dir.join(".vscode");
        std::fs::create_dir_all(&vscode).unwrap();
        std::fs::write(
            vscode.join("settings.json"),
            r#"{ "typescript.tsdk": "node_modules/typescript/lib" }"#,
        )
        .unwrap();
        std::fs::write(
            vscode.join("extensions.json"),
            r#"{ "recommendations": ["dbaeumer.vscode-eslint"] }"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let cfg = serde_json::json!({ "lang": "typescript", "dirs": [".", "frontend"] });
        let report =
            tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("merge должен пройти");

        // Ключ CLI сохранился, наши ключи добавлены
        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(vscode.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(merged["typescript.tsdk"], "node_modules/typescript/lib");
        assert_eq!(merged["editor.formatOnSave"], true);

        // Рекомендации объединяются без дубликатов
        let exts: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(vscode.join("extensions.json")).unwrap())
                .unwrap();
        let recs = exts["recommendations"].as_array().unwrap();
        assert!(
            recs.iter().any(|r| r == "dbaeumer.vscode-eslint"),
            "{recs:?}"
        );
        assert!(
            recs.iter().any(|r| r == "esbenp.prettier-vscode"),
            "{recs:?}"
        );

        // В frontend/ (без .vscode от CLI) папка НЕ создаётся
        assert!(
            !dir.join("frontend").join(".vscode").exists(),
            "не-корневой .vscode без CLI не создаётся"
        );
        assert!(report
            .modified_files
            .iter()
            .any(|f| f == "./.vscode/settings.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vscode_merge_writes_configured_python_interpreter() {
        // Движок передаёт канонический интерпретатор (venv проекта, в split —
        // <segment>/venv) — VS Code не должен указывать на несуществующий
        // `.venv` из исторического шаблона.
        let gen = VsCodeMergeGenerator;
        let dir = temp_test_dir("vscode_py_interp");
        let ctx = WizardContext::default();

        // python — главный язык: интерпретатор из конфига побеждает.
        let cfg = serde_json::json!({
            "lang": "python",
            "dirs": ["."],
            "python_interpreter": "${workspaceFolder}/backend/venv/bin/python",
        });
        tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("merge должен пройти");
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(".vscode/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings["python.defaultInterpreterPath"],
            "${workspaceFolder}/backend/venv/bin/python"
        );

        // python не главный язык: ключ всё равно добавляется.
        let dir2 = temp_test_dir("vscode_py_interp2");
        let cfg = serde_json::json!({
            "lang": "typescript",
            "dirs": ["."],
            "python_interpreter": "${workspaceFolder}/venv/bin/python",
        });
        tokio_test_block_on(gen.generate(&ctx, &dir2, &cfg)).expect("merge должен пройти");
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir2.join(".vscode/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings["python.defaultInterpreterPath"],
            "${workspaceFolder}/venv/bin/python"
        );
        assert_eq!(settings["editor.formatOnSave"], true);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn vscode_merge_writes_into_subdir_created_by_cli() {
        let gen = VsCodeMergeGenerator;
        let dir = temp_test_dir("vscode_merge_sub");
        let sub_vscode = dir.join("frontend").join(".vscode");
        std::fs::create_dir_all(&sub_vscode).unwrap();
        std::fs::write(
            sub_vscode.join("settings.json"),
            r#"{ "typescript.tsdk": "node_modules/typescript/lib" }"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let cfg = serde_json::json!({ "lang": "typescript", "dirs": [".", "frontend"] });
        tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("merge должен пройти");

        let merged: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(sub_vscode.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(merged["typescript.tsdk"], "node_modules/typescript/lib");
        assert_eq!(
            merged["editor.formatOnSave"], true,
            "наши настройки добавлены в подкаталог CLI"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_patches_build_and_identifier() {
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config");
        let src_tauri = dir.join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
        // Фронтенд-каталог обязан существовать (его создал фронтенд-скаффолд
        // ДО tauri init — генератор валидирует пути перед патчем).
        std::fs::create_dir_all(dir.join("frontend")).unwrap();
        std::fs::write(dir.join("frontend").join("package.json"), "{}").unwrap();
        std::fs::write(
            src_tauri.join("tauri.conf.json"),
            r#"{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "myapp",
  "version": "0.1.0",
  "identifier": "com.fallback",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  }
}"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "frontend_dir": "frontend",
            "dev_url": "http://localhost:5173",
            "before_dev_command": "npm --prefix frontend run dev",
            "before_build_command": "npm --prefix frontend run build",
            "identifier": "com.myapp",
        });
        let report =
            tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("патч должен пройти");
        assert_eq!(
            report.modified_files,
            vec!["src-tauri/tauri.conf.json".to_string()]
        );

        let patched: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src_tauri.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(patched["build"]["frontendDist"], "../frontend/dist");
        assert_eq!(
            patched["build"]["beforeDevCommand"],
            "npm --prefix frontend run dev"
        );
        assert_eq!(
            patched["build"]["beforeBuildCommand"],
            "npm --prefix frontend run build"
        );
        assert_eq!(patched["build"]["devUrl"], "http://localhost:5173");
        assert_eq!(patched["identifier"], "com.myapp");
        assert!(
            patched["build"].get("distDir").is_none(),
            "v2-конфиг не получает legacy distDir"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_rewrites_legacy_dist_dir() {
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config_v1");
        let src_tauri = dir.join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
        std::fs::create_dir_all(dir.join("frontend")).unwrap();
        std::fs::write(dir.join("frontend").join("package.json"), "{}").unwrap();
        std::fs::write(
            src_tauri.join("tauri.conf.json"),
            r#"{
  "build": { "distDir": "../dist", "devPath": "http://localhost:1420" },
  "package": { "productName": "myapp" }
}"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "frontend_dir": "frontend",
            "dev_url": "http://localhost:5173",
            "before_dev_command": "npm --prefix frontend run dev",
            "before_build_command": "npm --prefix frontend run build",
            "identifier": "com.myapp",
        });
        tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("патч должен пройти");

        let patched: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src_tauri.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            patched["build"]["distDir"], "../frontend/dist",
            "legacy distDir переписывается"
        );
        assert_eq!(patched["build"]["frontendDist"], "../frontend/dist");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_fails_without_tauri_conf() {
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config_missing");
        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "frontend_dir": "frontend",
            "before_dev_command": "npm --prefix frontend run dev",
            "before_build_command": "npm --prefix frontend run build",
            "identifier": "com.myapp",
        });
        let res = tokio_test_block_on(gen.generate(&ctx, &dir, &cfg));
        assert!(res.is_err(), "без tauri.conf.json генератор обязан падать");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_honors_tauri_dir_and_frontend_dist_overrides() {
        // Strict Subdir Mandate: конфиг лежит в backend/src-tauri/, dist —
        // ../../frontend/dist (путь передаёт движок, генератор не считает сам)
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config_seg");
        let src_tauri = dir.join("backend").join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
        std::fs::create_dir_all(dir.join("frontend")).unwrap();
        std::fs::write(dir.join("frontend").join("package.json"), "{}").unwrap();
        std::fs::write(
            src_tauri.join("tauri.conf.json"),
            r#"{
  "identifier": "com.fallback",
  "build": { "frontendDist": "../dist" }
}"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let cfg = serde_json::json!({
            "frontend_dir": "frontend",
            "tauri_dir": "backend",
            "frontend_dist": "../../frontend/dist",
            "before_dev_command": "npm --prefix ../frontend run dev",
            "before_build_command": "npm --prefix ../frontend run build",
            "identifier": "com.myapp",
        });
        let report =
            tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("патч должен пройти");
        assert_eq!(
            report.modified_files,
            vec!["backend/src-tauri/tauri.conf.json".to_string()]
        );

        let patched: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src_tauri.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            patched["build"]["frontendDist"], "../../frontend/dist",
            "движок задаёт путь из backend/"
        );
        assert_eq!(
            patched["build"]["beforeDevCommand"],
            "npm --prefix ../frontend run dev"
        );
        assert_eq!(patched["identifier"], "com.myapp");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vscode_folders_merge_merges_inner_into_root_and_deletes_inner() {
        let gen = VsCodeFoldersMergeGenerator;
        let dir = temp_test_dir("vscode_folders");
        let root_vscode = dir.join(".vscode");
        std::fs::create_dir_all(&root_vscode).unwrap();
        std::fs::write(
            root_vscode.join("settings.json"),
            r#"{ "editor.tabSize": 4 }"#,
        )
        .unwrap();
        // CLI-конфиг frontend/: typescript.tsdk должен ПОБЕДИТЬ (как в
        // VsCodeMergeGenerator — конфиги CLI не затираются)
        let front_vscode = dir.join("frontend").join(".vscode");
        std::fs::create_dir_all(&front_vscode).unwrap();
        std::fs::write(
            front_vscode.join("settings.json"),
            r#"{ "typescript.tsdk": "node_modules/typescript/lib", "editor.tabSize": 2 }"#,
        )
        .unwrap();
        std::fs::write(
            front_vscode.join("extensions.json"),
            r#"{ "recommendations": ["dbaeumer.vscode-eslint"] }"#,
        )
        .unwrap();
        // backend/: только extensions.json
        let back_vscode = dir.join("backend").join(".vscode");
        std::fs::create_dir_all(&back_vscode).unwrap();
        std::fs::write(
            back_vscode.join("extensions.json"),
            r#"{ "recommendations": ["rust-lang.rust-analyzer"] }"#,
        )
        .unwrap();

        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &serde_json::json!({})))
            .expect("merge должен пройти");

        assert!(
            !front_vscode.exists(),
            "frontend/.vscode удаляется после слияния"
        );
        assert!(
            !back_vscode.exists(),
            "backend/.vscode удаляется после слияния"
        );
        assert!(
            report
                .modified_files
                .contains(&"frontend/.vscode/settings.json".to_string()),
            "{:?}",
            report.modified_files
        );
        assert!(
            report
                .modified_files
                .contains(&"frontend/.vscode/extensions.json".to_string()),
            "{:?}",
            report.modified_files
        );
        assert!(
            report
                .modified_files
                .contains(&"backend/.vscode/extensions.json".to_string()),
            "{:?}",
            report.modified_files
        );

        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root_vscode.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings["typescript.tsdk"], "node_modules/typescript/lib",
            "конфиг CLI побеждает"
        );
        assert_eq!(
            settings["editor.tabSize"], 2,
            "глубокое слияние: ключ CLI поверх корневого"
        );
        assert!(
            settings.get("editor.formatOnSave").is_none(),
            "наши настройки не добавляются — это не VsCodeMergeGenerator: {settings}"
        );

        let exts: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root_vscode.join("extensions.json")).unwrap(),
        )
        .unwrap();
        let recs = exts["recommendations"].as_array().unwrap();
        assert!(
            recs.iter().any(|r| r == "dbaeumer.vscode-eslint"),
            "{recs:?}"
        );
        assert!(
            recs.iter().any(|r| r == "rust-lang.rust-analyzer"),
            "{recs:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vscode_folders_merge_is_noop_without_inner_folders() {
        let gen = VsCodeFoldersMergeGenerator;
        let dir = temp_test_dir("vscode_folders_empty");
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &serde_json::json!({})))
            .expect("no-op не должен падать");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );
        assert!(
            !dir.join(".vscode").exists(),
            "корневой .vscode без содержимого не создаётся"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_accepts_valid_package_json() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_ok_pkg");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"app","dependencies":{"express":"^4.18.2","fastify":"^4.28.0"},"devDependencies":{"typescript":"^5.0.0"}}"#,
        )
        .unwrap();
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "package.json",
                "kind": "package_json",
                "required_dependencies": ["express", "typescript"],
            }),
        ))
        .expect("валидный package.json с нужными зависимостями обязан пройти");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_rejects_missing_dependency_in_package_json() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_bad_pkg");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"app","dependencies":{"lodash":"^4.0.0"}}"#,
        )
        .unwrap();
        let ctx = WizardContext::default();
        let err = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "package.json",
                "kind": "package_json",
                "required_dependencies": ["express"],
            }),
        ))
        .expect_err("отсутствие обязательной зависимости — ошибка шага");
        assert!(err.contains("express"), "{err}");
        assert!(
            err.contains("lodash"),
            "перечисляются реально задекларированные: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_rejects_invalid_json_manifest() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_broken_json");
        std::fs::write(dir.join("package.json"), "{\"name\": \"app\" trailing").unwrap();
        let ctx = WizardContext::default();
        let err = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "package.json",
                "kind": "package_json",
                "required_dependencies": ["express"],
            }),
        ))
        .expect_err("битый JSON — ошибка шага, а не молчаливый успех");
        assert!(err.contains("invalid JSON"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_validates_composer_json_require() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_composer");
        std::fs::write(
            dir.join("composer.json"),
            r#"{"require":{"laravel/framework":"^11.0"},"require-dev":{"phpunit/phpunit":"^10.0"}}"#,
        )
        .unwrap();
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "composer.json",
                "kind": "composer_json",
                "required_dependencies": ["laravel/framework"],
            }),
        ))
        .expect("composer.json с laravel/framework обязан пройти");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );
        let err = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "composer.json",
                "kind": "composer_json",
                "required_dependencies": ["symfony/framework-bundle"],
            }),
        ))
        .expect_err("generic-фолбэк composer.json не проходит для symfony");
        assert!(err.contains("symfony/framework-bundle"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_parses_requirements_txt() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_requirements");
        std::fs::write(
            dir.join("requirements.txt"),
            "fastapi[standard]\nuvicorn>=0.30\n# comment\n-r base.txt\nalembic==1.13.1\n",
        )
        .unwrap();
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "requirements.txt",
                "kind": "requirements_txt",
                "required_dependencies": ["fastapi", "uvicorn", "alembic"],
            }),
        ))
        .expect("requirements.txt с нужными пакетами обязан пройти");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_check_validates_pyproject_toml_and_cargo_toml() {
        let gen = ManifestCheckGenerator;
        let dir = temp_test_dir("manifest_toml");
        std::fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"app\"\ndependencies = [\n  \"fastapi[standard]>=0.110\",\n  \"uvicorn\",\n]\n",
        )
        .unwrap();
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "pyproject.toml",
                "kind": "pyproject_toml",
                "required_dependencies": ["fastapi", "uvicorn"],
            }),
        ))
        .expect("pyproject.toml с fastapi и uvicorn обязан пройти");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );

        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\n[dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\ntokio = \"1\"\n",
        )
        .unwrap();
        let report = tokio_test_block_on(gen.generate(
            &ctx,
            &dir,
            &serde_json::json!({
                "path": "Cargo.toml",
                "kind": "cargo_toml",
                "required_dependencies": ["serde", "tokio"],
            }),
        ))
        .expect("Cargo.toml с serde и tokio обязан пройти");
        assert!(
            report.modified_files.is_empty(),
            "{:?}",
            report.modified_files
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
