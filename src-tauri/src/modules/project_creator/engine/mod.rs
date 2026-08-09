pub mod content;
pub mod executor;
// pub mod template;
use std::collections::HashMap;
use std::path::{Path};
use std::sync::OnceLock;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use chrono::Local;

use crate::modules::project_creator::models::*;

/// Полный движок рецептов:
///   1. plan() — составить план выполнения на основе WizardContext
///   2. execute() — выполнить план с отправкой событий
#[async_trait::async_trait]
pub trait RecipeEngine: Send + Sync {
    /// Составить ExecutionPlan (что делаем, в каком порядке)
    fn plan(
        &self,
        context: &WizardContext,
        project_path: &Path,
    ) -> Result<ExecutionPlan, String>;

    /// Предпросмотр — просто пройтись по плану, посчитать что будет выполнено
    fn preview(&self, plan: &ExecutionPlan) -> RecipePreview;

    /// Выполнить план, отправляя события в tx
    async fn execute(
        &self,
        plan: ExecutionPlan,
        tx: mpsc::Sender<ExecutionEvent>,
    ) -> ExecutionResult;
}

/// ============================================================================
/// DefaultRecipeEngine — заглушка. Всю логику будете писать вы по TZ.
/// ============================================================================
pub struct DefaultRecipeEngine {
    // pub template_engine: Arc<template::TemplateEngine>,
    pub executor: Arc<executor::StepExecutor>,
}

impl DefaultRecipeEngine {
    pub fn new() -> Self {
        Self {
            // template_engine: Arc::new(template::TemplateEngine::new()),
            executor: Arc::new(executor::StepExecutor::new()),
        }
    }
}

#[async_trait::async_trait]
impl RecipeEngine for DefaultRecipeEngine {
    fn plan(
        &self,
        context: &WizardContext,
        project_path: &Path,
    ) -> Result<ExecutionPlan, String> {
        let folder_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app");
        let recipe = compose_recipe(context, folder_name)?;
        // context.project_name (если задан) будет использован внутри compose_recipe
        let steps = flatten_and_filter(&recipe, context, project_path);
        Ok(ExecutionPlan {
            recipe,
            context: context.clone(),
            project_path: project_path.to_path_buf(),
            steps,
        })
    }

    fn preview(&self, plan: &ExecutionPlan) -> RecipePreview {
        let mut previews = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            let action = match step {
                Step::Command { command, .. } => format!("$ {}", command),
                Step::WriteFile { path, .. } => format!("write {}", path),
                // Step::RenderTemplate { path, .. } => format!("render {}", path),
                Step::CreateDirectory { path, .. } => format!("mkdir {}", path),
                Step::Generate { generator_id, .. } => format!("generate[{}]", generator_id),
                Step::Parallel { steps, .. } => format!("parallel ({} steps)", steps.len()),
            };
            previews.push(StepPreview {
                id: step.id(),
                label: step.label(),
                description: step.description(),
                action,
                will_execute: true,
                skip_reason: None,
            });
        }
        let total = previews.len();
        RecipePreview {
            recipe_id: plan.recipe.id.clone(),
            recipe_name: plan.recipe.name.clone(),
            step_previews: previews,
            total_steps: total,
            will_execute_count: total,
            will_skip_count: 0,
        }
    }

    async fn execute(
        &self,
        plan: ExecutionPlan,
        tx: mpsc::Sender<ExecutionEvent>,
    ) -> ExecutionResult {
        let start = Instant::now();
        let total = plan.steps.len();
        let mut results = Vec::with_capacity(total);
        let mut aborted = false;

        for (i, step) in plan.steps.iter().enumerate() {
            if aborted {
                results.push(StepResult {
                    step_id: step.id(),
                    label: step.label(),
                    status: StepStatus::Skipped {
                        reason: "Previous step failed, aborting".into(),
                    },
                    duration_ms: 0,
                });
                continue;
            }

            // Emit StepStarted
            let _ = tx.send(ExecutionEvent {
                event_type: ExecutionEventType::StepStarted,
                step_id: step.id(),
                step_index: i,
                total_steps: total,
                step_name: step.label(),
                step_description: step.description(),
                timestamp: chrono_event_time(),
            }).await;

            let step_start = Instant::now();

            let result = match step {
                Step::Command { .. } => {
                    self.executor.run_command(step, &plan, &tx, i).await
                }
                Step::WriteFile { .. } => {
                    self.executor.write_file(step, &plan, &tx, i).await
                }
                // Step::RenderTemplate { .. } => {
                //     self.executor.render_template(step, &plan, &tx, i, &self.template_engine).await
                // }
                Step::CreateDirectory { .. } => {
                    self.executor.create_directory(step, &plan, &tx, i).await
                }
                Step::Generate { .. } => {
                    // TBD: generator-based step
                    StepResult {
                        step_id: step.id(),
                        label: step.label(),
                        status: StepStatus::Skipped { reason: "Generator not yet implemented".into() },
                        duration_ms: 0,
                    }
                }
                Step::Parallel { .. } => {
                    // TBD: parallel execution
                    StepResult {
                        step_id: step.id(),
                        label: step.label(),
                        status: StepStatus::Skipped { reason: "Parallel not yet implemented".into() },
                        duration_ms: 0,
                    }
                }
            };

            let duration = step_start.elapsed().as_millis() as u64;

            let is_failure = matches!(&result.status, StepStatus::Failed { .. });
            let _ = tx.send(ExecutionEvent {
                event_type: ExecutionEventType::StepCompleted {
                    status: result.status.clone(),
                    duration_ms: duration,
                },
                step_id: step.id(),
                step_index: i,
                total_steps: total,
                step_name: step.label(),
                step_description: step.description(),
                timestamp: chrono_event_time(),
            }).await;

            results.push(StepResult {
                duration_ms: duration,
                ..result
            });

            if is_failure {
                let abort = matches!(&step, Step::Command { on_error: ErrorMode::Abort, .. }
                    | Step::WriteFile { on_error: ErrorMode::Abort, .. }
                    // | Step::RenderTemplate { on_error: ErrorMode::Abort, .. }
                    | Step::CreateDirectory { on_error: ErrorMode::Abort, .. }
                    | Step::Generate { on_error: ErrorMode::Abort, .. }
                    | Step::Parallel { on_error: ErrorMode::Abort, .. });
                if abort {
                    aborted = true;
                }
            }
        }

        let total_duration = start.elapsed().as_millis() as u64;
        let failed: Vec<String> = results.iter()
            .filter_map(|r| match &r.status {
                StepStatus::Failed { .. } => Some(r.step_id.clone()),
                _ => None,
            })
            .collect();

        let overall = if failed.is_empty() {
            OverallStatus::Success
        } else if aborted {
            OverallStatus::Aborted {
                last_step: results.last().map(|r| r.step_id.clone()),
                reason: format!("Failed at step: {}", failed.join(", ")),
            }
        } else {
            OverallStatus::PartialFailure { failed_steps: failed }
        };

        let result = ExecutionResult {
            recipe_id: plan.recipe.id.clone(),
            total_duration_ms: total_duration,
            step_results: results,
            overall: overall.clone(),
        };

        let _ = tx.send(ExecutionEvent {
            event_type: ExecutionEventType::AllCompleted { result: result.clone() },
            step_id: String::new(),
            step_index: total,
            total_steps: total,
            step_name: "Complete".into(),
            step_description: String::new(),
            timestamp: chrono_event_time(),
        }).await;

        result
    }
}

// ============================================================================
// Вспомогательные функции — их вы будете наполнять в TZ
// ============================================================================

/// Составить рецепт на основе WizardContext (TZ Task 1)
fn compose_recipe(context: &WizardContext, folder_name: &str) -> Result<Recipe, String> {
    // project_name может отличаться от folder_name (при auto-rename папки)
    let project_name = context.project_name.as_deref().unwrap_or(folder_name);
    let mut steps: Vec<Step> = Vec::new();
    let project_path = context.project_path.as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or(".");

     steps.push(Step::CreateDirectory {
        id: "create_root".into(),
        label: "Create project root".into(),
        description: "Ensuring project directory exists".into(),
        path: ".".into(),
        condition: None,
        on_error: ErrorMode::Abort,
    });

    // При моно-репозитории (backend + frontend) заранее создаём подпапки
    let layout = SegLayout::compute(context);
    if let Some(dir) = &layout.backend {
        steps.push(Step::CreateDirectory {
            id: "create_backend_dir".into(),
            label: format!("Create {}/", dir),
            description: format!("Create {} directory for backend frameworks", dir),
            path: dir.clone(),
            condition: None,
            on_error: ErrorMode::Abort,
        });
    }
    if let Some(dir) = &layout.frontend {
        steps.push(Step::CreateDirectory {
            id: "create_frontend_dir".into(),
            label: format!("Create {}/", dir),
            description: format!("Create {} directory for frontend frameworks", dir),
            path: dir.clone(),
            condition: None,
            on_error: ErrorMode::Abort,
        });
    }

    for lang in &context.languages {
        // Фреймворк сам создаёт каркас для этого языка (aspnetcore вместо
        // dotnet new console, nextjs вместо js-скаффолда) — generic-шаги
        // языка не нужны и конфликтуют с файлами фреймворка.
        if language_scaffold_suppressed(lang, context) {
            continue;
        }
        let mut lang_steps = steps_for_language(lang, project_name, project_path);
        if let Some(dir) = layout.for_language(lang) {
            lang_steps = into_segment(lang_steps, &dir);
        }
        steps.extend(lang_steps);
    }
    for fw in &context.frameworks {
        steps.extend(steps_for_framework(fw, project_path, project_name, context, layout.for_framework(fw).as_deref()));
    }
    steps.extend(steps_for_tools(&context.tools, project_path));
    steps.extend(steps_for_docker(context, project_path, project_name));
    steps.extend(steps_for_git(context, project_path, project_name));
    steps.extend(steps_for_ci(context, project_path, project_name));
    steps.extend(steps_for_readme(context, project_path, project_name));
    steps.extend(steps_for_vscode(context));

    

    Ok(Recipe {
        id: format!("recipe_{}", project_name),
        name: format!("{:?} project", context.project_type),
        description: format!("Full setup for {} project", project_name),
        tags: context.languages.clone(),
        steps,
    })
}

/// Развернуть Parallel, отфильтровать по condition
fn flatten_and_filter(recipe: &Recipe, context: &WizardContext, _project_path: &Path) -> Vec<Step> {
    let mut result = Vec::new();
    for step in &recipe.steps {
        match step {
            Step::Parallel { steps: inner, .. } => {
                // TBD: recursive flatten
                for s in inner {
                    if evaluate_condition(s.condition(), context) {
                        result.push(s.clone());
                    }
                }
            }
            _ => {
                if evaluate_condition(step.condition(), context) {
                    result.push(step.clone());
                }
            }
        }
    }
    result
}

