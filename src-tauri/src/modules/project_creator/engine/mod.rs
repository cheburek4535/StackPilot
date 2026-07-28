pub mod executor;
pub mod template;
use std::path::{Path};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

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
    pub template_engine: Arc<template::TemplateEngine>,
    pub executor: Arc<executor::StepExecutor>,
}

impl DefaultRecipeEngine {
    pub fn new() -> Self {
        Self {
            template_engine: Arc::new(template::TemplateEngine::new()),
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
        let recipe = compose_recipe(context)?;
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
fn compose_recipe(context: &WizardContext) -> Result<Recipe, String> {
    let mut steps: Vec<Step> = Vec::new();
    let project_path = context.project_path.as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or(".");
    
    let project_name = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");

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
        steps.extend(steps_for_framework(fw, project_path, project_name));
    }
    steps.extend(steps_for_tools(&context.tools, project_path));
    steps.extend(steps_for_docker(context, project_path));
    steps.extend(steps_for_git(context, project_path));
    steps.extend(steps_for_ci(context, project_path));
    steps.extend(steps_for_readme(context, project_path));
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
    // Simplified; in production use chrono
    "now".to_string()
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
            timeout_secs: Some(60), // Разумный таймаут по умолчанию
            condition: None,
            on_error: ErrorMode::Abort,
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
                    content: format!(r#"
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
                on_error: ErrorMode::Skip, // Maven может отсутствовать
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
            },
        ],
    }
}

fn steps_for_framework(fw: &str, project_path: &str, project_name: &str) -> Vec<Step> {
    let cmd = |id: &str, label: &str, desc: &str, command: &str, args: Vec<&str>| -> Step {
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: command.to_string(),
            args: args.into_iter().map(String::from).collect(),
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(300), // Некоторые фреймворки долго ставятся
            condition: None,
            on_error: ErrorMode::Skip, // Не у всех установлены глобальные CLI
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

        "tauri" => vec![
            cmd("tauri_init", "Init Tauri", "Initialize Tauri in current project",
                "cargo", vec!["tauri", "init", "--app-name", project_name, "--window-title", project_name, "--dev-url", "http://localhost:1420", "--before-dev-command", "", "--before-build-command", ""]),
        ],

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
            cmd("nextjs_create", "Create Next.js app", "Scaffold Next.js project",
                "npx", vec!["create-next-app@latest", ".", "--typescript", "--tailwind", "--eslint", "--app", "--no-src-dir", "--import-alias", "@/*", "--use-npm"]),
        ],

        "sveltekit" => vec![
            cmd("sveltekit_create", "Create SvelteKit app", "Scaffold SvelteKit project",
                "npx", vec!["sv", "create", "."]),
        ],

        "nuxt" => vec![
            cmd("nuxt_create", "Create Nuxt app", "Scaffold Nuxt project",
                "npx", vec!["nuxi", "init", "."]),
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
            cmd("electron_init", "Init Electron", "Create Electron app with electron-forge",
                "npx", vec!["create-electron-app", project_name]),
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
            cmd("rn_init", "Init React Native", "Create React Native project",
                "npx", vec!["@react-native-community/cli", "init", project_name]),
        ],

        "expo" => vec![
            cmd("expo_init", "Init Expo", "Create Expo project",
                "npx", vec!["create-expo-app", project_name]),
        ],

        "plasmo" => vec![
            cmd("plasmo_init", "Init Plasmo", "Create browser extension with Plasmo",
                "npx", vec!["plasmo", "init", project_name]),
        ],

        "nest" => vec![
            cmd("nest_new", "Create NestJS project", "Scaffold NestJS application",
                "npx", vec!["@nestjs/cli", "new", project_name, "--package-manager", "npm"]),
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
            cmd("solid_init", "Create SolidStart app", "Scaffold SolidStart project",
                "npx", vec!["create-solid", "."]),
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
            cmd("laravel_new", "Create Laravel project",
                "Scaffold Laravel application",
                "composer", vec!["create-project", "laravel/laravel", project_name]),
        ],

        "symfony" => vec![
            cmd("symfony_new", "Create Symfony project",
                "Scaffold Symfony application",
                "composer", vec!["create-project", "symfony/skeleton", project_name]),
        ],

        // ==================== Swift ====================
        "swiftui" => vec![
            cmd("swiftui_hint", "SwiftUI project hint",
                "SwiftUI projects are created through Xcode",
                "echo", vec!["Create this project through Xcode with SwiftUI template"]),
        ],

        "vapor" => vec![
            cmd("vapor_new", "Create Vapor project", "Scaffold Vapor application",
                "vapor", vec!["new", project_name]),
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
            cmd("phoenix_new", "Create Phoenix project", "Scaffold Phoenix application",
                "mix", vec!["phx.new", project_name]),
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
                    &get_env_example(tool_id)));
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

