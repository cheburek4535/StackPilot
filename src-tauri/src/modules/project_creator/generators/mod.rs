use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command as TokioCommand;

use crate::modules::project_creator::engine::content;
use crate::modules::project_creator::models::*;

/// Плейсхолдер в args, на месте которого ScaffoldGenerator подставляет имя
/// создаваемой CLI-папки ("." в dot-режиме или временную папку в temp+move).
pub const SCAFFOLD_TARGET: &str = "__TARGET__";

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
        download_starter(&url, &zip_path).await?;

        // 2) Проверка HTTP-статуса на Rust: настоящий ZIP или тело ошибки
        //    Initializr? Если ответ не архив (JSON с ключом message, HTML) —
        //    генерация прерывается, сломанный архив не сохраняется.
        let bytes = std::fs::read(&zip_path).map_err(|e| {
            format!("Spring Initializr error: failed to read downloaded archive 'project.zip': {}", e)
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
        run_cli(unzip_cmd, &unzip_args, &target, None, 300).await?;

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
// ScaffoldGenerator — «умный» запуск CLI-скаффолдеров без матрёшек.
//
// Проблема: create-vite/create-next-app/nuxi init и т.п. создают проект В
// ПОДПАПКЕ с именем, переданным аргументом. Вызов из корня с именем проекта
// даёт матрёшку (testapp13/testapp13), а "." большинство CLI либо не умеет
// обрабатывать, либо требует подтверждения.
//
// Решение — Temp-to-Target (используется ВСЕГДА, без dot-режима):
//   1. CLI выполняется НЕ в финальном каталоге, а во временной папке
//      temp_<target>/ в корне проекта (имя генерируется уникальным);
//   2. после успешного завершения всё содержимое (рекурсивно, ВКЛЮЧАЯ
//      скрытые файлы: .gitignore, .env и т.п.) ПРОГРАММНО переносится
//      в target_dir (frontend/, backend/ или корень);
//   3. временная папка удаляется.
// Так CLI никогда не работает «внутри» финального frontend//backend/ —
// «папка внутри папки» невозможна в принципе.
//
// Конфиг (Step::Generate.generator_config):
//   {
//     "command": "npx",
//     "args": ["create-vite@latest", "__TARGET__", "--template", "react-ts"],
//     "name_arg": 1,             // позиция имени проекта в args (обязательно)
//     "target_dir": "frontend",  // куда класть проект ("." = корень проекта)
//     "timeout_secs": 600        // опционально, по умолчанию 600
//   }
// ============================================================================
pub struct ScaffoldGenerator;

#[async_trait]
impl Generator for ScaffoldGenerator {
    fn id(&self) -> &str {
        "scaffold"
    }
    fn name(&self) -> &str {
        "Smart Scaffold"
    }
    fn description(&self) -> &str {
        "Runs a scaffolding CLI into a temp folder, then moves contents into the target dir"
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
            .ok_or_else(|| "ScaffoldGenerator: missing 'command' in config".to_string())?;
        let mut args: Vec<String> = config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let name_arg = config
            .get("name_arg")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "ScaffoldGenerator: missing 'name_arg' in config".to_string())?
            as usize;
        let target_dir = config
            .get("target_dir")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "ScaffoldGenerator: missing 'target_dir' in config".to_string())?;
        let timeout_secs = config
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(600);

        // Валидация: плейсхолдер обязан стоять на месте имени проекта.
        if name_arg >= args.len() {
            return Err(format!(
                "ScaffoldGenerator: 'name_arg' {} out of bounds for {} args",
                name_arg,
                args.len()
            ));
        }
        if args[name_arg] != SCAFFOLD_TARGET {
            return Err(format!(
                "ScaffoldGenerator: args[{}] must be the '{}' placeholder, got '{}'",
                name_arg, SCAFFOLD_TARGET, args[name_arg]
            ));
        }

        let target = if target_dir == "." {
            project_path.to_path_buf()
        } else {
            let target = project_path.join(target_dir);
            std::fs::create_dir_all(&target).map_err(|e| {
                format!(
                    "ScaffoldGenerator: failed to create target dir '{}': {}",
                    target_dir, e
                )
            })?;
            target
        };

        // Temp-to-Target: CLI выполняется во временной папке, содержимое
        // (включая скрытые файлы) переносится в target программно.
        let temp_name = unique_temp_name(project_path, target_dir);
        args[name_arg] = temp_name.clone();
        run_cli(command, &args, project_path, None, timeout_secs).await?;

        let temp_dir = project_path.join(&temp_name);
        if !temp_dir.exists() {
            return Err(format!(
                "ScaffoldGenerator: CLI '{}' did not create the expected folder '{}'",
                command, temp_name
            ));
        }
        merge_dir_contents(&temp_dir, &target).map_err(|e| {
            format!(
                "ScaffoldGenerator: failed to move '{}' into '{}': {}",
                temp_name, target_dir, e
            )
        })?;
        std::fs::remove_dir_all(&temp_dir).map_err(|e| {
            format!(
                "ScaffoldGenerator: failed to remove temp dir '{}': {}",
                temp_name, e
            )
        })?;

        Ok(GenerationReport {
            created_files: Vec::new(),
            modified_files: Vec::new(),
            skipped_files: Vec::new(),
            message: format!("Scaffold '{}' created in '{}'", command, target_dir),
        })
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
            .ok_or_else(|| "TauriConfigGenerator: missing 'before_dev_command' in config".to_string())?;
        let before_build_command = config
            .get("before_build_command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "TauriConfigGenerator: missing 'before_build_command' in config".to_string())?;
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

        let tauri_root = if tauri_dir.is_empty() {
            project_path.to_path_buf()
        } else {
            project_path.join(tauri_dir)
        };
        let config_path = tauri_root.join("src-tauri").join("tauri.conf.json");
        let mut value = read_json(&config_path).map_err(|e| {
            format!("TauriConfigGenerator: {} (tauri init не создал конфиг?)", e)
        })?;

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
        build.insert("devUrl".into(), serde_json::Value::String(dev_url.to_string()));
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
            created_files: Vec::new(),
            modified_files: vec![reported_path],
            skipped_files: Vec::new(),
            message: "Tauri configuration adapted to the frontend/ layout".to_string(),
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
        let dirs: Vec<String> = config
            .get("dirs")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec![".".to_string()]);

        let settings_ours = parse_json(&content::generate_vscode_settings(lang))?;
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
            skipped_files: Vec::new(),
            message: "VS Code settings merged with scaffolding CLI configs".to_string(),
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
    // Не модифицируем аргументы рецепта: отдельные CLI имеют собственный
    // синтаксис и сами явно объявляют `--yes`, если он им нужен.
    let mut cmd = spawn_command(command, args);
    cmd.current_dir(working_dir);
    if let Some(env) = env {
        cmd.envs(env);
    }
    // CI=1 заставляет npx/npm/create-* CLI пропускать интерактивные промпты.
    cmd.env("CI", "1");
    // CI не во всех версиях npm отключает подтверждение установки пакета
    // через npx; переменная npm гарантирует неинтерактивный режим.
    cmd.env("NPM_CONFIG_YES", "true");
    cmd.env("npm_config_yes", "true");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| {
            format!(
                "Command failed (failed to spawn)\ncommand: {}\nworking directory: {}\nerror: {}",
                command_display(command, args),
                working_dir.display(),
                e
            )
        })?;
    let stdout = child.stdout.take().expect("stdout should be piped");
    let stderr = child.stderr.take().expect("stderr should be piped");
    let out_handle = tokio::spawn(async move { tail_lines(stdout).await });
    let err_handle = tokio::spawn(async move { tail_lines(stderr).await });

    let waited = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let out_tail = out_handle.await.unwrap_or_default();
            let err_tail = err_handle.await.unwrap_or_default();
            return Err(format_command_error(
                &format!("process wait failed: {e}"),
                &command_display(command, args),
                working_dir,
                &format_output_tails(&out_tail, &err_tail),
            ));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let out_tail = out_handle.await.unwrap_or_default();
            let err_tail = err_handle.await.unwrap_or_default();
            return Err(format_command_error(
                &format!("timed out after {timeout_secs} seconds"),
                &command_display(command, args),
                working_dir,
                &format_output_tails(&out_tail, &err_tail),
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
            message: format!("Command '{}' completed successfully", command_display(command, args)),
        });
    }

    Err(format_command_error(
        &format!("exited with status {}", exit_status_text(&status)),
        &command_display(command, args),
        working_dir,
        &format_output_tails(&out_tail, &err_tail),
    ))
}