fn evaluate_condition(cond: Option<&StepCondition>, context: &WizardContext) -> bool {
    let Some(cond) = cond else { return true };
    match cond {
        StepCondition::Always => true,
        StepCondition::ContextHas { key, value } => {
            match key.as_str() {
                "language" => context.languages.iter().any(|l| l == value),
                "framework" => context.frameworks.iter().any(|f| f == value),
                "tool" => context.tools.iter().any(|t| t == value),
                _ => false,
            }
        }
        StepCondition::ContextMissing { key } => !evaluate_condition(Some(&StepCondition::ContextHas { key: key.clone(), value: String::new() }), context),
        StepCondition::FileExists { .. } | StepCondition::FileNotExists { .. } => {
            true // TBD: actual file checks
        }
        StepCondition::TechnologyDetected { name } => {
            context.tools.contains(name) || context.frameworks.contains(name)
        }
        StepCondition::TechnologyNotDetected { name } => {
            !context.tools.contains(name) && !context.frameworks.contains(name)
        }
        StepCondition::FeatureEnabled { feature } => {
            match feature.as_str() {
                "docker" => context.docker,
                "testing" => context.testing,
                "git_init" => context.git_init,
                "vscode_config" => context.vscode_config,
                "ci" => context.ci,
                _ => false,
            }
        }
    }
}

fn chrono_event_time() -> String {
    let local_time = Local::now();
    local_time.format("%H::%M:%S").to_string()
}

