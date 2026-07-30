pub mod content;
pub mod executor;
// pub mod template;
use std::path::{Path};
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
    
    for lang in &context.languages {
        steps.extend(steps_for_language(lang, project_name, project_path));
    }
    for fw in &context.frameworks {
        steps.extend(steps_for_framework(fw, project_path, project_name, context));
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
            on_error: ErrorMode::Abort,
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
                    "npx tsc --init --target ES2022 --module commonjs --outDir dist --rootDir src"));
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
                &format!("dotnet new console -n {}", project_name)),
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

        "dart" => vec![
            cmd("dart_create", "Create Dart project",
                &format!("Create new Dart project '{}'", project_name),
                &format!("dart create {}", project_name)),
        ],

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

fn steps_for_framework(fw: &str, project_path: &str, project_name: &str, context: &WizardContext) -> Vec<Step> {
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

        "clap" => vec![
            cmd("add_clap_deps", "Add Clap dependency", "Add clap with derive feature",
                "cargo", vec!["add", "clap", "--features", "derive"]),
            write_file("clap_main", "Create CLI entry point", "src/main.rs",
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
        ],

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

        "django" => vec![
            cmd("django_start", "Start Django project", "Create Django project structure",
                "django-admin", vec!["startproject", project_name, "."]),
        ],

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

        "aiogram" => vec![
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
            write_file("aiogram_requirements", "Aiogram dependencies", "requirements.txt",
                "aiogram\n"),
        ],

        // ==================== JavaScript / TypeScript ====================
        "nextjs" => vec![
            cmd_i("nextjs_create", "Create Next.js app", "Scaffold Next.js project",
                "npx", vec!["create-next-app@latest", ".", "--typescript", "--tailwind", "--eslint", "--app", "--no-src-dir", "--import-alias", "@/*", "--use-npm"],
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
                    "npx", vec!["sv", "create", "."],
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
                "npx", vec!["nuxi", "init", "."],
                vec![
                    InteractiveEntry {
                        trigger: "Which package manager would you like to use?".into(),
                        response_type: ResponseType::Text("npm".to_string()),
                    },
                    InteractiveEntry {
                        trigger: "Initialize a new git repository?".into(),
                        response_type: ResponseType::Confirm(false),
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

        "react-native" => vec![
            cmd_i("rn_init", "Init React Native", "Create React Native project",
                "npx", vec!["@react-native-community/cli", "init", project_name],
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
        ],

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
                "npx", vec!["@nestjs/cli", "new", project_name, "--package-manager", "npm"],
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
                "npx", vec!["create-solid", "."],
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

        "cobra" => vec![
            cmd("cobra_init", "Init Cobra CLI", "Initialize Cobra CLI project",
                "go", vec!["get", "github.com/spf13/cobra/cobra"]),
            write_file("cobra_main", "Create CLI entry", "cmd/main.go",
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
        ],

        // ==================== Java ====================
        "spring-boot" => vec![
            // Spring Boot обычно создаётся через Spring Initializr — curl + unzip
            // Это сложная команда, оставляем как есть
            cmd("spring_init", "Generate Spring Boot project",
                "Download Spring Boot starter from Initializr",
                "curl", vec![
                    "-s", &format!("https://start.spring.io/starter.zip?name={}&groupId=com.example&artifactId={}&dependencies=web", project_name, project_name),
                    "-o", "project.zip",
                ]),
            cmd("unzip_spring", "Extract Spring Boot", "Unzip the generated project",
                if cfg!(target_os = "windows") { "tar" } else { "unzip" },
                vec!["project.zip"]),
            cmd("cleanup_zip", "Clean up zip", "Remove project.zip",
                if cfg!(target_os = "windows") { "del" } else { "rm" },
                vec!["project.zip"]),
        ],

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
                "dotnet", vec!["new", "webapi", "-n", project_name]),
        ],

        "unity" => vec![
            cmd("unity_hint", "Unity project hint",
                "Unity projects are created through Unity Hub",
                "echo", vec!["Create this project through Unity Hub with the same name"]),
        ],

        "maui" => vec![
            cmd("maui_new", "Create MAUI app", "Scaffold .NET MAUI project",
                "dotnet", vec!["new", "maui", "-n", project_name]),
        ],

        "godot" => vec![
            cmd("godot_hint", "Godot project hint",
                "Godot projects are created through the Godot Editor",
                "echo", vec!["Create this project through Godot Engine Editor"]),
        ],

        // ==================== C++ ====================
        "unreal" => vec![
            cmd("unreal_hint", "Unreal Engine hint",
                "Unreal projects must be created through Epic Games Launcher",
                "echo", vec!["Create this project through Unreal Engine Editor"]),
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
        "flutter" => vec![
            cmd("flutter_create", "Create Flutter project", "Scaffold Flutter app",
                "flutter", vec!["create", project_name]),
        ],

        // ==================== Kotlin ====================
        "jetpack-compose" => vec![
            cmd("compose_hint", "Jetpack Compose hint",
                "Jetpack Compose projects are created through Android Studio",
                "echo", vec!["Create this project through Android Studio with Jetpack Compose template"]),
        ],

        "ktor" => vec![
            write_file("ktor_main", "Create Ktor entry", "src/main/kotlin/Application.kt",
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
            cmd_i("laravel_new", "Create Laravel project",
                "Scaffold Laravel application",
                "npx", vec!["laravel", "new", project_name],
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
        "zig-cli" => vec![
            // zig init-exe уже сделано в language step, здесь просто меняем main
            write_file("zig_main", "Create Zig CLI entry", "src/main.zig",
                &format!(r#"const std = @import("std");

pub fn main() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}}!\n", .{{"{}"}});
}}
"#, project_name)),
        ],

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
            // Infra tools — просто маркеры, compose соберёт steps_for_docker
            "postgresql" | "redis" | "mongodb" | "mysql" | "kafka" | "clickhouse" | "rabbitmq" | "minio" | "mailpit" => {
                // Эти инструменты будут добавлены как сервисы в docker-compose
                // Здесь можно создать .env.example с переменными окружения
                steps.push(write_file(&format!("env_{}", tool_id), 
                    &format!("Environment for {}", tool_id),
                    ".env.example",
                    &content::get_env_example(tool_id)));
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