fn command_display(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_output_tails(stdout: &str, stderr: &str) -> String {
    let stdout = if stdout.trim().is_empty() { "<empty>" } else { stdout };
    let stderr = if stderr.trim().is_empty() { "<empty>" } else { stderr };
    format!("stdout tail:\n{stdout}\nstderr tail:\n{stderr}")
}

fn format_command_error(reason: &str, command: &str, working_dir: &Path, detail: &str) -> String {
    format!(
        "Command failed ({reason})\ncommand: {command}\nworking directory: {}\n{detail}",
        working_dir.display()
    )
}

fn exit_status_text(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "terminated by signal".to_string(),
    }
}

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
        format!("Merged inner .vscode into root: {}", merged_files.join(", "))
    };
    Ok(GenerationReport {
        created_files: Vec::new(),
        modified_files: merged_files,
        skipped_files: Vec::new(),
        message,
    })
}

/// Уникальное имя временной папки для temp+move: `temp_<target>` + числовой
/// суффикс, если папка с таким именем уже занята.
fn unique_temp_name(project_path: &Path, target_dir: &str) -> String {
    let hint: String = target_dir
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
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

/// Рекурсивно перенести содержимое src в dst: каталоги сливаются, файлы
/// перезаписываются. Скрытые файлы включены. Исходные файлы после
/// копирования удаляются (перенос, а не копирование); опустевшие каталоги
/// тоже убираются.
fn merge_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(src).map_err(|e| {
        format!("read_dir {}: {}", src.display(), e)
    })? {
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
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_json(&text)
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| format!("serialize JSON: {}", e))?;
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
/// spawn передаёт аргумент curl'у байт-в-байт.
async fn download_starter(url: &str, zip_path: &Path) -> Result<(), String> {
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
    let mut cmd = TokioCommand::new("curl");
    cmd.args(&args);
    cmd.current_dir(zip_path.parent().unwrap_or(Path::new(".")));
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.stdin(std::process::Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Spring Initializr отказал: failed to spawn curl: {}", e))?;
    let stderr = child.stderr.take().expect("stderr should be piped");
    let err_handle = tokio::spawn(async move { tail_lines(stderr).await });
    let stdout = child.stdout.take().expect("stdout should be piped");
    let out_handle = tokio::spawn(async move { tail_lines(stdout).await });

    let waited = tokio::time::timeout(Duration::from_secs(180), child.wait()).await;
    let status = match waited {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(format!("Spring Initializr отказал: failed to run curl: {}", e));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(
                "Spring Initializr отказал: timed out after 180 seconds".to_string(),
            );
        }
    };
    let stderr_tail = err_handle.await.unwrap_or_default();
    let stdout_tail = out_handle.await.unwrap_or_default();

    // HTTP-статус от curl (-w "%{http_code}"): при любом HTTP-ответе
    // (включая 4xx/5xx) процесс завершается успешно, статус — в stdout.
    let http_code: u32 = stdout_tail.trim().parse().unwrap_or(0);

    if !status.success() && http_code == 0 {
        // curl не дождался HTTP-ответа (недоступный хост, обрыв соединения).
        let tail = stderr_tail.trim();
        let reason = if tail.is_empty() {
            format!("HTTP request failed (exit code {})", status.code().unwrap_or(-1))
        } else {
            truncate(tail, 500)
        };
        return Err(format!("Spring Initializr отказал: {}", reason));
    }

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

    Ok(())
}

/// Настоящий ZIP-архив? Проверяем магические байты (PK\x03\x04 — обычный
/// архив, PK\x05\x06 — пустой архив). Тело HTTP-ошибки (JSON/HTML) сюда
/// не подходит — это и есть детектор «ответ был не архивом».
fn is_zip_archive(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && (bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06"))
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

/// Параметр в командную строку cmd.exe: кавычки нужны при пробелах или
/// cmd-метасимволах (включая % — cmd раскрывает %VAR% даже в кавычках,
/// поэтому такие аргументы обязаны быть в кавычках и не содержать валидных
/// %VAR% пар). Внутренние кавычки удваиваются (cmd-эскейп "" внутри строки).
fn win_quote_arg(arg: &str) -> String {
    let needs_quote = arg.is_empty()
        || arg
            .chars()
            // `@` is valid in npm package names (`@nestjs/cli`, `create-vite@latest`).
            // Do not quote it by itself: cmd can pass the quotes through to npm,
            // producing EINVALIDPACKAGENAME for otherwise valid packages.
            .any(|c| c.is_whitespace() || "&()[]{}<>^|%!\"".contains(c));
    if !needs_quote {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('"', "\"\""))
    }
}

/// Командная строка для `cmd /S /C`: каждый токен кавычкуется по
/// необходимости, вся строка оборачивается во ВНЕШНИЕ кавычки — cmd снимает
/// внешнюю пару, внутренние кавычки сохраняются. Без внешней обёртки
/// команда-путь с пробелами ломается: cmd снимает первую кавычку и режет
/// токен по пробелу («C:\Users\John» → «Doe\pip.exe» отдельным аргументом).
pub fn win_command_line(command: &str, args: &[String]) -> String {
    let mut line = win_quote_arg(command);
    for arg in args {
        line.push(' ');
        line.push_str(&win_quote_arg(arg));
    }
    format!("\"{}\"", line)
}

/// Параметр для `sh -c`: одинарные кавычки с экранированием '\'' —
/// защищает пробелы и метасимволы (&, ;, |, $, `, ", ...).
pub fn sh_quote(arg: &str) -> String {
    let needs_quote = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || "&;|<>()$`\\\"'*?[]~#!{}".contains(c));
    if !needs_quote {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
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
                // Не прогоняем обычные CLI через `cmd /C`: вложенные кавычки
                // (особенно у `node -e` и путей Composer) в таком режиме
                // искажаются ещё до запуска процесса. Прямой запуск также
                // корректно обрабатывает stdout/stderr и коды возврата.
                let program = windows_command_program(command);
                let batch = is_windows_batch(&program);
                let mut cmd = if batch {
                    let mut shell = TokioCommand::new("cmd");
                    shell.arg("/D").arg("/S").arg("/C").arg(windows_shell_line(&program, args));
                    shell
                } else {
                    TokioCommand::new(program)
                };
                if !batch {
                    cmd.args(args);
                }
                cmd
            }
        }
        _ => {
            let mut shell_cmd = String::from(command);
            for arg in args {
                shell_cmd.push(' ');
                shell_cmd.push_str(&sh_quote(arg));
            }
            let mut cmd = TokioCommand::new("sh");
            cmd.arg("-c").arg(shell_cmd);
            cmd
        }
    }
}