fn steps_for_language(lang: &str, project_name: &str, project_path: &str) -> Vec<Step> {
    // Вспомогательная функция для команды с рабочей директорией
    let cmd = |id: &str, label: &str, desc: &str, command: &str| -> Step {
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: command.to_string(),
            args: vec![],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(60),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    };

    // Создание директории — частая операция
    let mkdir = |id: &str, path: &str| -> Step {
        Step::CreateDirectory {
            id: id.to_string(),
            label: format!("Create {}", path),
            description: format!("Create {} directory", path),
            path: path.to_string(),
            condition: None,
            on_error: ErrorMode::Abort,
        }
    };

    match lang.to_lowercase().as_str() {
        "rust" => vec![
            cmd("cargo_init", "Init Cargo project", 
                &format!("Initialize new Rust project '{}'", project_name),
                &format!("cargo init --name {}", project_name)),
        ],

        "typescript" | "javascript" => {
            let mut steps = vec![
                mkdir("create_src", "src"),
                // package.json с базовыми полями
                Step::WriteFile {
                    id: "package_json".into(),
                    label: "Create package.json".into(),
                    description: "Initialize package.json with project metadata".into(),
                    path: "package.json".into(),
                    content: format!(
    r#"{{
  "name": "{}",
  "version": "1.0.0",
  "description": "",
  "main": "src/index.{}",
  "scripts": {{
    "start": "node src/index.{}",
    "dev": "node --watch src/index.{}"
  }},
  "keywords": [],
  "author": "",
  "license": "ISC"
}}"#, 
                        project_name, 
                        if lang == "typescript" { "ts" } else { "js" },
                        if lang == "typescript" { "ts" } else { "js" },
                        if lang == "typescript" { "ts" } else { "js" }
                    ),
                    overwrite: false,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ];
            
            // Для TypeScript добавляем tsconfig.json и инициализацию
            if lang == "typescript" {
                steps.push(cmd("tsc_init", "Init TypeScript", 
                    "Generate tsconfig.json",
                    "npx -p typescript tsc --init --target ES2022 --module commonjs --outDir dist --rootDir src"));
                steps.push(Step::WriteFile {
                    id: "ts_src_index".into(),
                    label: "Create src/index.ts".into(),
                    description: "Create entry point for TypeScript".into(),
                    path: "src/index.ts".into(),
                    content: "console.log('Hello from TypeScript!');\n".into(),
                    overwrite: false,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            } else {
                steps.push(Step::WriteFile {
                    id: "js_src_index".into(),
                    label: "Create src/index.js".into(),
                    description: "Create entry point for JavaScript".into(),
                    path: "src/index.js".into(),
                    content: "console.log('Hello from Node.js!');\n".into(),
                    overwrite: false,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }
            
            steps
        },

        "python" => vec![
            mkdir("create_src", "src"),
            Step::WriteFile {
                id: "pyproject_toml".into(),
                label: "Create pyproject.toml".into(),
                description: "Initialize Python project configuration".into(),
                path: "pyproject.toml".into(),
                content: format!("[project]\nname = \"{}\"\nversion = \"0.1.0\"\ndescription = \"\"\nrequires-python = \">=3.10\"\n", project_name),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
            Step::WriteFile {
                id: "requirements_txt".into(),
                label: "Create requirements.txt".into(),
                description: "Initialize requirements file".into(),
                path: "requirements.txt".into(),
                content: String::new(),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "go" => vec![
            cmd("go_mod_init", "Init Go module", 
                &format!("Initialize Go module '{}'", project_name),
                &format!("go mod init {}", project_name)),
            mkdir("create_cmd", "cmd"),
            mkdir("create_internal", "internal"),
            // Создаём main.go
            Step::WriteFile {
                id: "main_go".into(),
                label: "Create main.go".into(),
                description: "Create Go entry point".into(),
                path: "cmd/main.go".into(),
                content: format!("package main\n\nimport \"fmt\"\n\nfunc main() {{\n\tfmt.Println(\"Hello from {}!\")\n}}\n", project_name),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "java" => vec![
            Step::Command {
                id: "maven_init".into(),
                label: "Init Maven project".into(),
                description: "Generate Maven project structure".into(),
                command: "mvn".into(),
                args: vec![
                    "archetype:generate".into(),
                    "-DgroupId=com.example".into(),
                    format!("-DartifactId={}", project_name),
                    "-DarchetypeArtifactId=maven-archetype-quickstart".into(),
                    "-DarchetypeVersion=1.4".into(),
                    "-DinteractiveMode=false".into(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(120),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ],

        "csharp" => vec![
            cmd("dotnet_new", "Init .NET project", 
                "Create new .NET console project",
                &format!("dotnet new console -n {} --force", project_name)),
        ],

        "c" | "cpp" => {
            let ext = if lang == "cpp" { "cpp" } else { "c" };
            vec![
                mkdir("create_src", "src"),
                mkdir("create_include", "include"),
                Step::WriteFile {
                    id: "main_source".into(),
                    label: format!("Create main.{}", ext),
                    description: "Create main source file".into(),
                    path: format!("src/main.{}", ext),
                    content: if lang == "cpp" {
                        format!("#include <iostream>\n\nint main() {{\n    std::cout << \"Hello from {}!\" << std::endl;\n    return 0;\n}}\n", project_name)
                    } else {
                        format!("#include <stdio.h>\n\nint main() {{\n    printf(\"Hello from {}!\\n\");\n    return 0;\n}}\n", project_name)
                    },
                    overwrite: false,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
                // Базовый Makefile
                Step::WriteFile {
                    id: "makefile".into(),
                    label: "Create Makefile".into(),
                    description: "Create basic Makefile for C/C++".into(),
                    path: "Makefile".into(),
                    content: format!(
                        "CC={}\nCFLAGS=-Iinclude -Wall -Wextra\nSRC=src/main.{}\nTARGET={}\n\nall: $(TARGET)\n\n$(TARGET): $(SRC)\n\t$(CC) $(CFLAGS) $(SRC) -o $(TARGET)\n\nclean:\n\trm -f $(TARGET)\n\nrun: all\n\t./$(TARGET)\n",
                        if lang == "cpp" { "g++" } else { "gcc" },
                        ext,
                        project_name
                    ),
                    overwrite: false,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ]
        },

        "zig" => vec![
            cmd("zig_init", "Init Zig project", 
                "Initialize Zig executable project",
                "zig init-exe"),
        ],

        "dart" => {
            // Dart-пакеты не принимают дефис в имени — приводим к подчёркиванию
            let safe_name = project_name.replace('-', "_");
            vec![
                cmd("dart_create", "Create Dart project",
                    &format!("Create new Dart project '{}'", project_name),
                    &format!("dart create {}", safe_name)),
            ]
        }

        "kotlin" => vec![
            // Gradle init — интерактивный, поэтому просто создаём структуру
            mkdir("create_src_main", "src/main/kotlin"),
            Step::WriteFile {
                id: "main_kt".into(),
                label: "Create Main.kt".into(),
                description: "Create Kotlin entry point".into(),
                path: "src/main/kotlin/Main.kt".into(),
                content: format!("fun main() {{\n    println(\"Hello from {}!\")\n}}\n", project_name),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "php" => vec![
            mkdir("create_src", "src"),
            Step::WriteFile {
                id: "composer_json".into(),
                label: "Create composer.json".into(),
                description: "Initialize Composer configuration".into(),
                path: "composer.json".into(),
                content: format!("{{\n  \"name\": \"app/{}\",\n  \"description\": \"\",\n  \"type\": \"project\",\n  \"autoload\": {{\n    \"psr-4\": {{\n      \"App\\\\\": \"src/\"\n    }}\n  }}\n}}\n", project_name),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "swift" => vec![
            cmd("swift_init", "Init Swift package",
                &format!("Initialize Swift package '{}'", project_name),
                &format!("swift package init --name {} --type executable", project_name)),
        ],

        "elixir" => vec![
            cmd("mix_new", "Create Elixir project",
                &format!("Create new Elixir project '{}'", project_name),
                &format!("mix new {}", project_name)),
        ],

        "gleam" => vec![
            cmd("gleam_new", "Create Gleam project",
                &format!("Create new Gleam project '{}'", project_name),
                &format!("gleam new {}", project_name)),
        ],

        _ => vec![
            // Для неизвестных языков создаём базовую структуру
            mkdir("create_src", "src"),
            Step::Command {
                id: "echo_unsupported".into(),
                label: "Unsupported language notice".into(),
                description: format!("Language '{}' has no specific init steps", lang),
                command: "echo".into(),
                args: vec![format!("Project initialized with basic structure for {}", lang)],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(5),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ],
    }
}

/// Вспомогательные функции для определения категорий языков
fn is_frontend_lang(l: &str) -> bool {
    matches!(l, "typescript" | "javascript" | "dart" | "kotlin" | "swift" | "csharp")
}
fn _is_backend_lang(l: &str) -> bool {
    matches!(l, "rust" | "python" | "go" | "java" | "csharp" | "php" | "elixir" | "zig" | "gleam" | "cpp" | "c")
}

// ============================================================================
// Данные о языках/фреймворках — читаются из wizard_tree.json (не код!).
// Раньше списки «фронтенд/бэкенд-фреймворков» дублировались константами
// здесь и полем output_subdir в wizard_tree.json — при правке данных
// движок молча расходился с мастером. Единый источник истины — JSON.
// ============================================================================

fn wizard_tree() -> &'static WizardTreeData {
    static TREE: OnceLock<WizardTreeData> = OnceLock::new();
    TREE.get_or_init(|| {
        let raw = include_str!("../knowledge/wizard_tree.json");
        serde_json::from_str(raw).expect("wizard_tree.json должен быть корректным JSON")
    })
}

fn framework_def(id: &str) -> Option<&'static FrameworkDef> {
    wizard_tree().frameworks.iter().find(|f| f.id == id)
}

fn language_def(id: &str) -> Option<&'static LanguageDef> {
    wizard_tree().languages.iter().find(|l| l.id == id)
}

/// Сторона языка по его category (используется, когда мастер не прислал
/// явные backend_languages/frontend_languages — старые сессии).
/// "both"-языки (csharp, dart, kotlin...) по умолчанию считаются бэкендом.
fn language_side_infer(lang: &str) -> Option<&'static str> {
    match language_def(lang).and_then(|l| l.category.as_deref()) {
        Some("backend") | Some("both") => Some("backend"),
        Some("frontend") | Some("static") => Some("frontend"),
        _ => None,
    }
}

/// Generic-скаффолд языка подавляется, если выбран фреймворк, который сам
/// создаёт каркас проекта для этого языка (aspnetcore вместо `dotnet new
/// console`, nextjs вместо js-скаффолда, spring-boot вместо maven archetype
/// и т.п.). Флаг suppresses_language_scaffold живёт в wizard_tree.json.
fn language_scaffold_suppressed(lang: &str, context: &WizardContext) -> bool {
    context.frameworks.iter().any(|fw| {
        framework_def(fw).is_some_and(|def| {
            def.suppresses_language_scaffold && def.languages.iter().any(|l| l == lang)
        })
    })
}

// ============================================================================
// Сегментация проекта: backend/ + frontend/ (моно-репозиторий)
//
// Сторона определяется ЯЗЫКАМИ, которые пользователь выбрал в мастере:
// backend_languages/frontend_languages — явные назначения (шаги «Backend»
// и «Frontend»). Фреймворк следует за своим языком (requires_language),
// а не за собственным kind: express — серверный фреймворк, но если
// пользователь выбрал его как фреймворк своего «фронтенд»-языка, его файлы
// попадают в frontend/. Это чинит кейс «aspnetcore + express»: раньше оба
// были kind=backend, сегментация не включалась и файлы сталкивались в корне.
//
// Если мастер не прислал явные стороны (старые сессии) — стороны выводятся
// из category языка. Если выбрана только одна сторона — сегментация не
// применяется, всё создаётся в корне проекта, как раньше.
// ============================================================================

struct SegLayout {
    frontend: Option<String>,
    backend: Option<String>,
    /// язык → "backend" | "frontend" (явное назначение мастера или вывод по category)
    lang_side: HashMap<String, &'static str>,
}

impl SegLayout {
    fn compute(context: &WizardContext) -> SegLayout {
        let mut lang_side: HashMap<String, &'static str> = HashMap::new();
        for l in &context.backend_languages {
            lang_side.insert(l.clone(), "backend");
        }
        for l in &context.frontend_languages {
            lang_side.insert(l.clone(), "frontend");
        }
        // Языки без явного назначения — по category (обратная совместимость).
        for l in &context.languages {
            lang_side
                .entry(l.clone())
                .or_insert_with(|| language_side_infer(l).unwrap_or("backend"));
        }
        // Забытые языки (в списке языка нет, а сторона заявлена) — не важны.

        let has_backend = lang_side.values().any(|s| *s == "backend");
        let has_frontend = lang_side.values().any(|s| *s == "frontend");
        if has_backend && has_frontend {
            SegLayout {
                frontend: Some("frontend".into()),
                backend: Some("backend".into()),
                lang_side,
            }
        } else {
            SegLayout {
                frontend: None,
                backend: None,
                lang_side,
            }
        }
    }

    /// Каталог сегмента для языка (None = корень проекта)
    fn for_language(&self, lang: &str) -> Option<String> {
        match self.lang_side.get(lang).copied() {
            Some("backend") => self.backend.clone(),
            Some("frontend") => self.frontend.clone(),
            _ => None,
        }
    }

    /// Каталог сегмента для фреймворка (None = корень проекта).
    /// Приоритет — явная сторона фреймворка (side в wizard_tree.json):
    /// nest (backend) должен попасть в backend/ даже если TypeScript стоит
    /// и на фронтенд-стороне. Для side="either" сторона берётся из языка,
    /// который фреймворк требует (qt + cpp на бэкенде → backend/).
    fn for_framework(&self, id: &str) -> Option<String> {
        if let Some(fw) = framework_def(id) {
            match fw.side.as_str() {
                "backend" => return self.backend.clone(),
                "frontend" => return self.frontend.clone(),
                _ => {}
            }
            if fw.side == "either" {
                for lang in &fw.languages {
                    if let Some(side) = self.lang_side.get(lang).copied() {
                        return match side {
                            "backend" => self.backend.clone(),
                            "frontend" => self.frontend.clone(),
                            _ => None,
                        };
                    }
                }
            }
        }
        None
    }
}

/// Шаги CLI-генераторов, которые сами создают подпапку с именем проекта
/// (create-next-app <name>, flutter create <name> и т.п.).
/// При сегментации в такой команде подменяется имя создаваемой подпапки
/// (индекс в args), а рабочая директория остаётся корнем проекта;
/// остальные шаги (write_file, остальные команды) переносятся в сегмент.
const FOLDER_MAKER_STEPS: &[(&str, usize)] = &[
    ("nextjs_create", 1),   // npx create-next-app@latest <имя>
    ("sveltekit_create", 2),// npx sv create <имя>
    ("nuxt_create", 3),     // npx --yes nuxi@latest init <имя>
    ("solid_init", 1),      // npx create-solid <имя>
    ("flutter_create", 1),  // flutter create <имя>
    ("electron_init", 1),   // npx create-electron-app <имя>
    ("rn_init", 2),         // npx @react-native-community/cli init <имя>
    ("expo_init", 1),       // npx create-expo-app <имя>
    ("plasmo_init", 2),     // npx plasmo init <имя>
    ("laravel_new", 3),     // npx --yes @laravel/installer new <имя>
    ("symfony_new", 1),     // symfony new <имя>
    ("phoenix_new", 1),     // mix phx.new <имя>
    ("vapor_new", 1),       // vapor new <имя>
    ("vite_create", 1),     // npx create-vite@latest <имя>
];

/// Шаги-«хвосты» генераторов подпапок, которые должны выполняться ВНУТРИ
/// созданной подпапки (npm install после create-vite и т.п.). При
/// сегментации их working_dir переносится на имя сегмента, а в корневом
/// режиме остаётся именем созданной папки.
const FOLDER_WORKDIR_STEPS: &[&str] = &[
    "vite_install",
];

fn join_seg(wd: &str, seg: &str) -> String {
    if wd.is_empty() || wd == "." {
        seg.to_string()
    } else {
        format!("{}/{}", wd.trim_end_matches(['/', '\\']), seg)
    }
}

/// Заворачивает шаги фреймворка в каталог сегмента (backend/ или frontend/):
/// пути WriteFile/CreateDirectory и рабочие директории команд получают
/// префикс, а у генераторов подпапок (create-next-app и т.п.) меняется имя
/// архивного каталога.
fn into_segment(steps: Vec<Step>, dir: &str) -> Vec<Step> {
    steps.into_iter().map(|step| {
        let replaces = matches!(&step, Step::Command { id, .. } if
            FOLDER_MAKER_STEPS.iter().any(|(sid, _)| *sid == id.as_str()));
        let name_index = FOLDER_MAKER_STEPS.iter()
            .find(|(sid, _)| matches!(&step, Step::Command { id, .. } if *sid == id.as_str()))
            .map(|(_, idx)| *idx);
        match step {
            Step::Command { id, label, description, command, mut args, working_dir, env, timeout_secs, condition, on_error, interactive } => {
                if replaces {
                    if let Some(idx) = name_index {
                        if let Some(arg) = args.get_mut(idx) {
                            *arg = dir.to_string();
                        }
                    }
                    // генератор сам создаст подпапку — рабочая директория остаётся корневой
                    Step::Command { id, label, description, command, args, working_dir, env, timeout_secs, condition, on_error, interactive }
                } else if FOLDER_WORKDIR_STEPS.contains(&id.as_str()) {
                    // «хвост» генератора подпапки (npm install после create-vite):
                    // при сегментации созданная папка уже переименована в сегмент —
                    // рабочая директория становится самим сегментом
                    Step::Command { id, label, description, command, args, working_dir: Some(dir.to_string()), env, timeout_secs, condition, on_error, interactive }
                } else {
                    Step::Command {
                        id, label, description, command, args,
                        working_dir: working_dir.map(|wd| join_seg(&wd, dir)),
                        env, timeout_secs, condition, on_error, interactive,
                    }
                }
            }
            Step::WriteFile { id, label, description, path, content, overwrite, condition, on_error } => {
                Step::WriteFile {
                    id, label, description,
                    path: format!("{}/{}", dir, path),
                    content, overwrite, condition, on_error,
                }
            }
            Step::CreateDirectory { id, label, description, path, condition, on_error } => {
                Step::CreateDirectory {
                    id, label, description,
                    path: format!("{}/{}", dir, path),
                    condition, on_error,
                }
            }
            other => other,
        }
    }).collect()
}

fn steps_for_framework(fw: &str, project_path: &str, project_name: &str, context: &WizardContext, seg: Option<&str>) -> Vec<Step> {
    let mut steps = steps_for_framework_impl(fw, project_path, project_name, context);

    // Inplace-фреймворки (scaffold не задан: express, fastapi, gin, clap...)
    // дописывают файлы в каркас, созданный language-скаффолдом. Их файлы —
    // это «настоящий» контент приложения, а скаффолд языка — заглушка:
    // express обязан перезаписать package.json/src/index.js, иначе шаг
    // молча скипается (executor не пишет поверх при overwrite=false).
    // Раньше это обещание было описано в коммите, но не реализовано —
    // express-шаги в связке js+express просто пропадали.
    if let Some(def) = framework_def(fw) {
        if def.scaffold.is_none() {
            for step in &mut steps {
                if let Step::WriteFile { overwrite, .. } = step {
                    *overwrite = true;
                }
            }
        }
    }

    match seg {
        Some(dir) => into_segment(steps, dir),
        None => steps,
    }
}

/// Проверка целостности генерации: не пишут ли два разных фреймворка один
/// и тот же файл (WriteFile). Пути считаются ТОЧНО как в compose_recipe —
/// с учётом сегментации mono-репозитория (SegLayout), иначе легальные
/// связки (tauri→backend/, react→frontend/) дали бы ложные срабатывания.
///
/// Даже при overwrite=false второй пишущий молча скипнется (executor не
/// пишет поверх), и проект останется без своего файла — поэтому это
/// проблема стека, а не «шум». Легальные связки (gin+cobra, axum+clap,
/// zap+zig-cli, fastapi+aiogram) маршрутизируются в разные пути движком,
/// любые новые комбинации отсекаются до генерации.
pub fn duplicate_framework_write_paths(context: &WizardContext) -> Vec<String> {
    let project_name = context
        .project_name
        .clone()
        .unwrap_or_else(|| "app".to_string());
    let layout = SegLayout::compute(context);
    let mut by_path: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for fw in &context.frameworks {
        let seg = layout.for_framework(fw);
        let steps = steps_for_framework(fw, ".", &project_name, context, seg.as_deref());
        for step in steps {
            if let Step::WriteFile { path, .. } = step {
                by_path.entry(path).or_default().push(fw.clone());
            }
        }
    }
    let mut issues: Vec<String> = by_path
        .into_iter()
        .filter(|(_, fws)| {
            fws.len() > 1 && {
                let mut unique = fws.clone();
                unique.sort();
                unique.dedup();
                unique.len() > 1
            }
        })
        .map(|(path, fws)| {
            format!(
                "Фреймворки «{}» создают один и тот же файл «{}» — такая связка сломает сгенерированный проект.",
                fws.join("» и «"),
                path
            )
        })
        .collect();
    issues.sort();
    issues
}

fn steps_for_framework_impl(fw: &str, project_path: &str, project_name: &str, context: &WizardContext) -> Vec<Step> {
    // Определяем язык фронтенда (для Tauri, Expo и др.)
    let frontend_lang = context.languages.iter().find(|l| is_frontend_lang(l));
    let has_typescript = context.languages.iter().any(|l| l == "typescript");
    let has_javascript = context.languages.iter().any(|l| l == "javascript");

    let cmd = |id: &str, label: &str, desc: &str, command: &str, args: Vec<&str>| -> Step {
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: command.to_string(),
            args: args.into_iter().map(String::from).collect(),
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(300),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    };

    // cmd_i — то же самое, но с interactive-записями
    let cmd_i = |id: &str, label: &str, desc: &str, command: &str, args: Vec<&str>, interactive: Vec<InteractiveEntry>| -> Step {
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: command.to_string(),
            args: args.into_iter().map(String::from).collect(),
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(300),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive,
        }
    };

    let write_file = |id: &str, label: &str, path: &str, content: &str| -> Step {
        Step::WriteFile {
            id: id.to_string(),
            label: label.to_string(),
            description: format!("Create {}", path),
            path: path.to_string(),
            content: content.to_string(),
            overwrite: false,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    match fw.to_lowercase().as_str() {
        // ==================== Rust ====================
        "axum" => vec![
            write_file("axum_main", "Create Axum entry point", "src/main.rs",
                &format!(r#"use axum::{{routing::get, Router}};

#[tokio::main]
async fn main() {{
    let app = Router::new().route("/", get(|| async {{ "Hello from {}!" }}));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}}
"#, project_name)),
            // Добавление зависимостей в Cargo.toml — это отдельная тема
            // Пока просто команда, которую executor должен уметь парсить/выполнять
            cmd("add_axum_deps", "Add Axum dependencies", "Add axum + tokio to Cargo.toml",
                "cargo", vec!["add", "axum", "tokio", "--features", "tokio/full"]),
        ],

        "tauri" => {
            // Определяем язык фронтенда для Tauri
            let frontend_choice = if frontend_lang.is_some() {
                "TypeScript / JavaScript"
            } else if has_typescript {
                "TypeScript / JavaScript"
            } else {
                "Rust"
            };
            vec![
                cmd_i("tauri_init", "Init Tauri", "Initialize Tauri in current project",
                    "cargo", vec!["tauri", "init", "--app-name", project_name, "--window-title", project_name, "--dev-url", "http://localhost:1420", "--before-dev-command", "", "--before-build-command", ""],
                    vec![
                        InteractiveEntry {
                            trigger: "Choose which language to use for your frontend".into(),
                            response_type: ResponseType::Text(frontend_choice.to_string()),
                        },
                        InteractiveEntry {
                            trigger: "Choose your package manager".into(),
                            response_type: ResponseType::Text("npm".to_string()),
                        },
                        InteractiveEntry {
                            trigger: "Choose your UI template".into(),
                            response_type: if context.frameworks.iter().any(|f| f == "react" || f == "nextjs") {
                                ResponseType::Text("React".to_string())
                            } else if context.frameworks.iter().any(|f| f == "vue" || f == "nuxt") {
                                ResponseType::Text("Vue".to_string())
                            } else if context.frameworks.iter().any(|f| f == "svelte" || f == "sveltekit") {
                                ResponseType::Text("Svelte".to_string())
                            } else {
                                ResponseType::Text("Vanilla".to_string())
                            },
                        },
                        InteractiveEntry {
                            trigger: "Would you like to install WiX Toolset v3?".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                    ]),
            ]
        },

        "clap" => {
            // Легальная связка axum + clap: веб-сервер владеет src/main.rs,
            // CLI становится отдельным бинарником Cargo (src/bin/cli.rs).
            // Поодиночке clap занимает src/main.rs.
            let cli_path = if context.frameworks.iter().any(|f| f == "axum") {
                "src/bin/cli.rs"
            } else {
                "src/main.rs"
            };
            let cli_id = if cli_path == "src/main.rs" { "clap_main" } else { "clap_cli" };
            let cli_label = if cli_path == "src/main.rs" { "Create CLI entry point" } else { "Create CLI binary (src/bin/cli.rs)" };
            vec![
                cmd("add_clap_deps", "Add Clap dependency", "Add clap with derive feature",
                    "cargo", vec!["add", "clap", "--features", "derive"]),
                write_file(cli_id, cli_label, cli_path,
                    &format!(r#"use clap::Parser;

#[derive(Parser)]
#[command(name = "{}", version = "0.1.0", about = "A CLI tool")]
struct Cli {{
    /// Optional name to greet
    name: Option<String>,
}}

fn main() {{
    let cli = Cli::parse();
    println!("Hello, {{}}!", cli.name.as_deref().unwrap_or("world"));
}}
"#, project_name)),
            ]
        },

        // ==================== Python ====================
        "fastapi" => vec![
            write_file("fastapi_main", "Create FastAPI entry point", "src/main.py",
                &format!(r#"from fastapi import FastAPI

app = FastAPI(title="{}", version="0.1.0")

@app.get("/")
async def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    import uvicorn
    uvicorn.run("main:app", host="0.0.0.0", port=3000, reload=True)
"#, project_name, project_name)),
            // Зависимости добавим потом через pip или pyproject
            Step::WriteFile {
                id: "fastapi_requirements".into(),
                label: "Add FastAPI dependencies".into(),
                description: "Write requirements.txt with FastAPI + uvicorn".into(),
                path: "requirements.txt".into(),
                content: "fastapi[standard]\nuvicorn\n".into(),
                overwrite: false,
                condition: None,
                on_error: ErrorMode::Skip,
            },
        ],

        "django" => {
            // django-admin startproject требует валидный Python-идентификатор:
            // «my-project» (дефис) не подходит — заменяем на подчёркивание.
            let safe_name = project_name.replace('-', "_");
            vec![
                cmd("django_start", "Start Django project", "Create Django project structure",
                    "django-admin", vec!["startproject", &safe_name, "."]),
            ]
        }

        "flask" => vec![
            write_file("flask_app", "Create Flask app", "src/app.py",
                &format!(r#"from flask import Flask

app = Flask(__name__)

@app.route("/")
def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=3000, debug=True)
"#, project_name)),
            write_file("flask_requirements", "Flask dependencies", "requirements.txt",
                "flask\n"),
        ],

        "aiogram" => {
            // Если рядом FastAPI — aiogram уже добавлен в requirements.txt
            // fastapi-секцией, дублировать файл нельзя (последний пишущий
            // затрёт зависимости первого).
            let with_fastapi = context.frameworks.iter().any(|f| f == "fastapi");
            let mut steps = vec![
                write_file("aiogram_bot", "Create Telegram bot", "src/bot.py",
                    &format!(r#"import asyncio
from aiogram import Bot, Dispatcher, types
from aiogram.filters import Command

bot = Bot(token="YOUR_BOT_TOKEN")
dp = Dispatcher()

@dp.message(Command("start"))
async def cmd_start(message: types.Message):
    await message.answer("Hello from {}!")

async def main():
    await dp.start_polling(bot)

if __name__ == "__main__":
    asyncio.run(main())
"#, project_name)),
            ];
            if !with_fastapi {
                steps.push(write_file("aiogram_requirements", "Aiogram dependencies", "requirements.txt",
                    "aiogram\n"));
            }
            steps
        },

        // ==================== Vite: React / Vue / Svelte ====================
        // create-vite с --template работает без интерактива; npm install
        // выполняется внутри созданной подпапки (рабочая директория —
        // <project_path>/<project_name>, при сегментации переносится движком)
        "react" | "vue" | "svelte" => {
            let template = match (fw.to_lowercase().as_str(), has_typescript) {
                ("react", true) => "react-ts",
                ("react", false) => "react",
                ("vue", true) => "vue-ts",
                ("vue", false) => "vue",
                ("svelte", true) => "svelte-ts",
                _ => "svelte",
            };
            vec![
                cmd("vite_create", &format!("Create {fw} app"), "Scaffold Vite project",
                    "npx", vec!["create-vite@latest", project_name, "--template", template]),
                Step::Command {
                    id: "vite_install".into(),
                    label: "Install npm dependencies".into(),
                    description: "npm install inside the created app".into(),
                    command: "npm".into(),
                    args: vec!["install".into()],
                    working_dir: Some(format!("{}/{}", project_path, project_name)),
                    env: None,
                    timeout_secs: Some(600),
                    condition: None,
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                },
            ]
        }

        // ==================== JavaScript / TypeScript ====================
        "nextjs" => vec![
            cmd_i("nextjs_create", "Create Next.js app", "Scaffold Next.js project",
                "npx", vec!["create-next-app@latest", project_name, "--typescript", "--tailwind", "--eslint", "--app", "--no-src-dir", "--import-alias", "@/*", "--use-npm"],
                vec![
                    InteractiveEntry {
                        trigger: "Would you like to use TypeScript?".into(),
                        response_type: if has_typescript { ResponseType::Confirm(true) } else { ResponseType::Confirm(false) },
                    },
                    InteractiveEntry {
                        trigger: "Would you like to use ESLint?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to use Tailwind CSS?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                    InteractiveEntry {
                        trigger: "Would you like your code inside a `src/` directory?".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to use App Router?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to customize the import alias".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                ]),
        ],

        "sveltekit" => {
            // SvelteKit CLI: выбор шаблона, TypeScript, доп. инструменты
            let ts_choice = if has_typescript {
                "Yes, using TypeScript syntax"
            } else {
                "No"
            };
            vec![
                cmd_i("sveltekit_create", "Create SvelteKit app", "Scaffold SvelteKit project",
                    "npx", vec!["sv", "create", project_name],
                    vec![
                        InteractiveEntry {
                            trigger: "Which Svelte app template?".into(),
                            response_type: ResponseType::Select(1), // "Skeleton project"
                        },
                        InteractiveEntry {
                            trigger: "Add type checking with TypeScript?".into(),
                            response_type: ResponseType::Text(ts_choice.into()),
                        },
                        InteractiveEntry {
                            trigger: "Select additional options".into(),
                            response_type: ResponseType::Keys("\n".to_string()), // Enter (ничего не выбираем)
                        },
                    ]),
            ]
        },

        "nuxt" => vec![
            cmd_i("nuxt_create", "Create Nuxt app", "Scaffold Nuxt project",
                "npx", vec!["--yes", "nuxi@latest", "init", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Which package manager would you like to use?".into(),
                        response_type: ResponseType::Text("npm".to_string()),
                    },
                    InteractiveEntry {
                        trigger: "Initialize a new git repository?".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                    InteractiveEntry {
                        trigger: "Directory not empty".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                ]),
        ],

        "express" => vec![
            write_file("express_index", "Create Express entry", "src/index.js",
                &format!(r#"const express = require('express');
const app = express();
const PORT = process.env.PORT || 3000;

app.get('/', (req, res) => {{
    res.json({{ message: 'Hello from {}!' }});
}});

app.listen(PORT, () => {{
    console.log(`Server running on http://localhost:${{PORT}}`);
}});
"#, project_name)),
            write_file("express_package", "Express dependencies", "package.json",
                &format!(r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/index.js",
  "scripts": {{
    "start": "node src/index.js",
    "dev": "node --watch src/index.js"
  }},
  "dependencies": {{
    "express": "^4.18.2"
  }}
}}
"#, project_name)),
        ],

        "electron" => vec![
            cmd_i("electron_init", "Init Electron", "Create Electron app with electron-forge",
                "npx", vec!["create-electron-app", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Choose a template:".into(),
                        response_type: if has_typescript { ResponseType::Text("TypeScript".to_string()) } else { ResponseType::Text("Vite".to_string()) },
                    },
                    InteractiveEntry {
                        trigger: "Initialize a git repository?".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                ]),
        ],

        "telegraf" => vec![
            write_file("telegraf_bot", "Create Telegram bot", "src/bot.js",
                &format!(r#"const {{ Telegraf }} = require('telegraf');
const bot = new Telegraf('YOUR_BOT_TOKEN');

bot.start((ctx) => ctx.reply('Hello from {}!'));

bot.launch();
process.once('SIGINT', () => bot.stop('SIGINT'));
process.once('SIGTERM', () => bot.stop('SIGTERM'));
"#, project_name)),
            write_file("telegraf_package", "Telegraf package.json", "package.json",
                &format!(r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/bot.js",
  "dependencies": {{
    "telegraf": "^4.16.3"
  }}
}}
"#, project_name)),
        ],

        "react-native" => {
            let rn_name = project_name.replace('-', "_");
            vec![
                cmd_i("rn_init", "Init React Native", "Create React Native project",
                    "npx", vec!["@react-native-community/cli", "init", &rn_name],
                    vec![
                        InteractiveEntry {
                            trigger: "Do you want to install CocoaPods dependencies?".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                        InteractiveEntry {
                            trigger: "Downloading and installing the modern architecture dependencies. Proceed?".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                    ]),
            ]
        },

        "expo" => {
            let expo_template = if has_typescript {
                "Blank (TypeScript)"
            } else {
                "Blank"
            };
            vec![
                cmd_i("expo_init", "Init Expo", "Create Expo project",
                    "npx", vec!["create-expo-app", project_name],
                    vec![
                        InteractiveEntry {
                            trigger: "What is your app named?".into(),
                            response_type: ResponseType::Text(project_name.into()),
                        },
                        InteractiveEntry {
                            trigger: "Choose a template:".into(),
                            response_type: ResponseType::Text(expo_template.into()),
                        },
                        InteractiveEntry {
                            trigger: "Download and install CocoaPods dependencies?".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                        InteractiveEntry {
                            trigger: "Do you want to log in".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                        InteractiveEntry {
                            trigger: "Install the iOS and Android".into(),
                            response_type: ResponseType::Confirm(false),
                        },
                    ]),
            ]
        },

        "plasmo" => vec![
            cmd_i("plasmo_init", "Init Plasmo", "Create browser extension with Plasmo",
                "npx", vec!["plasmo", "init", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Project name".into(),
                        response_type: ResponseType::Text(project_name.into()),
                    },
                    InteractiveEntry {
                        trigger: "Select your primary framework/compiler".into(),
                        response_type: if has_typescript || has_javascript {
                            ResponseType::Text("React (Next-like)".to_string())
                        } else {
                            ResponseType::Text("Vanilla".to_string())
                        },
                    },
                ]),
        ],

        "nest" => vec![
            cmd_i("nest_new", "Create NestJS project", "Scaffold NestJS application",
                "npx", vec!["@nestjs/cli", "new", ".", "--package-manager", "npm"],
                vec![
                    InteractiveEntry {
                        trigger: "Which package manager would you love to use".into(),
                        response_type: ResponseType::Text("npm".to_string()),
                    },
                ]),
        ],

        "fastify" => vec![
            write_file("fastify_index", "Create Fastify entry", "src/index.js",
                &format!(r#"const fastify = require('fastify')({{ logger: true }});

fastify.get('/', async () => {{
    return {{ message: 'Hello from {}!' }};
}});

const start = async () => {{
    try {{
        await fastify.listen({{ port: 3000 }});
        console.log('Server running on http://localhost:3000');
    }} catch (err) {{
        fastify.log.error(err);
        process.exit(1);
    }}
}};
start();
"#, project_name)),
            write_file("fastify_package", "Fastify package.json", "package.json",
                &format!(r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/index.js",
  "scripts": {{
    "start": "node src/index.js",
    "dev": "node --watch src/index.js"
  }},
  "dependencies": {{
    "fastify": "^4.28.0"
  }}
}}
"#, project_name)),
        ],

        "solidjs" => vec![
            cmd_i("solid_init", "Create SolidStart app", "Scaffold SolidStart project",
                "npx", vec!["create-solid", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Is this a server-side rendered app".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                    InteractiveEntry {
                        trigger: "Use TypeScript?".into(),
                        response_type: if has_typescript { ResponseType::Confirm(true) } else { ResponseType::Confirm(false) },
                    },
                ]),
        ],

        // ==================== Go ====================
        "gin" => vec![
            write_file("gin_main", "Create Gin entry", "cmd/main.go",
                &format!(r#"package main

import (
    "net/http"
    "github.com/gin-gonic/gin"
)

func main() {{
    r := gin.Default()
    r.GET("/", func(c *gin.Context) {{
        c.JSON(http.StatusOK, gin.H{{"message": "Hello from {}!"}})
    }})
    r.Run(":3000")
}}
"#, project_name)),
            cmd("get_gin", "Install Gin", "Add Gin dependency",
                "go", vec!["get", "github.com/gin-gonic/gin"]),
        ],

        "cobra" => {
            // Легальная связка gin + cobra: веб-сервер владеет cmd/main.go,
            // CLI получает собственный пакет cmd/cli/main.go. Поодиночке
            // cobra занимает cmd/main.go.
            let (cli_path, cli_id, cli_label) = if context.frameworks.iter().any(|f| f == "gin") {
                ("cmd/cli/main.go", "cobra_cli", "Create CLI entry (cmd/cli/main.go)")
            } else {
                ("cmd/main.go", "cobra_main", "Create CLI entry")
            };
            vec![
                cmd("cobra_init", "Init Cobra CLI", "Initialize Cobra CLI project",
                    "go", vec!["get", "github.com/spf13/cobra/cobra"]),
                write_file(cli_id, cli_label, cli_path,
                    &format!(r#"package main

import (
    "fmt"
    "github.com/spf13/cobra"
)

func main() {{
    var rootCmd = &cobra.Command{{
        Use:   "{}",
        Short: "A CLI tool built with Cobra",
        Run: func(cmd *cobra.Command, args []string) {{
            fmt.Println("Hello from {}!")
        }},
    }}
    rootCmd.Execute()
}}
"#, project_name, project_name)),
            ]
        },

        // ==================== Java ====================
        "spring-boot" => {
            // Spring Boot создаётся через Spring Initializr: скачиваем
            // starter.zip и распаковываем. Зависимости собираем из
            // выбранных БД/инструментов (web всегда).
            let mut deps: Vec<&str> = vec!["web"];
            for tool in &context.tools {
                match tool.as_str() {
                    "mongodb" => deps.push("data-mongodb"),
                    "postgresql" => {
                        deps.push("data-jpa");
                        deps.push("postgresql");
                    }
                    "mysql" => {
                        deps.push("data-jpa");
                        deps.push("mysql");
                    }
                    "redis" => deps.push("data-redis"),
                    _ => {}
                }
            }
            let deps_str = deps.join(",");
            vec![
                cmd("spring_init", "Generate Spring Boot project",
                    "Download Spring Boot starter from Initializr",
                    "curl", vec![
                        "-fsL", &format!("https://start.spring.io/starter.zip?name={}&groupId=com.example&artifactId={}&dependencies={}", project_name, project_name, deps_str),
                        "-o", "project.zip",
                    ]),
                cmd("unzip_spring", "Extract Spring Boot", "Unzip the generated project",
                    if cfg!(target_os = "windows") { "tar" } else { "unzip" },
                    if cfg!(target_os = "windows") { vec!["-xf", "project.zip"] } else { vec!["-o", "project.zip"] }),
                cmd("cleanup_zip", "Clean up zip", "Remove project.zip",
                    if cfg!(target_os = "windows") { "del" } else { "rm" },
                    vec!["project.zip"]),
            ]
        }

        "android" => vec![
            // Android Studio — это GUI, CLI создать сложно
            // Оставляем как команду-заглушку
            cmd("android_studio_hint", "Open in Android Studio",
                "Hint: open this project in Android Studio",
                "echo", vec!["Open this project in Android Studio to complete setup"]),
        ],

        // ==================== C# ====================
        "aspnetcore" => vec![
            cmd("aspnet_new", "Create ASP.NET Core Web API", "Scaffold Web API project",
                "dotnet", vec!["new", "webapi", "-n", project_name, "--force"]),
        ],

        "maui" => vec![
            cmd("maui_new", "Create MAUI app", "Scaffold .NET MAUI project",
                "dotnet", vec!["new", "maui", "-n", project_name, "--force"]),
        ],

        "qt" => vec![
            write_file("qt_main", "Create Qt main", "src/main.cpp",
                &format!(r#"#include <QApplication>
#include <QWidget>
#include <QPushButton>

int main(int argc, char *argv[]) {{
    QApplication app(argc, argv);
    QWidget window;
    window.setWindowTitle("{}");
    window.resize(400, 300);
    QPushButton btn("Hello from {}!", &window);
    btn.setGeometry(50, 50, 300, 200);
    window.show();
    return app.exec();
}}
"#, project_name, project_name)),
            write_file("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt",
                &format!(r#"cmake_minimum_required(VERSION 3.16)
project({})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

add_executable({} src/main.cpp)
target_link_libraries({} Qt6::Widgets)
"#, project_name, project_name, project_name)),
        ],

        // ==================== Dart ====================
        "flutter" => {
            // flutter create требует имя без дефиса (валидный Dart-пакет)
            let safe_name = project_name.replace('-', "_");
            vec![
                cmd("flutter_create", "Create Flutter project", "Scaffold Flutter app",
                    "flutter", vec!["create", &safe_name]),
            ]
        }

        // ==================== Kotlin ====================
        "jetpack-compose" => vec![
            cmd("compose_hint", "Jetpack Compose hint",
                "Jetpack Compose projects are created through Android Studio",
                "echo", vec!["Create this project through Android Studio with Jetpack Compose template"]),
        ],

        "ktor" => vec![
            // Пишем В Application.kt, а в Main.kt: kotlin language-скаффолд
            // создаёт src/main/kotlin/Main.kt с main() — второй main()
            // в Application.kt не дал бы проекту собраться. Inplace-фреймворк
            // перезаписывает заглушку (overwrite выставляется в steps_for_framework).
            write_file("ktor_main", "Create Ktor entry", "src/main/kotlin/Main.kt",
                &format!(r#"import io.ktor.application.*
import io.ktor.response.*
import io.ktor.routing.*
import io.ktor.server.engine.*
import io.ktor.server.netty.*

fun main() {{
    embeddedServer(Netty, port = 3000) {{
        routing {{
            get("/") {{
                call.respond(mapOf("message" to "Hello from {}!"))
            }}
        }}
    }}.start(wait = true)
}}
"#, project_name)),
        ],

        // ==================== PHP ====================
        "laravel" => vec![
            // Пакет «laravel» на npm не имеет bin («could not determine
            // executable to run») — официальный путь через @laravel/installer.
            cmd_i("laravel_new", "Create Laravel project",
                "Scaffold Laravel application",
                "npx", vec!["--yes", "@laravel/installer", "new", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Would you like to install a starter kit?".into(),
                        response_type: ResponseType::Select(1), // "Breeze" (index 1)
                    },
                    InteractiveEntry {
                        trigger: "Which frontend stack would you like to use?".into(),
                        response_type: if has_typescript || has_javascript {
                            ResponseType::Text("React with Inertia".to_string())
                        } else {
                            ResponseType::Text("Blade with Alpine".to_string())
                        },
                    },
                    InteractiveEntry {
                        trigger: "Which testing framework do you prefer?".into(),
                        response_type: ResponseType::Text("Pest".to_string()),
                    },
                    InteractiveEntry {
                        trigger: "Which database will your application use?".into(),
                        response_type: ResponseType::Text("SQLite".to_string()),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to run the default database migrations?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                ]),
        ],

        "symfony" => vec![
            cmd_i("symfony_new", "Create Symfony project",
                "Scaffold Symfony application",
                "symfony", vec!["new", project_name, "--dir", project_path],
                vec![
                    InteractiveEntry {
                        trigger: "Do you want to include support for Docker?".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                    InteractiveEntry {
                        trigger: "wants to execute a script. Do you confirm?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                ]),
        ],

        // ==================== Swift ====================
        "swiftui" => vec![
            cmd("swiftui_hint", "SwiftUI project hint",
                "SwiftUI projects are created through Xcode",
                "echo", vec!["Create this project through Xcode with SwiftUI template"]),
        ],

        "vapor" => vec![
            cmd_i("vapor_new", "Create Vapor project", "Scaffold Vapor application",
                "vapor", vec!["new", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Would you like to use Fluent?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                    InteractiveEntry {
                        trigger: "Choose a database engine:".into(),
                        response_type: ResponseType::Text("SQLite".to_string()),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to use Leaf?".into(),
                        response_type: ResponseType::Confirm(false),
                    },
                ]),
        ],

        // ==================== Zig ====================
        "zig-cli" => {
            // Легальная связка zap + zig-cli: сервер (zap) занимает
            // src/main.zig, CLI становится модулем src/cli.zig — один
            // бинарник, запуск CLI через «<app> cli».
            if context.frameworks.iter().any(|f| f == "zap") {
                vec![write_file(
                    "zig_cli_module",
                    "Create Zig CLI module",
                    "src/cli.zig",
                    &format!(r#"const std = @import("std");

pub fn run() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}} CLI!\n", .{{"{}"}});
}}
"#, project_name)),
                ]
            } else {
                vec![write_file("zig_main", "Create Zig CLI entry", "src/main.zig",
                    &format!(r#"const std = @import("std");

pub fn main() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}}!\n", .{{"{}"}});
}}
"#, project_name)),
                ]
            }
        },

        "zap" => {
            // Zig-веб: zap через zig fetch (зависимость в build.zig.zon).
            // В связке с zig-cli main.zig получает диспетчер «<app> cli».
            let has_cli = context.frameworks.iter().any(|f| f == "zig-cli");
            let main_zig = if has_cli {
                format!(r#"const std = @import("std");
const zap = @import("zap");
const cli = @import("cli.zig");

fn on_request(r: zap.Request) void {{
    r.sendBody("Hello from {{s}}!", .{{"{}"}}) catch {{}};
}}

pub fn main() !void {{
    var args = std.process.args();
    _ = args.next();
    if (args.next()) |arg| {{
        if (std.mem.eq(u8, arg, "cli")) return cli.run();
    }}
    var listener = zap.HttpListener.init(.{{
        .on_request = on_request,
        .port = 3000,
    }});
    try listener.listen();
    std.debug.print("Listening on http://localhost:3000\n", .{{}});
    zap.start(.{{ .threads = 1, .workers = 1 }});
}}
"#, project_name)
            } else {
                format!(r#"const std = @import("std");
const zap = @import("zap");

fn on_request(r: zap.Request) void {{
    r.sendBody("Hello from {{s}}!", .{{"{}"}}) catch {{}};
}}

pub fn main() !void {{
    var listener = zap.HttpListener.init(.{{
        .on_request = on_request,
        .port = 3000,
    }});
    try listener.listen();
    std.debug.print("Listening on http://localhost:3000\n", .{{}});
    zap.start(.{{ .threads = 1, .workers = 1 }});
}}
"#, project_name)
            };
            vec![
                write_file("zap_zon", "Create build.zig.zon", "build.zig.zon",
                    r#".{
    .name = .my_app,
    .version = "0.1.0",
    .minimum_zig_version = "0.14.0",
    .dependencies = .{},
}
"#),
                cmd("zap_fetch", "Add Zap dependency", "Fetch zap and save to build.zig.zon",
                    "zig", vec!["fetch", "--save", "https://github.com/zigzap/zap/archive/refs/tags/v0.4.0.tar.gz"]),
                write_file("zap_main", "Create Zap server entry", "src/main.zig", &main_zig),
            ]
        },

        // ==================== Elixir ====================
        "phoenix" => vec![
            cmd_i("phoenix_new", "Create Phoenix project", "Scaffold Phoenix application",
                "mix", vec!["phx.new", project_name],
                vec![
                    InteractiveEntry {
                        trigger: "Fetch and install dependencies?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                    InteractiveEntry {
                        trigger: "Would you like to build assets?".into(),
                        response_type: ResponseType::Confirm(true),
                    },
                ]),
        ],

        _ => vec![
            cmd("fw_unknown", "Unknown framework",
                &format!("Framework '{}' has no specific setup steps", fw),
                "echo", vec![&format!("No automated setup available for framework: {}", fw)]),
        ],
    }
}


fn steps_for_tools(tools: &[String], project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut infra_envs: Vec<String> = Vec::new();

    let write_file = |id: &str, label: &str, path: &str, content: &str| -> Step {
        Step::WriteFile {
            id: id.to_string(),
            label: label.to_string(),
            description: format!("Create {}", path),
            path: path.to_string(),
            content: content.to_string(),
            overwrite: false,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    for tool_id in tools {
        match tool_id.as_str() {
            // Database tools
            "sqlalchemy" => {
                steps.push(write_file("sqlalchemy_config", "SQLAlchemy config",
                    "src/database.py",
                    r#"from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker, DeclarativeBase

DATABASE_URL = "postgresql://user:password@localhost:5432/dbname"

engine = create_engine(DATABASE_URL)
SessionLocal = sessionmaker(autocommit=False, autoflush=False, bind=engine)

class Base(DeclarativeBase):
    pass

def get_db():
    db = SessionLocal()
    try:
        yield db
    finally:
        db.close()
"#));
            }
            "alembic" => {
                steps.push(Step::Command {
                    id: "alembic_init".into(),
                    label: "Init Alembic".into(),
                    description: "Initialize Alembic migrations".into(),
                    command: "alembic".into(),
                    args: vec!["init".into(), "migrations".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(30),
                    condition: None,
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
            }
            "prisma" => {
                steps.push(Step::Command {
                    id: "prisma_init".into(),
                    label: "Init Prisma".into(),
                    description: "Initialize Prisma ORM".into(),
                    command: "npx".into(),
                    args: vec!["prisma".into(), "init".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(30),
                    condition: None,
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
            }
            "drizzle" => {
                steps.push(write_file("drizzle_config", "Drizzle config",
                    "drizzle.config.ts",
                    r#"import type { Config } from "drizzle-kit";

export default {
  schema: "./src/db/schema.ts",
  out: "./drizzle",
  driver: "pg",
  dbCredentials: {
    connectionString: process.env.DATABASE_URL!,
  },
} satisfies Config;
"#));
            }
            // Testing tools
            "pytest" => {
                steps.push(write_file("pytest_config", "Pytest config",
                    "pytest.ini",
                    r#"[pytest]
testpaths = tests
python_files = test_*.py
python_classes = Test*
python_functions = test_*
addopts = -v --tb=short
"#));
                steps.push(Step::CreateDirectory {
                    id: "create_tests_dir".into(),
                    label: "Create tests/".into(),
                    description: "Create tests directory".into(),
                    path: "tests".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }
            "ruff" => {
                steps.push(write_file("ruff_config", "Ruff config",
                    "ruff.toml",
                    r#"[lint]
select = ["E", "F", "I", "N", "W"]
ignore = []

[format]
quote-style = "double"
indent-style = "space"
"#));
            }
            "airflow" => {
                infra_envs.push(content::get_env_example("airflow"));
                steps.push(Step::CreateDirectory {
                    id: "create_dags_dir".into(),
                    label: "Create dags/".into(),
                    description: "Create Airflow DAGs directory".into(),
                    path: "dags".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
                steps.push(write_file("airflow_example_dag", "Example Airflow DAG",
                    "dags/example_dag.py",
                    r#"from datetime import datetime, timedelta

from airflow import DAG
from airflow.operators.python import PythonOperator

default_args = {"owner": "user", "retries": 1, "retry_delay": timedelta(minutes=5)}

with DAG(
    dag_id="example_dag",
    default_args=default_args,
    schedule="@daily",
    start_date=datetime(2024, 1, 1),
    catchup=False,
    tags=["example"],
) as dag:

    def print_hello() -> None:
        print("Hello from StackPilot Airflow!")

    hello = PythonOperator(task_id="print_hello", python_callable=print_hello)

    hello
"#));
            }
            // Infra tools — сервисы docker-compose; переменные окружения
            // собираем в один .env.example в конце (иначе каждый следующий
            // инструмент видел бы существующий файл и шаг скипался).
            "postgresql" | "redis" | "mongodb" | "mysql" | "kafka" | "clickhouse" | "rabbitmq" | "minio" | "mailpit" => {
                infra_envs.push(content::get_env_example(tool_id));
            }
            "grafana" | "opentelemetry" => {
                steps.push(write_file(&format!("config_{}", tool_id),
                    &format!("Config hint for {}", tool_id),
                    &format!("config/{}.md", tool_id),
                    &format!("# {} configuration\n\nSee documentation for setup details.\n", tool_id)));
            }
            "npm" | "gradle" | "maven" => {
                // Инструменты сборки — уже учтены в language/framework
                // Можно пропустить или добавить файлы конфигурации
            }
            _ => {
                // Для неизвестных — просто создаём директорию config/
                steps.push(Step::CreateDirectory {
                    id: format!("config_dir_{}", tool_id),
                    label: format!("Create config dir for {}", tool_id),
                    description: format!("Create configuration directory for {}", tool_id),
                    path: "config".into(),
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }
        }
    }

    // Один .env.example на все инфра-сервисы (пишется один раз — в цикле
    // выше шаги для каждого инструмента по отдельности скипались бы).
    if !infra_envs.is_empty() {
        let mut combined = String::new();
        for env in &infra_envs {
            combined.push_str(env);
            combined.push('\n');
        }
        steps.push(write_file(
            "env_example",
            "Create .env.example",
            ".env.example",
            &combined,
        ));
    }

    steps
}



fn steps_for_docker(context: &WizardContext, _project_path: &str, project_name: &str) -> Vec<Step> {
    if context.docker == false {return Vec::new()}

    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());

    let mut result = Vec::new();

    if let Some(dockerfile_content) = content::generate_dockerfile_content(primary_lang, primary_fw, project_name) {
        result.push(Step::WriteFile{
            id: "dockerfile".into(),
            label: "Create Dockerfile".into(),
            description: format!("Create Dockerfile for {} + {:?}", primary_lang, primary_fw),
            path: "Dockerfile".into(),
            content: dockerfile_content,
            overwrite: true,
            condition: None,
            on_error: ErrorMode::Skip
        })
    };
    result.push(Step::WriteFile { 
        id: ("docker_ignore".into()), 
        label: ("Create .dockerignore".into()), 
        description: ("Generate .dockerignore file".into()), 
        path: (".dockerignore".into()), 
        content: (content::dockerignore_content(primary_lang)),
        overwrite: (true), 
        condition: (None), 
        on_error: (ErrorMode::Skip) });

    let services = content::collect_docker_services(&context.tools);
    if !services.is_empty() {
        let app_port = match primary_fw {
            Some("django") | Some("fastapi") => "8000",
            Some("spring-boot") | Some("ktor") | Some("aspnetcore") => "8080",
            Some("laravel") | Some("symfony") => "8000",
            Some("phoenix") => "4000",
            _ => "3000",
        };
        result.push(Step::WriteFile{
            id: "docker_compose".into(),
            label: "Create docker-compose".into(),
            description: "Generate docker-compose file".into(),
            path: "docker-compose.yaml".into(),
            content: content::generate_docker_compose(&services, project_name, app_port),
            overwrite: true,
            condition: None,
            on_error: ErrorMode::Skip
        })
    }
    
    result
}

fn steps_for_git(context: &WizardContext, project_path: &str, project_name: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    
    if !context.git_init {
        return steps;
    }
    
    // .gitignore с контентом под все языки проекта
    let gitignore = content::gitignore_content(&context.languages);
    steps.push(Step::WriteFile {
        id: "gitignore".into(),
        label: "Create .gitignore".into(),
        description: "Generate .gitignore for project languages".into(),
        path: ".gitignore".into(),
        content: gitignore,
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    // Генераторы (create-electron-app, flutter create и т.п.) часто сами
    // инициализируют git во вложенных каталогах — `git add .` потом падает
    // с «'dir' does not have a commit checked out». Убираем вложенные .git,
    // корневой (созданный ранее пользователем или нами) не трогаем.
    steps.push(Step::Command {
        id: "git_cleanup_nested".into(),
        label: "Remove nested Git repositories".into(),
        description: "Remove nested .git directories left by generators".into(),
        command: format!(
            r#"Get-ChildItem -LiteralPath '{}' -Recurse -Force -Directory -Filter '.git' | Where-Object {{ $_.FullName -ne '{}' }} | Remove-Item -Recurse -Force"#,
            project_path,
            format!("{}\\.git", project_path)
        ),
        args: vec![],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(30),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });

    // git init
    steps.push(Step::Command {
        id: "git_init".into(),
        label: "Initialize Git repository".into(),
        description: "Run git init".into(),
        command: "git".into(),
        args: vec!["init".into()],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });
    
    // git add + commit (опционально)
    steps.push(Step::Command {
        id: "git_add".into(),
        label: "Stage all files".into(),
        description: "Run git add .".into(),
        command: "git".into(),
        args: vec!["add".into(), ".".into()],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });
    
    steps.push(Step::Command {
        id: "git_commit".into(),
        label: "Create initial commit".into(),
        description: "Run git commit with initial message".into(),
        command: "git".into(),
        args: vec![
            "commit".into(), 
            "-m".into(), 
            format!("Initial commit: {} project", project_name)
        ],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });
    
    steps
}



fn steps_for_ci(context: &WizardContext, _project_path: &str, project_name: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    
    if !context.ci {
        return steps;
    }
    
    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    
    // Создаём директорию .github/workflows
    steps.push(Step::CreateDirectory {
        id: "github_dir".into(),
        label: "Create .github directory".into(),
        description: "Create .github/workflows directory for CI".into(),
        path: ".github/workflows".into(),
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    let ci_content = content::generate_ci_content(primary_lang, primary_fw, project_name);
    
    steps.push(Step::WriteFile {
        id: "ci_workflow".into(),
        label: "Create CI workflow".into(),
        description: format!("Generate GitHub Actions workflow for {}", primary_lang),
        path: ".github/workflows/ci.yaml".into(),
        content: ci_content,
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    steps
}



fn steps_for_readme(context: &WizardContext, _project_path: &str, project_name: &str) -> Vec<Step> {
    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    let project_type = context.project_type.as_deref().unwrap_or("project");
    
    let readme = content::generate_readme(project_name, primary_lang, primary_fw, project_type, &context.tools);
    
    vec![Step::WriteFile {
        id: "readme".into(),
        label: "Create README.md".into(),
        description: "Generate README.md with project info".into(),
        path: "README.md".into(),
        content: readme,
        overwrite: false,
        condition: None,
        on_error: ErrorMode::Skip,
    }]
}




fn steps_for_vscode(context: &WizardContext) -> Vec<Step> {
    if !context.vscode_config {
        return Vec::new();
    }
    
    let mut steps = vec![
        Step::CreateDirectory {
            id: "vscode_dir".into(),
            label: "Create .vscode directory".into(),
            description: "Create .vscode directory for IDE settings".into(),
            path: ".vscode".into(),
            condition: None,
            on_error: ErrorMode::Skip,
        }
    ];
    
    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    
    // settings.json
    steps.push(Step::WriteFile {
        id: "vscode_settings".into(),
        label: "Create VS Code settings".into(),
        description: "Generate .vscode/settings.json".into(),
        path: ".vscode/settings.json".into(),
        content: content::generate_vscode_settings(primary_lang),
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    // extensions.json
    steps.push(Step::WriteFile {
        id: "vscode_extensions".into(),
        label: "Create VS Code extensions".into(),
        description: "Generate .vscode/extensions.json with recommended extensions".into(),
        path: ".vscode/extensions.json".into(),
        content: content::generate_vscode_extensions(primary_lang),
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    steps
}




// Helper methods on Step (нужны, так как enum не может иметь методов напрямую)
// ============================================================================

impl Step {
    pub fn id(&self) -> String {
        match self {
            Step::Command { id, .. } => id.clone(),
            Step::WriteFile { id, .. } => id.clone(),
            // Step::RenderTemplate { id, .. } => id.clone(),
            Step::CreateDirectory { id, .. } => id.clone(),
            Step::Generate { id, .. } => id.clone(),
            Step::Parallel { id, .. } => id.clone(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Step::Command { label, .. } => label.clone(),
            Step::WriteFile { label, .. } => label.clone(),
            // Step::RenderTemplate { label, .. } => label.clone(),
            Step::CreateDirectory { label, .. } => label.clone(),
            Step::Generate { label, .. } => label.clone(),
            Step::Parallel { label, .. } => label.clone(),
        }
    }

    pub fn description(&self) -> String {
        match self {
            Step::Command { description, .. } => description.clone(),
            Step::WriteFile { description, .. } => description.clone(),
            // Step::RenderTemplate { description, .. } => description.clone(),
            Step::CreateDirectory { description, .. } => description.clone(),
            Step::Generate { description, .. } => description.clone(),
            Step::Parallel { description, .. } => description.clone(),
        }
    }

    pub fn condition(&self) -> Option<&StepCondition> {
        match self {
            Step::Command { condition, .. } => condition.as_ref(),
            Step::WriteFile { condition, .. } => condition.as_ref(),
            // Step::RenderTemplate { condition, .. } => condition.as_ref(),
            Step::CreateDirectory { condition, .. } => condition.as_ref(),
            Step::Generate { condition, .. } => condition.as_ref(),
            Step::Parallel { condition: _, .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: vec!["typescript".into()],
            frameworks: vec!["nextjs".into()],
            ..Default::default()
        }
    }

    fn cmd_args(step: &Step) -> Vec<String> {
        match step {
            Step::Command { args, .. } => args.clone(),
            other => panic!("ожидался Command, получили {:?}", other.id()),
        }
    }

    #[test]
    fn frontend_frameworks_use_project_subfolder() {
        // Фронтенды создают проект в подпапке <project_name>, а не в корне:
        // иначе они перезапишут package.json бэкенда (express+nextjs и т.п.).
        for fw_id in ["nextjs", "nuxt", "sveltekit", "solidjs"] {
            let steps = steps_for_framework(fw_id, "C:\\dev\\myapp", "myapp", &context(), None);
            assert_eq!(steps.len(), 1, "{fw_id}");
            let args = cmd_args(&steps[0]);
            assert!(
                args.contains(&"myapp".to_string()),
                "{fw_id} не создаёт проект в подпапке: {args:?}"
            );
        }
    }

    #[test]
    fn split_layout_puts_frameworks_into_segments() {
        // nextjs + fastapi: фронтенд — в frontend/, сервер — в backend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let next_steps = steps_for_framework("nextjs", "C:\\dev\\myapp", "myapp", &ctx, Some("frontend"));
        let args = cmd_args(&next_steps[0]);
        assert!(args.contains(&"frontend".to_string()), "nextjs должен создаваться в frontend/: {args:?}");

        let api_steps = steps_for_framework("fastapi", "C:\\dev\\myapp", "myapp", &ctx, Some("backend"));
        assert!(
            api_steps.iter().any(|s| matches!(s, Step::WriteFile { path, .. } if path == "backend/src/main.py")),
            "fastapi должен писать в backend/src/main.py"
        );
    }

    #[test]
    fn solo_backend_framework_stays_in_root() {
        // Только fastapi (без фронтенда) — сегментации нет, файлы в корне
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];

        let api_steps = steps_for_framework("fastapi", "C:\\dev\\myapp", "myapp", &ctx, None);
        assert!(
            api_steps.iter().any(|s| matches!(s, Step::WriteFile { path, .. } if path == "src/main.py")),
            "fastapi без фронтенда пишет в корень"
        );
    }

    #[test]
    fn compose_recipe_creates_segment_dirs_for_split_stack() {
        // Полный план для nextjs + fastapi: создаются папки backend/ и frontend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"frontend".to_string()), "backend/ и frontend/ должны создаваться: {mkdirs:?}");
        assert!(mkdirs.contains(&"backend".to_string()), "backend/ и frontend/ должны создаваться: {mkdirs:?}");
    }

    #[test]
    fn inplace_framework_follows_its_own_side() {
        // express — бэкенд-фреймворк (side=backend в wizard_tree). Даже если
        // typescript назначен «фронтенд»-языком, express работает в backend/
        // (в новой модели комбинация «express + бэкенд на другом языке»
        // блокируется валидацией, а здесь проверяется размещение файлов).
        let mut ctx = context();
        ctx.languages = vec!["javascript".into(), "typescript".into()];
        ctx.backend_languages = vec!["javascript".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["express".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");

        // express-файлы пишутся в backend/ (своя сторона), а не в frontend/
        for (step_id, expected_path) in [("express_index", "backend/src/index.js"), ("express_package", "backend/package.json")] {
            let step = recipe.steps.iter().find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::WriteFile { path, overwrite, .. } => {
                    assert_eq!(path, expected_path, "{step_id} должен писать в {expected_path}");
                    assert!(*overwrite, "{step_id} обязан перезаписать заглушку языка");
                }
                _ => panic!("{step_id} — WriteFile"),
            }
        }
    }

    #[test]
    fn standalone_backend_plus_frontend_segments() {
        // aspnetcore (standalone, backend) + nextjs (standalone, frontend) —
        // валидный полный стек: каждый фреймворк создаётся в своём сегменте.
        let mut ctx = context();
        ctx.languages = vec!["csharp".into(), "typescript".into()];
        ctx.backend_languages = vec!["csharp".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["aspnetcore".into(), "nextjs".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");

        // aspnetcore (dotnet new webapi) выполняется в backend/
        let asp = recipe.steps.iter().find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command { working_dir, .. } => {
                let wd = working_dir.as_deref().expect("aspnetcore должен работать в backend/");
                assert!(
                    wd.ends_with("backend"),
                    "aspnetcore должен работать в backend/, а не в корне: {wd}"
                );
            }
            _ => panic!("aspnet_new — Command"),
        }

        // nextjs (create-next-app) — folder-maker: рабочая директория остаётся
        // корнем, но создаваемая подпапка переименовывается в сегмент frontend/
        let next = recipe.steps.iter().find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Command { args, .. } => {
                assert_eq!(args.get(1).map(String::as_str), Some("frontend"),
                    "nextjs должен создаваться в frontend/, а не в корне: {args:?}");
            }
            _ => panic!("nextjs_create — Command"),
        }

        // Скаффолд csharp (dotnet new console) подавлен aspnetcore,
        // js-скаффолд подавлен nextjs
        for suppressed in ["dotnet_new", "package_json", "js_src_index"] {
            assert!(
                !recipe.steps.iter().any(|s| s.id() == suppressed),
                "шаг {suppressed} не должен выполняться: aspnetcore/nextjs создают каркас сами"
            );
        }
    }

    #[test]
    fn rust_tauri_keeps_cargo_init() {
        // tauri НЕ подавляет язык: cargo tauri init требует существующий
        // cargo-проект. Отдельный кейс против слепого подавления скаффолда.
        let mut ctx = context();
        ctx.languages = vec!["rust".into()];
        ctx.frameworks = vec!["tauri".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().any(|s| s.id() == "cargo_init"),
            "cargo init обязателен перед cargo tauri init"
        );
        assert!(recipe.steps.iter().any(|s| s.id() == "tauri_init"));
    }

    #[test]
    fn python_scaffold_kept_for_fastapi_requirements_overwrite() {
        // python-скаффолд (pyproject.toml) остаётся — fastapi его не создаёт,
        // а requirements.txt fastapi перезаписывает (иначе зависимости терялись).
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        assert!(recipe.steps.iter().any(|s| s.id() == "pyproject_toml"));
        let reqs = recipe.steps.iter().find(|s| s.id() == "fastapi_requirements").unwrap();
        match reqs {
            Step::WriteFile { path, overwrite, .. } => {
                assert_eq!(path, "requirements.txt");
                assert!(*overwrite, "fastapi должен перезаписать requirements.txt");
            }
            _ => panic!("fastapi_requirements — WriteFile"),
        }
    }

    #[test]
    fn inplace_framework_entry_files_overwrite() {
        // Inplace-фреймворки (scaffold=None) перезаписывают entry-файлы,
        // иначе их шаги молча скипались на файлах language-скаффолда.
        for (fw, entry_ids) in [
            ("express", vec!["express_index", "express_package"]),
            ("gin", vec!["gin_main"]),
            ("clap", vec!["clap_main"]),
            ("axum", vec!["axum_main"]),
            ("flask", vec!["flask_app", "flask_requirements"]),
        ] {
            let steps = steps_for_framework(fw, "C:\\dev\\myapp", "myapp", &context(), None);
            for id in &entry_ids {
                let step = steps.iter().find(|s| s.id() == id.to_string())
                    .unwrap_or_else(|| panic!("{fw}: шаг {id} должен существовать"));
                match step {
                    Step::WriteFile { overwrite, .. } => assert!(*overwrite, "{fw}: шаг {id} должен перезаписываться"),
                    _ => panic!("{fw}: {id} — WriteFile"),
                }
            }
        }
    }

    #[test]
    fn complex_monolith_builds_full_recipe() {
        // Сложный монолит: python + typescript на одной стороне (без
        // сегментов), fastapi (inplace) + react (vite-подпапка) + airflow
        // + postgres. Всё создаётся в корне проекта.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into(), "typescript".into()];
        ctx.frontend_languages = vec![];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["airflow".into(), "postgresql".into()];
        ctx.docker = true;

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // Монолит: без backend/ и frontend/ сегментов
        assert!(
            !recipe.steps.iter().any(|s| matches!(s, Step::CreateDirectory { path, .. }
                if path == "backend" || path == "frontend")),
            "в монолите не должно быть сегментов"
        );

        // react: create-vite создаёт подпапку с именем проекта
        let vite = recipe.steps.iter().find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match vite {
            Step::Command { args, .. } => {
                assert_eq!(args.get(0).map(String::as_str), Some("create-vite@latest"));
                assert_eq!(args.get(1).map(String::as_str), Some("myapp"),
                    "create-vite должен создавать папку с именем проекта: {args:?}");
                assert_eq!(args.get(3).map(String::as_str), Some("react-ts"),
                    "typescript → react-ts шаблон: {args:?}");
            }
            _ => panic!("vite_create — Command"),
        }

        // npm install выполняется ВНУТРИ созданной vite-папки
        let install = recipe.steps.iter().find(|s| s.id() == "vite_install")
            .expect("vite_install должен быть в плане");
        match install {
            Step::Command { working_dir, args, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/myapp"));
                assert_eq!(args, &vec!["install".to_string()]);
            }
            _ => panic!("vite_install — Command"),
        }

        // fastapi (inplace): entry-файлы в корне проекта
        let main = recipe.steps.iter().find(|s| s.id() == "fastapi_main")
            .expect("fastapi_main должен быть в плане");
        match main {
            Step::WriteFile { path, .. } => assert_eq!(path, "src/main.py"),
            _ => panic!("fastapi_main — WriteFile"),
        }

        // airflow: dags/ директория + пример DAG
        assert!(
            recipe.steps.iter().any(|s| matches!(s, Step::CreateDirectory { path, .. } if path == "dags")),
            "airflow должен создать dags/"
        );
        assert!(
            recipe.steps.iter().any(|s| s.id() == "airflow_example_dag"),
            "airflow должен создать example_dag.py"
        );

        // docker-compose: airflow + postgres
        let compose = recipe.steps.iter().find(|s| s.id() == "docker_compose")
            .expect("docker_compose должен быть в плане");
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("airflow"), "compose должен включать airflow");
                assert!(content.contains("postgres"), "compose должен включать postgres");
            }
            _ => panic!("docker_compose — WriteFile"),
        }

        // .env.example: переменные airflow и postgres
        let env = recipe.steps.iter().find(|s| s.id() == "env_example")
            .expect("env_example должен быть в плане");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("AIRFLOW__CORE__EXECUTOR"));
                assert!(content.contains("POSTGRES_USER"));
            }
            _ => panic!("env_example — WriteFile"),
        }
    }

    #[test]
    fn guard_legal_pairs_produce_no_duplicates() {
        // Легальные связки разводятся движком по разным путям — guard молчит.
        let cases: Vec<(Vec<&str>, Vec<&str>)> = vec![
            (vec!["gin", "cobra"], vec!["go"]),
            (vec!["axum", "clap"], vec!["rust"]),
            (vec!["zap", "zig-cli"], vec!["zig"]),
            (vec!["fastapi", "aiogram"], vec!["python"]),
            (vec!["electron", "react"], vec!["typescript"]),
            (vec!["tauri", "svelte"], vec!["rust", "typescript"]),
        ];
        for (fws, langs) in cases {
            let mut ctx = context();
            ctx.frameworks = fws.into_iter().map(String::from).collect();
            ctx.languages = langs.into_iter().map(String::from).collect();
            let issues = duplicate_framework_write_paths(&ctx);
            assert!(issues.is_empty(), "ошибки для легальной связки: {issues:?}");
        }
    }

    #[test]
    fn guard_colliding_frameworks_are_reported() {
        // express и fastify пишут src/index.js и package.json в один и тот
        // же каталог (inplace, без сегментации): второй пишущий молча
        // скипнется — сгенерированный проект сломается.
        let mut ctx = context();
        ctx.languages = vec!["javascript".into()];
        ctx.frameworks = vec!["express".into(), "fastify".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(
            issues.iter().any(|i| i.contains("src/index.js")),
            "ожидался конфликт по src/index.js: {issues:?}"
        );
    }

    #[test]
    fn guard_split_stack_with_tauri_react_is_clean() {
        // tauri (either, язык rust → backend/) + react (frontend/) в
        // mono-репозитории не пересекаются по путям.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.backend_languages = vec!["rust".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];
        let issues = duplicate_framework_write_paths(&ctx);
        assert!(issues.is_empty(), "ложные срабатывания: {issues:?}");
    }
}