fn get_env_example(tool_id: &str) -> String {
    match tool_id {
        "postgresql" => r#"POSTGRES_USER=user
POSTGRES_PASSWORD=password
POSTGRES_DB=dbname
POSTGRES_PORT=5432
DATABASE_URL=postgresql://user:password@localhost:5432/dbname
"#.to_string(),
        "redis" => r#"REDIS_URL=redis://localhost:6379/0
"#.to_string(),
        "mongodb" => r#"MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=dbname
"#.to_string(),
        "mysql" => r#"MYSQL_ROOT_PASSWORD=rootpassword
MYSQL_DATABASE=dbname
MYSQL_USER=user
MYSQL_PASSWORD=password
MYSQL_PORT=3306
"#.to_string(),
        _ => format!("# Environment variables for {}\n", tool_id),
    }
}


struct DockerService {
    name: String,
    image: String,
    ports: Vec<String>,
    environment: Vec<(String, String)>,
    volumes: Vec<String>,
    depends_on: Vec<String>
}

fn steps_for_docker(context: &WizardContext, project_path: &str) -> Vec<Step> {
    if context.docker == false {return Vec::new()}

    let project_name = Path::new(project_path).file_name().and_then(|f| f.to_str()).unwrap_or("app");

    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());

    let mut result = Vec::new();

    if let Some(dockerfile_content) = generate_dockerfile_content(primary_lang, primary_fw, project_name) {
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
        content: (dockerignore_content(primary_lang)),
        overwrite: (true), 
        condition: (None), 
        on_error: (ErrorMode::Skip) });

    let services = collect_docker_services(&context.tools);
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
            content: generate_docker_compose(&services, project_name, app_port),
            overwrite: true,
            condition: None,
            on_error: ErrorMode::Skip
        })
    }
    
    result
}