/// Имя исполняемого файла для прямого запуска на Windows. npm-экосистема
/// устанавливает эти команды как batch-файлы; CreateProcess не умеет
/// запускать `.cmd/.bat` без cmd.exe, поэтому явно добавляем расширение.
/// Пути и уже расширенные имена оставляем без изменений.
pub fn windows_command_program(command: &str) -> String {
    let trimmed = command.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(".cmd")
        || lower.ends_with(".bat")
        || lower.ends_with(".exe")
        || trimmed.contains('\\')
        || trimmed.contains('/')
    {
        return trimmed.to_string();
    }
    match lower.as_str() {
        "npx" | "npm" | "pnpm" | "yarn" | "vite" | "nest" => {
            format!("{}.cmd", trimmed)
        }
        "composer" => "composer.bat".to_string(),
        _ => trimmed.to_string(),
    }
}

pub fn is_windows_batch(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
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
    fn extract_error_text_parses_initializr_json_message() {
        let body = br#"{"timestamp":"2026-08-13T00:00:00.000+00:00","status":400,"error":"Bad Request","message":"Invalid dependency 'web' for type 'maven-project'","path":"/starter.zip"}"#;
        // Из тела достаётся именно JSON-поле message — оно подставляется в
        // «Spring Initializr отказал: <message>».
        let text = extract_error_text(body);
        assert_eq!(
            text,
            "Invalid dependency 'web' for type 'maven-project'",
            "message извлекается без статуса/error: {text}"
        );
    }

    #[test]
    fn extract_error_text_falls_back_to_raw_html() {
        let body = b"<html><body>Spring Initializr is temporarily down</body></html>";
        let text = extract_error_text(body);
        assert!(text.contains("Spring Initializr is temporarily down"), "{text}");
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
    fn win_command_line_quotes_spaces_and_metachars() {
        // Команда-путь с пробелами (venv\Scripts\pip.exe) оборачивается во
        // внешние кавычки: cmd /S /C снимает внешнюю пару, внутренние
        // кавычки сохраняются — токен не режется по пробелу.
        let line = win_command_line(
            "C:\\Users\\John Doe\\app\\venv\\Scripts\\pip.exe",
            &["install".into(), "-r".into(), "C:\\req file.txt".into(), "alembic".into()],
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

    /// Уникальная временная папка для тестов (по имени тега).
    fn temp_test_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("stackpilot_gen_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn scaffold_generator_validates_placeholder_config() {
        let gen = ScaffoldGenerator;
        let ctx = WizardContext::default();
        // нет name_arg — конфиг обязан падать без запуска CLI
        let cfg = serde_json::json!({ "command": "npx", "args": ["create-vite@latest", "app"] });
        let res = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &cfg));
        assert!(res.is_err(), "конфиг без name_arg обязан падать");

        // name_arg вне границ args
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "app"],
            "name_arg": 5,
            "target_dir": ".",
            "dot_capable": true,
        });
        let res = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &cfg));
        assert!(res.is_err(), "name_arg вне границ обязан падать");

        // плейсхолдер не на месте
        let cfg = serde_json::json!({
            "command": "npx",
            "args": ["create-vite@latest", "app"],
            "name_arg": 1,
            "target_dir": ".",
            "dot_capable": true,
        });
        let res = tokio_test_block_on(gen.generate(&ctx, Path::new("."), &cfg));
        assert!(res.is_err(), "отсутствие плейсхолдера обязано падать");
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
        assert_eq!(std::fs::read_to_string(dst.join("keep.txt")).unwrap(), "keep");
        assert!(!src.join("package.json").exists(), "исходный файл удаляется после копирования");
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
        assert_eq!(merged["typescript.tsdk"], "node_modules/typescript/lib", "ключ CLI побеждает");
        assert_eq!(merged["editor.formatOnSave"], true, "наши ключи добавляются");
        assert_eq!(merged["[typescript]"]["editor.formatOnSave"], false, "вложенный ключ CLI побеждает");
        assert_eq!(merged["[typescript]"]["editor.defaultFormatter"], "esbenp.prettier-vscode");
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
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("merge должен пройти");

        // Ключ CLI сохранился, наши ключи добавлены
        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(vscode.join("settings.json")).unwrap()).unwrap();
        assert_eq!(merged["typescript.tsdk"], "node_modules/typescript/lib");
        assert_eq!(merged["editor.formatOnSave"], true);

        // Рекомендации объединяются без дубликатов
        let exts: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(vscode.join("extensions.json")).unwrap()).unwrap();
        let recs = exts["recommendations"].as_array().unwrap();
        assert!(recs.iter().any(|r| r == "dbaeumer.vscode-eslint"), "{recs:?}");
        assert!(recs.iter().any(|r| r == "esbenp.prettier-vscode"), "{recs:?}");

        // В frontend/ (без .vscode от CLI) папка НЕ создаётся
        assert!(!dir.join("frontend").join(".vscode").exists(), "не-корневой .vscode без CLI не создаётся");
        assert!(report.modified_files.iter().any(|f| f == "./.vscode/settings.json"));
        let _ = std::fs::remove_dir_all(&dir);
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

        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(sub_vscode.join("settings.json")).unwrap()).unwrap();
        assert_eq!(merged["typescript.tsdk"], "node_modules/typescript/lib");
        assert_eq!(merged["editor.formatOnSave"], true, "наши настройки добавлены в подкаталог CLI");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_patches_build_and_identifier() {
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config");
        let src_tauri = dir.join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
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
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("патч должен пройти");
        assert_eq!(report.modified_files, vec!["src-tauri/tauri.conf.json".to_string()]);

        let patched: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src_tauri.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(patched["build"]["frontendDist"], "../frontend/dist");
        assert_eq!(patched["build"]["beforeDevCommand"], "npm --prefix frontend run dev");
        assert_eq!(patched["build"]["beforeBuildCommand"], "npm --prefix frontend run build");
        assert_eq!(patched["build"]["devUrl"], "http://localhost:5173");
        assert_eq!(patched["identifier"], "com.myapp");
        assert!(patched["build"].get("distDir").is_none(), "v2-конфиг не получает legacy distDir");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_config_generator_rewrites_legacy_dist_dir() {
        let gen = TauriConfigGenerator;
        let dir = temp_test_dir("tauri_config_v1");
        let src_tauri = dir.join("src-tauri");
        std::fs::create_dir_all(&src_tauri).unwrap();
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
        assert_eq!(patched["build"]["distDir"], "../frontend/dist", "legacy distDir переписывается");
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
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &cfg)).expect("патч должен пройти");
        assert_eq!(report.modified_files, vec!["backend/src-tauri/tauri.conf.json".to_string()]);

        let patched: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(src_tauri.join("tauri.conf.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(patched["build"]["frontendDist"], "../../frontend/dist", "движок задаёт путь из backend/");
        assert_eq!(patched["build"]["beforeDevCommand"], "npm --prefix ../frontend run dev");
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

        assert!(!front_vscode.exists(), "frontend/.vscode удаляется после слияния");
        assert!(!back_vscode.exists(), "backend/.vscode удаляется после слияния");
        assert!(report.modified_files.contains(&"frontend/.vscode/settings.json".to_string()), "{:?}", report.modified_files);
        assert!(report.modified_files.contains(&"frontend/.vscode/extensions.json".to_string()), "{:?}", report.modified_files);
        assert!(report.modified_files.contains(&"backend/.vscode/extensions.json".to_string()), "{:?}", report.modified_files);

        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root_vscode.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["typescript.tsdk"], "node_modules/typescript/lib", "конфиг CLI побеждает");
        assert_eq!(settings["editor.tabSize"], 2, "глубокое слияние: ключ CLI поверх корневого");
        assert!(
            settings.get("editor.formatOnSave").is_none(),
            "наши настройки не добавляются — это не VsCodeMergeGenerator: {settings}"
        );

        let exts: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root_vscode.join("extensions.json")).unwrap(),
        )
        .unwrap();
        let recs = exts["recommendations"].as_array().unwrap();
        assert!(recs.iter().any(|r| r == "dbaeumer.vscode-eslint"), "{recs:?}");
        assert!(recs.iter().any(|r| r == "rust-lang.rust-analyzer"), "{recs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vscode_folders_merge_is_noop_without_inner_folders() {
        let gen = VsCodeFoldersMergeGenerator;
        let dir = temp_test_dir("vscode_folders_empty");
        let ctx = WizardContext::default();
        let report = tokio_test_block_on(gen.generate(&ctx, &dir, &serde_json::json!({})))
            .expect("no-op не должен падать");
        assert!(report.modified_files.is_empty(), "{:?}", report.modified_files);
        assert!(!dir.join(".vscode").exists(), "корневой .vscode без содержимого не создаётся");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