fn collect_docker_services(tools: &[String]) -> Vec<DockerService> {
    let mut services = Vec::new();
    
    for tool in tools.iter() {
        match tool.as_str() {
            "postgresql" => services.push(DockerService {
                name: "postgres".into(),
                image: "postgres:16-alpine".into(),
                ports: vec!["5432:5432".into()],
                environment: vec![
                    ("POSTGRES_USER".into(), "postgres".into()),
                    ("POSTGRES_PASSWORD".into(), "12345".into()),
                    ("POSTGRES_DB".into(), "postgres".into()),
                ],
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),
            
            "redis" => services.push(DockerService { 
                name: "redis".into(),
                image: "redis:7-alpine".into(),
                ports: vec!["6379:6379".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mongodb" => services.push(DockerService {
                name: "mongodb".into(),
                image: "mongo:7".into(),
                ports: vec!["27017:27017".into()],
                environment: vec![
                    ("MONGO_INITDB_ROOT_USERNAME".into(), "root".into()),
                    ("MONGO_INITDB_ROOT_PASSWORD".into(), "example".into()),
                ],
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mysql" => services.push(DockerService {
                name: "mysql".into(),
                image: "mysql:8".into(),
                ports: vec!["3306:3306".into()],
                environment: vec![
                    ("MYSQL_ROOT_PASSWORD".into(), "root_pwd".into()),
                    ("MYSQL_DATABASE".into(), "mydb".into()),
                ],
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "kafka" => {
                services.push(DockerService {
                    name: "zookeeper".into(),
                    image: "confluentinc/cp-zookeeper:latest".into(),
                    ports: vec!["2181:2181".into()],
                    environment: vec![
                        ("ZOOKEEPER_CLIENT_PORT".into(), "2181".into()),
                        ("ZOOKEEPER_TICK_TIME".into(), "2000".into()),
                    ],
                    volumes: Vec::new(),
                    depends_on: Vec::new(),
                });

                services.push(DockerService {
                    name: "kafka".into(),
                    image: "confluentinc/cp-kafka:latest".into(),
                    ports: vec!["9092:9092".into()],
                    environment: vec![
                        ("KAFKA_BROKER_ID".into(), "1".into()),
                        ("KAFKA_ZOOKEEPER_CONNECT".into(), "zookeeper:2181".into()),
                        ("KAFKA_ADVERTISED_LISTENERS".into(), "PLAINTEXT://localhost:9092".into()),
                        ("KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR".into(), "1".into()),
                    ],
                    volumes: Vec::new(),
                    depends_on: vec!["Zookeeper".into()],
                });
            },

            "clickhouse" => services.push(DockerService {
                name: "clickHouse".into(),
                image: "clickhouse/clickhouse-server:latest".into(),
                ports: vec!["8123:8123".into(), "9000:9000".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mailpit" => services.push(DockerService {
                name: "mailpit".into(),
                image: "axllent/mailpit:latest".into(),
                ports: vec!["1025:1025".into(), "8025:8025".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            _ => {}
        }
    }
    
    services
}

/// Генерирует готовый контент Dockerfile как строку.
/// Возвращает Option, потому что для некоторых фреймворков Docker не нужен (например, Tauri).
fn generate_dockerfile_content(
    lang: &str, 
    framework: Option<&str>, 
    project_name: &str
) -> Option<String> {
    match lang {
        "python" => {
            // Базовые значения для Python
            let (base_image, entrypoint, port) = match framework {
                Some("fastapi") => (
                    "python:3.13-slim",
                    "src/main.py",
                    "3000"
                ),
                Some("django") => (
                    "python:3.13-slim", 
                    "manage.py",        // Django запускается иначе
                    "8000"              // Django default port
                ),
                Some("flask") => (
                    "python:3.13-slim",
                    "src/app.py",
                    "3000"
                ),
                _ => (
                    "python:3.13-slim",
                    "src/main.py",
                    "3000"
                ),
            };

            Some(format!(r#"FROM {base_image}

WORKDIR /app

# Install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application code
COPY . .

# Expose the port
EXPOSE {port}

# Run the application
CMD ["python", "{entrypoint}"]
"#))
        }

        "rust" => {
            let (port, bin_name) = match framework {
                Some("axum") => ("3000", project_name),
                Some("clap") => return None, // CLI не нужен Docker
                _ => ("3000", project_name),
            };

            Some(format!(r#"# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app
COPY --from=builder /app/target/release/{bin_name} .

EXPOSE {port}

CMD ["./{bin_name}"]
"#))
        }

        "typescript" | "javascript" | "node" => {
    let (base_image, needs_build, entrypoint, port, build_steps) = match framework {
        Some("nextjs") => (
            "node:22-alpine",
            true,
            "node_modules/.bin/next",
            "3000",
            "RUN npm run build\n",  // Next.js запускается через next start
        ),
        Some("nuxt") => (
            "node:22-alpine",
            true,
            ".output/server/index.mjs",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("sveltekit") => (
            "node:22-alpine",
            true,
            "build/index.js",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("nest") => (
            "node:22-alpine",
            true,
            "dist/main.js",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("fastify") | Some("express") => (
            "node:22-alpine",
            false,
            "src/index.js",
            "3000",
            "",
        ),
        _ => (
            "node:22-alpine",
            false,
            "src/index.js",
            "3000",
            "",
        ),
    };

    if needs_build {
        // Для фреймворков, которым нужна сборка
        Some(format!(r#"FROM {base_image}

WORKDIR /app

# Install all dependencies (including dev for build)
COPY package*.json ./
RUN npm ci

# Copy source and build
COPY . .
RUN npm run build

# Prune dev dependencies for production
RUN npm prune --production

EXPOSE {port}

CMD ["node", "{entrypoint}"]
"#))
    } else {
        // Для простых серверов без сборки
        Some(format!(r#"FROM {base_image}

WORKDIR /app

# Install production dependencies only
COPY package*.json ./
RUN npm ci --only=production

# Copy application code
COPY . .

EXPOSE {port}

CMD ["node", "{entrypoint}"]
"#))
    }
}

    "go" => {
            if framework == Some("cobra") {return None}
            Some(format!(r#"# Build stage
FROM golang:1.24-alpine AS builder

WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o app ./cmd/main.go

# Runtime stage
FROM alpine:3.21

WORKDIR /app
COPY --from=builder /app/app .

EXPOSE 3000

CMD ["./app"]
"#))
    }
    "csharp" => {
    let (runtime_image, port, project_file) = match framework {
        Some("aspnetcore") => (
            "mcr.microsoft.com/dotnet/aspnet:8.0",
            "EXPOSE 8080",
            format!("{}.csproj", project_name)
        ),
        _ => (
            "mcr.microsoft.com/dotnet/runtime:8.0",
            "",
            format!("{}.csproj", project_name)
        ),
    };

    Some(format!(r#"FROM mcr.microsoft.com/dotnet/sdk:8.0 AS build
WORKDIR /src
COPY {project_file} .
RUN dotnet restore
COPY . .
RUN dotnet publish -c Release -o /app/publish

FROM {runtime_image} AS final
WORKDIR /app
{port}
COPY --from=build /app/publish .
ENTRYPOINT ["dotnet", "{project_name}.dll"]
"#))
},
    "java" => {
        let has_spring = framework == Some("spring-boot");
        if !has_spring {
            return None; // Только Spring Boot поддерживает Docker из коробки
        }
    Some(format!(r#"FROM eclipse-temurin:21-jdk-alpine AS build
WORKDIR /app
COPY mvnw pom.xml ./
COPY .mvn .mvn
RUN ./mvnw dependency:go-offline
COPY src ./src
RUN ./mvnw package -DskipTests

FROM eclipse-temurin:21-jre-alpine AS final
WORKDIR /app
COPY --from=build /app/target/*.jar app.jar
EXPOSE 8080
ENTRYPOINT ["java", "-jar", "app.jar"]
"#))
},
    "php" => {
    let has_laravel = framework == Some("laravel") || framework == Some("symfony");
    if !has_laravel {
        return None;
    }
    Some(format!(r#"FROM php:8.3-fpm-alpine

RUN docker-php-ext-install pdo pdo_mysql

COPY --from=composer:2 /usr/bin/composer /usr/bin/composer

WORKDIR /app
COPY composer.json composer.lock ./
RUN composer install --no-dev --optimize-autoloader

COPY . .
RUN php artisan config:cache || true

EXPOSE 8000
CMD ["php", "artisan", "serve", "--host=0.0.0.0", "--port=8000"]
"#))
},
    "elixir" => {
    let is_phoenix = framework == Some("phoenix");
    if !is_phoenix {
        return None;
    }
    Some(format!(r#"FROM hexpm/elixir:1.17-erlang-27-alpine AS build
WORKDIR /app
RUN mix local.hex --force && mix local.rebar --force
COPY mix.exs mix.lock ./
RUN mix deps.get --only prod
COPY . .
RUN mix compile

FROM hexpm/elixir:1.17-erlang-27-alpine AS final
WORKDIR /app
COPY --from=build /app/_build/prod/rel/{project_name} .
EXPOSE 4000
CMD ["./bin/{project_name}", "start"]
"#))
},
    "cpp" => {
        let is_qt = framework == Some("qt");
        if is_qt {
            Some(format!(r#"FROM stateoftheartio/qt6:6.7-gcc-ubuntu-24.04 AS build
WORKDIR /app
COPY . .
RUN mkdir build && cd build && \
    qt-cmake -DCMAKE_BUILD_TYPE=Release .. && \
    cmake --build .

FROM ubuntu:24.04 AS final
RUN apt-get update && apt-get install -y \
    libqt6gui6 libqt6core6 libqt6widgets6 \
    libgl1-mesa-glx \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/build/{project_name} .
ENTRYPOINT ["./{project_name}", "-platform", "offscreen"]"#))
        } else {
            Some(format!(r#"FROM ubuntu:24.04 AS build
RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN mkdir build && cd build && \
    cmake -DCMAKE_BUILD_TYPE=Release .. && \
    cmake --build .

FROM ubuntu:24.04 AS final
WORKDIR /app

COPY --from=build /app/build/{project_name} .
ENTRYPOINT ["./{project_name}"]"#))
        }
    },
    "zig" => {
Some(format!(r#"FROM ziglang/zig:0.13.0 AS build
WORKDIR /app

COPY build.zig build.zig.zon ./
COPY src/ ./src/

RUN zig build -Doptimize=ReleaseFast

FROM scratch
WORKDIR /

COPY --from=build /app/zig-out/bin/{project_name} /{project_name}

ENTRYPOINT ["/{project_name}"]
"#))
    },
    "kotlin" => {
    let has_fw = framework == Some("ktor") || framework == Some("spring-boot");
    if has_fw {
        Some(format!(r#"FROM gradle:8.10-jdk21 AS build
WORKDIR /app
COPY build.gradle.kts settings.gradle.kts ./
RUN gradle dependencies --no-daemon
COPY src ./src
RUN gradle build -x test --no-daemon

FROM eclipse-temurin:21-jre-alpine AS final
WORKDIR /app
EXPOSE 8080
# Копируем все JAR файлы и находим тот, что без plain/sources
COPY --from=build /app/build/libs/ ./libs/
RUN cp $(ls ./libs/*.jar | grep -v -E 'plain|sources|javadoc') app.jar
ENTRYPOINT ["java", "-jar", "app.jar"]
"#))
    } else {
        None  // Без фреймворка не генерируем Dockerfile
    }
},
    "swift" => {
    let has_fw = framework == Some("vapor");
    if has_fw {
        Some(format!(r#"FROM swift:6.0-noble AS build
WORKDIR /build
COPY Package.swift Package.resolved ./
RUN swift package resolve
COPY . .
RUN swift build -c release --static-swift-backtrace

FROM swift:6.0-noble-slim AS final
WORKDIR /app
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
EXPOSE 8080
COPY --from=build /build/.build/release/{project_name} ./App
COPY --from=build /build/Public ./Public
ENTRYPOINT ["./App"]
"#))
    } else {
        Some(format!(r#"FROM swift:6.0-noble AS build
WORKDIR /build
COPY Package.swift ./
RUN swift package resolve
COPY . .
RUN swift build -c release

FROM swift:6.0-noble-slim AS final
WORKDIR /app
COPY --from=build /build/.build/release/{project_name} ./
ENTRYPOINT ["./{project_name}"]
"#))
    }
},
    "dart" => {
        Some(format!(r#"# Этап сборки
FROM dart:3.5 AS build
WORKDIR /app
COPY pubspec.yaml ./
RUN dart pub get
COPY . .
RUN dart compile exe bin/main.dart -o bin/main

# Этап запуска
FROM scratch
COPY --from=build /app/bin/main /main
ENTRYPOINT ["/main"]
"#))
    },
    "gleam" => {
    Some(format!(r#"FROM ghcr.io/gleam-lang/gleam:v1.4-erlang-alpine AS build
WORKDIR /app
COPY gleam.toml manifest.toml ./
RUN gleam deps download
COPY . .
RUN gleam export erlang-shipment

FROM erlang:27-alpine AS final
WORKDIR /app
COPY --from=build /app/build/erlang-shipment ./
ENTRYPOINT ["/app/entrypoint.sh"]
"#))
},
        _ => None, // Неизвестный язык — не генерируем Dockerfile
    }
}

fn generate_docker_compose(services: &[DockerService], project_name: &str, app_port: &str) -> String {
    let mut result = String::new();
    let mut volumes_section = String::new();

    result.push_str("version: '3.8'\n\nservices:\n");
    
    // App service всегда добавляется
    result.push_str(&format!(r#"  app:
    build: .
    container_name: {project_name}_app
    ports:
      - "{app_port}:{app_port}"
"#));

    // Если есть сервисы — добавляем depends_on
    if !services.is_empty() {
        result.push_str("    depends_on:\n");
        for service in services.iter() {
            result.push_str(&format!("      - {}\n", service.name));
        }
        result.push_str("    environment:\n");
        // Стандартные переменные окружения для подключения к сервисам
        for service in services.iter() {
            match service.name.as_str() {
                "postgres" => result.push_str("      - DATABASE_URL=postgresql://user:password@postgres:5432/dbname\n"),
                "redis" => result.push_str("      - REDIS_URL=redis://redis:6379/0\n"),
                "mongodb" => result.push_str("      - MONGODB_URI=mongodb://mongodb:27017\n"),
                _ => {}
            }
        }
    }
    result.push('\n');

    // Сервисы БД/кешей
    for service in services.iter() {
        result.push_str(&format!("  {}:\n", service.name));
        result.push_str(&format!("    image: {}\n", service.image));
        result.push_str(&format!("    container_name: {}_{}\n", project_name, service.name));
        
        if !service.ports.is_empty() {
            result.push_str("    ports:\n");
            for port in &service.ports {
                result.push_str(&format!("      - \"{}\"\n", port));
            }
        }
        
        if !service.environment.is_empty() {
            result.push_str("    environment:\n");
            for (key, value) in &service.environment {
                result.push_str(&format!("      {}: {}\n", key.to_uppercase(), value));
            }
        }
        
        if !service.volumes.is_empty() {
            result.push_str("    volumes:\n");
            for volume in &service.volumes {
                result.push_str(&format!("      - {}\n", volume));
                // Извлекаем имя volume для секции volumes в конце файла
                let vol_name = volume.split(':').next().unwrap_or(volume);
                volumes_section.push_str(&format!("\n  {}:", vol_name));
            }
        }
        if !service.depends_on.is_empty() {
            result.push_str("   depends_on:\n");
            for dep in &service.depends_on {
                result.push_str(&format!("   - {}\n", dep.to_lowercase()))
            }
        }
        
        result.push('\n');
    }

    // Секция volumes в конце файла
    if !volumes_section.is_empty() {
        result.push_str("volumes:");
        result.push_str(&volumes_section);
        result.push('\n');
    }

    result
}

fn dockerignore_content(lang: &str) -> String {
    let common = ".git\n.gitignore\n.env\n*.md\n";
    let specific = match lang {
        "rust" => "target/\n",
        "python" => "__pycache__/\n.venv/\n*.pyc\n",
        _ => "node_modules/\ndist/\n",
    };
    format!("{}{}", common, specific)
}

fn steps_for_git(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    
    if !context.git_init {
        return steps;
    }
    
    let project_name = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");
    
    // .gitignore с контентом под все языки проекта
    let gitignore = gitignore_content(&context.languages);
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
        on_error: ErrorMode::Skip, // Не критично, если git не установлен
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
    });
    
    steps
}

fn gitignore_content(languages: &[String]) -> String {
    let mut content = String::from(
        "# OS generated files\n\
         .DS_Store\n\
         .DS_Store?\n\
         ._*\n\
         .Spotlight-V100\n\
         .Trashes\n\
         ehthumbs.db\n\
         Thumbs.db\n\
         \n\
         # IDE\n\
         .vscode/\n\
         .idea/\n\
         *.swp\n\
         *.swo\n\
         *~\n\
         \n\
         # Environment\n\
         .env\n\
         .env.local\n\
         .env.*.local\n\
         \n\
         # Logs\n\
         *.log\n\
         logs/\n\
         \n"
    );
    
    for lang in languages {
        match lang.as_str() {
            "rust" => {
                content.push_str("# Rust\n\
                                  target/\n\
                                  **/*.rs.bk\n\
                                  *.pdb\n\
                                  \n");
            }
            "python" => {
                content.push_str("# Python\n\
                                  __pycache__/\n\
                                  *.py[cod]\n\
                                  *$py.class\n\
                                  *.so\n\
                                  .Python\n\
                                  build/\n\
                                  develop-eggs/\n\
                                  dist/\n\
                                  downloads/\n\
                                  eggs/\n\
                                  .eggs/\n\
                                  lib/\n\
                                  lib64/\n\
                                  parts/\n\
                                  sdist/\n\
                                  var/\n\
                                  wheels/\n\
                                  *.egg-info/\n\
                                  .installed.cfg\n\
                                  *.egg\n\
                                  MANIFEST\n\
                                  *.manifest\n\
                                  *.spec\n\
                                  pip-log.txt\n\
                                  pip-delete-this-directory.txt\n\
                                  htmlcov/\n\
                                  .tox/\n\
                                  .nox/\n\
                                  .coverage\n\
                                  .coverage.*\n\
                                  .cache\n\
                                  nosetests.xml\n\
                                  coverage.xml\n\
                                  *.cover\n\
                                  .hypothesis/\n\
                                  .pytest_cache/\n\
                                  *.mo\n\
                                  *.pot\n\
                                  venv/\n\
                                  .venv/\n\
                                  ENV/\n\
                                  env/\n\
                                  \n");
            }
            "typescript" | "javascript" | "node" => {
                content.push_str("# Node\n\
                                  node_modules/\n\
                                  npm-debug.log*\n\
                                  yarn-debug.log*\n\
                                  yarn-error.log*\n\
                                  lerna-debug.log*\n\
                                  .pnpm-debug.log*\n\
                                  report.[0-9]*.[0-9]*.[0-9]*.[0-9]*.json\n\
                                  pids\n\
                                  *.pid\n\
                                  *.seed\n\
                                  *.pid.lock\n\
                                  lib-cov\n\
                                  coverage/\n\
                                  .nyc_output\n\
                                  .grunt\n\
                                  bower_components\n\
                                  .lock-wscript\n\
                                  build/Release\n\
                                  jspm_packages/\n\
                                  typings/\n\
                                  .npm\n\
                                  .eslintcache\n\
                                  .node_repl_history\n\
                                  *.tgz\n\
                                  .yarn-integrity\n\
                                  .next/\n\
                                  .nuxt/\n\
                                  dist/\n\
                                  \n");
            }
            "go" => {
                content.push_str("# Go\n\
                                  *.exe\n\
                                  *.exe~\n\
                                  *.dll\n\
                                  *.so\n\
                                  *.dylib\n\
                                  *.test\n\
                                  *.out\n\
                                  go.work\n\
                                  \n");
            }
            "java" => {
                content.push_str("# Java\n\
                                  *.class\n\
                                  *.jar\n\
                                  *.war\n\
                                  *.nar\n\
                                  *.ear\n\
                                  *.zip\n\
                                  *.tar.gz\n\
                                  *.rar\n\
                                  hs_err_pid*\n\
                                  .gradle/\n\
                                  build/\n\
                                  target/\n\
                                  \n");
            }
            "csharp" => {
                content.push_str("# .NET\n\
                                  bin/\n\
                                  obj/\n\
                                  *.user\n\
                                  *.suo\n\
                                  *.cache\n\
                                  *.docstates\n\
                                  packages/\n\
                                  \n");
            }
            "cpp" | "c" => {
                content.push_str("# C/C++\n\
                                  *.o\n\
                                  *.obj\n\
                                  *.exe\n\
                                  *.out\n\
                                  *.app\n\
                                  *.a\n\
                                  *.so\n\
                                  *.dylib\n\
                                  \n");
            }
            "php" => {
                content.push_str("# PHP\n\
                                  vendor/\n\
                                  composer.lock\n\
                                  \n");
            }
            "zig" => {
                content.push_str("# Zig\n\
                                  zig-out/\n\
                                  zig-cache/\n\
                                  \n");
            }
            "swift" => {
                content.push_str("# Swift\n\
                                  .build/\n\
                                  DerivedData/\n\
                                  *.xcodeproj\n\
                                  *.xcworkspace\n\
                                  \n");
            }
            "kotlin" => {
                content.push_str("# Kotlin\n\
                                  .gradle/\n\
                                  build/\n\
                                  .idea/\n\
                                  *.iml\n\
                                  out/\n\
                                  local.properties\n\
                                  \n");
            }
            "elixir" => {
                content.push_str("# Elixir\n\
                                  _build/\n\
                                  deps/\n\
                                  .elixir_ls/\n\
                                  \n");
            }
            "dart" => {
                content.push_str("# Dart\n\
                                  .dart_tool/\n\
                                  .packages\n\
                                  build/\n\
                                  pubspec.lock\n\
                                  \n");
            }
            "gleam" => {
                content.push_str("# Gleam\n\
                                  build/\n\
                                  \n");
            }
            _ => {}
        }
    }
    
    content
}

fn steps_for_ci(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    
    if !context.ci {
        return steps;
    }
    
    let project_name = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");
    
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
    
    let ci_content = generate_ci_content(primary_lang, primary_fw, project_name);
    
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

fn generate_ci_content(lang: &str, framework: Option<&str>, project_name: &str) -> String {
    match lang {
        "rust" => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
      - name: Cache dependencies
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{{{ runner.os }}}}-cargo-${{{{ hashFiles('**/Cargo.lock') }}}}
      - name: Run tests
        run: cargo test --verbose
      - name: Build
        run: cargo build --release
"#),
        
        "python" => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.13'
      - name: Install dependencies
        run: |
          python -m pip install --upgrade pip
          pip install -r requirements.txt
      - name: Lint with ruff
        run: |
          pip install ruff
          ruff check .
      - name: Test with pytest
        run: |
          pip install pytest
          pytest
"#),
        
        "typescript" | "javascript" | "node" => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Use Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '22'
          cache: 'npm'
      - name: Install dependencies
        run: npm ci
      - name: Run tests
        run: npm test
      - name: Build
        run: npm run build --if-present
"#),
        
        "go" => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Go
        uses: actions/setup-go@v5
        with:
          go-version: '1.24'
      - name: Test
        run: go test ./...
      - name: Build
        run: go build -v ./...
"#),
        
        "java" => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up JDK 21
        uses: actions/setup-java@v4
        with:
          java-version: '21'
          distribution: 'temurin'
      - name: Setup Gradle
        uses: gradle/gradle-build-action@v3
      - name: Run tests
        run: ./gradlew test
      - name: Build
        run: ./gradlew build -x test
"#),
        
        _ => format!(r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run build script
        run: echo "Add build steps here for {lang} project"
"#),
    }
}

fn steps_for_readme(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let project_name = Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");
    
    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    let project_type = context.project_type.as_deref().unwrap_or("project");
    
    let readme = generate_readme(project_name, primary_lang, primary_fw, project_type, &context.tools);
    
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

fn generate_readme(name: &str, lang: &str, framework: Option<&str>, project_type: &str, tools: &[String]) -> String {
    let fw_str = framework.map(|f| format!(" with {}", f)).unwrap_or_default();
    let tools_str = if tools.is_empty() {
        String::new()
    } else {
        format!("\n\n## Tools & Services\n\n{}", tools.iter()
            .map(|t| format!("- {}", t))
            .collect::<Vec<_>>()
            .join("\n"))
    };
    
    format!(r#"# {name}

A {project_type} built on {lang}{fw_str}.

## Getting Started

### Prerequisites

- {lang} installed on your system

### Installation

```bash
# Clone the repository
git clone <repo-url>
cd {name}

# Install dependencies
# (instructions depend on the language)
```bash

### Running the application
```bash
# Run the application
# (add specific instructions here)
```bash

###Project Structure
```text
{name}/
├── src/           # Source code
├── tests/         # Test files
└── README.md      # This file
```text

{tools_str}


***Made with StackPilot***"#)

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
        content: generate_vscode_settings(primary_lang),
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
        content: generate_vscode_extensions(primary_lang),
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    });
    
    steps
}

fn generate_vscode_settings(lang: &str) -> String {
    match lang {
        "rust" => r#"{
    "rust-analyzer.checkOnSave.command": "clippy",
    "[rust]": {
        "editor.formatOnSave": true
    }
}"#.to_string(),
        
        "python" => r#"{
    "python.defaultInterpreterPath": "${workspaceFolder}/.venv/bin/python",
    "python.analysis.typeCheckingMode": "basic",
    "[python]": {
        "editor.formatOnSave": true,
        "editor.defaultFormatter": "charliermarsh.ruff",
        "editor.codeActionsOnSave": {
            "source.organizeImports": "explicit"
        }
    }
}"#.to_string(),
        
        "typescript" | "javascript" | "node" => r#"{
    "typescript.tsdk": "node_modules/typescript/lib",
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "esbenp.prettier-vscode",
    "[typescript]": {
        "editor.defaultFormatter": "esbenp.prettier-vscode"
    }
}"#.to_string(),
        
        "go" => r#"{
    "go.useLanguageServer": true,
    "go.lintTool": "golangci-lint",
    "go.formatTool": "goimports",
    "[go]": {
        "editor.formatOnSave": true,
        "editor.codeActionsOnSave": {
            "source.organizeImports": "explicit"
        }
    }
}"#.to_string(),
        
        _ => r#"{
    "editor.formatOnSave": true
}"#.to_string(),
    }
}

fn generate_vscode_extensions(lang: &str) -> String {
    match lang {
        "rust" => r#"{
    "recommendations": [
        "rust-lang.rust-analyzer",
        "tamasfe.even-better-toml"
    ]
}"#.to_string(),
        
        "python" => r#"{
    "recommendations": [
        "ms-python.python",
        "charliermarsh.ruff",
        "ms-python.mypy-type-checker"
    ]
}"#.to_string(),
        
        "typescript" | "javascript" | "node" => r#"{
    "recommendations": [
        "dbaeumer.vscode-eslint",
        "esbenp.prettier-vscode"
    ]
}"#.to_string(),
        
        "go" => r#"{
    "recommendations": [
        "golang.go"
    ]
}"#.to_string(),
        
        "java" => r#"{
    "recommendations": [
        "vscjava.vscode-java-pack"
    ]
}"#.to_string(),
        
        "csharp" => r#"{
    "recommendations": [
        "ms-dotnettools.csharp"
    ]
}"#.to_string(),
        
        _ => r#"{
    "recommendations": []
}"#.to_string(),
    }
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
