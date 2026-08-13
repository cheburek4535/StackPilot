pub mod content;
pub mod executor;
pub mod template;
use std::collections::HashMap;
use std::path::{Path};
use std::sync::OnceLock;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use chrono::Local;

use crate::modules::project_creator::generators::GeneratorRegistry;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
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
    pub executor: Arc<executor::StepExecutor>,
}

impl DefaultRecipeEngine {
    pub fn new() -> Self {
        // Единый реестр встроенных генераторов (spring-boot, fs-cleanup,
        // cli) — тот же, что получает ProjectCreatorState.
        let generators = Arc::new(GeneratorRegistry::with_defaults());
        Self {
            executor: Arc::new(executor::StepExecutor::with_generators(generators)),
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
                    self.executor.run_generate(step, &plan, &tx, i).await
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

/// Составить рецепт на основе WizardContext (TZ Task 1).
///
/// Шаги складываются в СТРОГУЮ очередь фаз (см. также `ExecutionPhase`):
///   1. Root Scaffolding — CLI фреймворков со scaffold="root" (tauri,
///      django, spring-boot, nest) запускаются ПЕРВЫМИ в корне проекта.
///      Движок не создаёт для них backend//frontend/ — корнем владеет CLI.
///   2. Subdir Scaffolding — сегменты моно-репозитория, language-скаффолды
///      (cargo init, package.json...) и CLI фреймворков со scaffold="subdir"
///      (create-vite, create-next-app, ...). Если корнем владеет root-скаффолд,
///      компаньоны (nextjs при nest) получают собственный сегмент frontend/
///      и выполняются ВНУТРИ него с аргументом "." — иначе CLI создаёт
///      вложенную папку <project_name>/ (матрешка testapp2/testapp2).
///   3. Установка инструментов — prisma init, alembic init, docker-compose
///      пишутся ПОСЛЕ каркасов (prisma требует package.json).
///   4. git init — чистим вложенные .git от генераторов и создаём корневой
///      репозиторий ДО шаблонизации.
///   5. ФИНАЛЬНАЯ шаблонизация — README.md, docker-compose.yaml, .gitignore
///      рендерятся ПОСЛЕ всех CLI-фреймворков и перезаписывают их версии
///      (overwrite=true), иначе create-next-app/nest new затирают шаблон.
///   6. Финализация — единственная установка зависимостей (npm install
///      ровно один раз на каждый JS-каталог, в самом конце) и стартовый
///      git add/commit со всеми готовыми файлами.
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

    // Фаза 1: Root Scaffolding. Root-фреймворки выполняются до ВСЕГО
    // остального в корне проекта (см. SegLayout::compute — присутствие
    // хотя бы одного scaffold="root" отключает сегментацию backend/frontend).
    let layout = SegLayout::compute(context);
    let root_present = context.frameworks.iter().any(|fw| {
        framework_def(fw).is_some_and(|def| def.scaffold.as_deref() == Some("root"))
    });
    let mut rest_frameworks: Vec<String> = Vec::new();
    // Каталоги, в которых после всех CLI-каркасов нужен РОВНО ОДИН npm install
    // ("." = корень проекта). Скаффолдеры запускаются с --skip-install/
    // --no-install, поэтому node_modules не плодятся на каждом шаге.
    let mut js_dirs: Vec<String> = Vec::new();
    for fw in &context.frameworks {
        if framework_def(fw).is_some_and(|def| def.scaffold.as_deref() == Some("root")) {
            steps.extend(steps_for_framework(fw, project_path, project_name, context, None, false));
            // Root-JS-фреймворк (nest): работает в корне с --skip-install,
            // его package.json ставится один раз в финальной фазе.
            if is_js_framework(fw) {
                push_unique(&mut js_dirs, ".".to_string());
            }
            // tauri: фронтенд живёт в frontend/ (компаньон react/vue/svelte
            // или vite-vanilla без компаньона) — npm install выполняется там.
            if fw == "tauri" {
                push_unique(&mut js_dirs, "frontend".to_string());
            }
        } else {
            rest_frameworks.push(fw.clone());
        }
    }

    // Сегмент для фреймворка рядом с root-скаффолдом: корень уже занят
    // (nest, django, spring-boot...), поэтому остальным фреймворкам
    // назначается каталог по их side (frontend → frontend/). Без этого
    // create-next-app создал бы вложенную папку <project_name>/ прямо в
    // корне — матрёшка testapp2/testapp2.
    let root_rest_seg = |fw: &str| -> Option<String> {
        if !root_present {
            return None;
        }
        match framework_def(fw).map(|def| def.side.as_str()) {
            Some("backend") => Some("backend".to_string()),
            Some("frontend") => Some("frontend".to_string()),
            _ => None,
        }
    };

    // Фаза 2: Subdir Scaffolding. Сегменты моно-репозитория (backend + frontend)
    // создаются только когда root-фреймворков нет; генераторы подпапок сами
    // создают свои каталоги (create-next-app frontend и т.п.). При
    // root-скаффолде папки сегментов создаёт движок — CLI компаньона будет
    // работать ВНУТРИ них с аргументом ".".
    let mut seg_dirs: Vec<String> = Vec::new();
    if root_present {
        for fw in &rest_frameworks {
            // Компаньон, которого root-фреймворк скаффолдит сам
            // (tauri → react-ts), сегмента не получает.
            if root_scaffold_consumes_companion(fw, context) {
                continue;
            }
            if let Some(seg) = root_rest_seg(fw) {
                push_unique(&mut seg_dirs, seg);
            }
        }
    } else {
        if let Some(dir) = &layout.backend {
            seg_dirs.push(dir.clone());
        }
        if let Some(dir) = &layout.frontend {
            seg_dirs.push(dir.clone());
        }
    }
    for dir in &seg_dirs {
        steps.push(Step::CreateDirectory {
            id: format!("create_{}_dir", dir),
            label: format!("Create {}/", dir),
            description: format!("Create {} directory for frameworks", dir),
            path: dir.clone(),
            condition: None,
            on_error: ErrorMode::Abort,
        });
    }

    for lang in &context.languages {
        // Фреймворк сам создаёт каркас для этого языка (aspnetcore вместо
        // dotnet new console, nextjs вместо js-скаффолда, tauri вместо
        // cargo init) — generic-шаги языка не нужны и конфликтуют с
        // файлами фреймворка.
        if language_scaffold_suppressed(lang, context) {
            continue;
        }
        let lang_seg = if root_present {
            match language_side_infer(lang) {
                Some("backend") => seg_dirs.iter().find(|d| *d == "backend").cloned(),
                Some("frontend") => seg_dirs.iter().find(|d| *d == "frontend").cloned(),
                _ => None,
            }
        } else {
            layout.for_language(lang)
        };
        let mut lang_steps = steps_for_language(lang, project_name, project_path);
        if let Some(dir) = &lang_seg {
            lang_steps = into_segment(lang_steps, dir, root_present);
        }
        steps.extend(lang_steps);
        // JS-язык без фреймворка-каркаса: package.json ляжет в этот каталог —
        // там нужен финальный npm install.
        if matches!(lang.to_lowercase().as_str(), "typescript" | "javascript") {
            push_unique(&mut js_dirs, lang_seg.clone().unwrap_or_else(|| ".".to_string()));
        }
    }

    for fw in rest_frameworks {
        let seg = if root_present {
            root_rest_seg(&fw)
        } else {
            layout.for_framework(&fw)
        };
        let fw_steps = steps_for_framework(&fw, project_path, project_name, context, seg.as_deref(), root_present);
        // Поглощённый root-фреймворком компаньон (nest → react) не создаёт
        // своих файлов — npm install для него не нужен.
        if is_js_framework(&fw) && !fw_steps.is_empty() {
            // Scaffold-фреймворки кладут package.json в scaffold_target_dir
            // (frontend/), остальные — в сегмент или подпапку <project_name>.
            let dir = if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw.as_str()) {
                scaffold_target_dir(&fw, seg.as_deref())
            } else {
                seg.clone().unwrap_or_else(|| project_name.to_string())
            };
            push_unique(&mut js_dirs, dir);
        }
        steps.extend(fw_steps);
    }

    // Фаза 3: установка инструментов (prisma init требует существующий
    // package.json — выполняется строго после каркасов).
    steps.extend(steps_for_tools(context, project_path));

    // Фаза 4: git init — ДО шаблонизации: убираем вложенные .git, созданные
    // генераторами, и инициализируем корневой репозиторий.
    steps.extend(steps_for_git_init(context, project_path));

    // Фаза 5: ФИНАЛЬНАЯ шаблонизация — ПОСЛЕ выполнения ВСЕХ CLI-фреймворков.
    // README.md, docker-compose.yaml и .gitignore, созданные самими CLI
    // (create-next-app, nest new...), перезаписываются нашими шаблонами
    // (overwrite=true) — иначе шаблон молча теряется.
    steps.extend(steps_for_docker(context, project_path, project_name));
    steps.extend(steps_for_gitignore(context, project_path));
    steps.extend(steps_for_ci(context, project_path, project_name));
    steps.extend(steps_for_readme(context, project_path, project_name));
    steps.extend(steps_for_vscode(context));

    // Фаза 6: финализация — единственная установка зависимостей в самом
    // конце (npm install ровно один раз на JS-каталог) и стартовый
    // git add/commit со всеми готовыми файлами.
    steps.extend(steps_for_finalize(context, &js_dirs, project_path, project_name));

    Ok(Recipe {
        id: format!("recipe_{}", project_name),
        name: format!("{:?} project", context.project_type),
        description: format!("Full setup for {} project", project_name),
        tags: context.languages.clone(),
        steps,
    })
}

/// Добавить значение в список, если его там ещё нет.
fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// JS/TS-фреймворки: создают package.json и нуждаются в npm install.
fn is_js_framework(fw: &str) -> bool {
    framework_def(fw).is_some_and(|def| {
        def.languages
            .iter()
            .any(|l| l == "typescript" || l == "javascript")
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
                "Initialize Zig project",
                "zig init"),
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
fn _is_frontend_lang(l: &str) -> bool {
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
    // tauri: фронтенд (JS/TS) живёт в frontend/ и создаётся сам —
    // компаньоном react/vue/svelte или vite-vanilla. Generic-скаффолд
    // JS/TS (package.json, src/, tsc --init) в корне не нужен: он либо
    // конфликтует с каркасом tauri, либо пишет мусорные заглушки.
    if matches!(lang, "typescript" | "javascript")
        && context.frameworks.iter().any(|f| f == "tauri")
    {
        return true;
    }
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
        // Root-scaffold фреймворки (tauri, django, spring-boot, nest) создают
        // проект ПРЯМО В КОРНЕ: их CLI (create-tauri-app, django-admin
        // startproject ., Spring Initializr, nest new .) не умеет работать
        // «внутри» предварительно созданных backend//frontend/ сегментов —
        // они либо падают, либо тащат каркас в корень. Сегментация
        // отключается целиком: никаких eager-папок frontend/backend.
        if context.frameworks.iter().any(|fw| {
            framework_def(fw).is_some_and(|def| def.scaffold.as_deref() == Some("root"))
        }) {
            return SegLayout {
                frontend: None,
                backend: None,
                lang_side: HashMap::new(),
            };
        }

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
/// (flutter create <name>, mix phx.new <name> и т.п.).
/// Индекс — позиция имени создаваемой подпапки в args.
/// При сегментации в такой команде подменяется имя создаваемой подпапки
/// (индекс в args), а рабочая директория остаётся корнем проекта;
/// остальные шаги (write_file, остальные команды) переносятся в сегмент.
/// Фронтенд-скаффолдеры (create-vite, create-next-app, nuxi init, expo,
/// laravel/symfony через composer) из этого списка УБРАНЫ — их каталогом
/// управляет ScaffoldGenerator (Step::Generate "scaffold", см. генератор).
const FOLDER_MAKER_STEPS: &[(&str, usize)] = &[
    ("solid_init", 1),      // npx create-solid <имя>
    ("flutter_create", 1),  // flutter create <имя>
    ("electron_init", 1),   // npx create-electron-app <имя>
    ("rn_init", 2),         // npx @react-native-community/cli init <имя>
    ("plasmo_init", 2),     // npx plasmo init <имя>
    ("phoenix_new", 1),     // mix phx.new <имя>
    ("vapor_new", 1),       // vapor new <имя>
];

/// CLI-генераторы подпапок, которые УМЕЮТ работать в текущей папке с
/// аргументом "." (flutter create ., create-solid . и т.п.).
/// Когда корнем владеет root-скаффолд (nest + solidjs), такие команды
/// выполняются ВНУТРИ уже созданной движком папки сегмента с "." — вместо
/// создания вложенной <project_name>/ (матрешка testapp2/testapp2).
/// Остальные (RN CLI, electron-forge...) оставляем в режиме переименования
/// папки: их CLI не гарантирует работу в текущей директории.
const FOLDER_MAKER_DOT_CAPABLE: &[&str] = &[
    "solid_init",
    "flutter_create",
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
/// префикс. Поведение генераторов подпапок зависит от `cli_inplace`:
///   - cli_inplace == false (обычная сегментация): имя создаваемой подпапки
///     меняется на имя сегмента (create-next-app frontend из корня);
///   - cli_inplace == true (рядом с root-скаффолдом, корень занят): папку
///     сегмента уже создал движок, CLI выполняется ВНУТРИ неё с ".".
fn into_segment(steps: Vec<Step>, dir: &str, cli_inplace: bool) -> Vec<Step> {
    steps.into_iter().map(|step| {
        let folder_maker_index = FOLDER_MAKER_STEPS.iter()
            .find(|(sid, _)| matches!(&step, Step::Command { id, .. } if *sid == id.as_str()))
            .map(|(_, idx)| *idx);
        let replaces = folder_maker_index.is_some();
        match step {
            Step::Command { id, label, description, command, mut args, working_dir, env, timeout_secs, condition, on_error, interactive } => {
                if replaces {
                    if let Some(idx) = folder_maker_index {
                        if let Some(arg) = args.get_mut(idx) {
                            if cli_inplace && FOLDER_MAKER_DOT_CAPABLE.contains(&id.as_str()) {
                                // Root-скаффолд владеет корнем: создаём проект
                                // в ТЕКУЩЕЙ папке сегмента, а не вложенную
                                // папку с именем проекта.
                                *arg = ".".to_string();
                            } else {
                                *arg = dir.to_string();
                            }
                        }
                    }
                    if cli_inplace && FOLDER_MAKER_DOT_CAPABLE.contains(&id.as_str()) {
                        // CLI работает внутри папки сегмента
                        Step::Command { id, label, description, command, args, working_dir: Some(dir.to_string()), env, timeout_secs, condition, on_error, interactive }
                    } else {
                        // генератор сам создаст подпапку — рабочая директория остаётся корневой
                        Step::Command { id, label, description, command, args, working_dir, env, timeout_secs, condition, on_error, interactive }
                    }
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
            // Scaffold-генератор (Step::Generate "scaffold") сам кладёт проект
            // в target_dir: при сегментации каталогом становится сегмент.
            Step::Generate { id, label, description, generator_id, mut generator_config, condition, on_error } => {
                if generator_id == "scaffold" {
                    generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                }
                Step::Generate { id, label, description, generator_id, generator_config, condition, on_error }
            }
            other => other,
        }
    }).collect()
}

/// Скаффолдеры, которые генерируют package.json и называют его по имени
/// папки (frontend/, <project_name>/) вместо project_name из WizardContext.
/// Для них движок добавляет пост-шаг, переписывающий поле name
/// (см. package_name_patch_step) — чинит баг «frontend/package.json
/// называется frontend».
const PACKAGE_JSON_SCAFFOLDS: &[&str] = &[
    "react", "vue", "svelte", "nextjs", "sveltekit", "nuxt", "solidjs",
    "electron", "expo", "react-native", "plasmo", "tauri", "nest",
];

/// Фреймворки, чей каркас создаёт ScaffoldGenerator (Step::Generate
/// "scaffold", см. generators/mod.rs): CLI-генератор вызывается с "." внутри
/// целевого каталога (dot-режим) или во временную папку с программным
/// переносом (temp+move) — без матрёшек testapp/testapp.
/// Для них каталог проекта — scaffold_target_dir(...), а не подпапка
/// <project_name>/ (см. также js_dirs и pkg-name patch).
const SCAFFOLD_GENERATOR_FRAMEWORKS: &[&str] = &[
    "react", "vue", "svelte", "nextjs", "sveltekit", "nuxt", "expo", "laravel", "symfony",
];

/// Каталог, куда ScaffoldGenerator кладёт проект: при сегментации — каталог
/// сегмента; в монолите — по output_subdir фреймворка (frontend → "frontend",
/// backend-фреймворки (laravel/symfony) → корень ".").
fn scaffold_target_dir(fw: &str, seg: Option<&str>) -> String {
    if let Some(dir) = seg {
        return dir.to_string();
    }
    match framework_def(fw).and_then(|def| def.output_subdir.as_deref()) {
        Some("frontend") => "frontend".to_string(),
        _ => ".".to_string(),
    }
}

/// Пост-шаг после CLI-скаффолдинга: переписывает ТОЛЬКО поле name в
/// package.json (node -e сохраняет форматирование и остальные поля).
/// Исправляет баг шаблонизатора: скаффолдер называет проект по имени
/// родительской папки (frontend/) вместо project_name из WizardContext.
fn package_name_patch_step(id: &str, label: &str, workdir: Option<&str>, project_name: &str) -> Step {
    // Апостроф в имени проекта ломает JS-строку — экранируем.
    let safe_name = project_name.replace('\'', "\\'");
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: "Set package.json name to the real project name".into(),
        command: "node".into(),
        args: vec![
            "-e".into(),
            format!(
                "const fs=require('fs');const p='package.json';const j=JSON.parse(fs.readFileSync(p,'utf8'));j.name='{}';fs.writeFileSync(p,JSON.stringify(j,null,2)+'\\n')",
                safe_name
            ),
        ],
        working_dir: workdir.map(String::from),
        env: None,
        timeout_secs: Some(30),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    }
}

/// Root-фреймворк (scaffold="root") со своим CLI скаффолдит своих
/// frontend-компаньонов сам (create-tauri-app → react-ts).
/// Шаги такого компаньона подавляются: второй фронтенд (лишняя vite-папка)
/// поверх каркаса root-фреймворка не нужен.
///
/// Исключение — tauri: с новым пайплайном (frontend/ + tauri init --ci)
/// компаньон react/vue/svelte скаффолдится ОТДЕЛЬНО в frontend/ и не
/// подавляется.
fn root_scaffold_consumes_companion(fw: &str, context: &WizardContext) -> bool {
    context.frameworks.iter().any(|root| {
        root != "tauri"
            && framework_def(root).is_some_and(|def| {
                def.scaffold.as_deref() == Some("root") && def.companions.iter().any(|c| c == fw)
            })
    })
}

fn steps_for_framework(fw: &str, project_path: &str, project_name: &str, context: &WizardContext, seg: Option<&str>, cli_inplace: bool) -> Vec<Step> {
    // Root-фреймворк со своим CLI сам скаффолдит фронтенд-компаньона —
    // отдельные шаги компаньона не нужны (см. root_scaffold_consumes_companion).
    if root_scaffold_consumes_companion(fw, context) {
        return Vec::new();
    }

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

    let mut steps = match seg {
        Some(dir) => into_segment(steps, dir, cli_inplace),
        None => steps,
    };

    // Scaffold-генераторы: into_segment не трогает Generate-шаги, поэтому
    // целевой каталог выставляется здесь — по сегменту или output_subdir.
    if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw) {
        let target_dir = scaffold_target_dir(fw, seg);
        for step in &mut steps {
            if let Step::Generate { generator_id, generator_config, .. } = step {
                if generator_id == "scaffold" {
                    generator_config["target_dir"] =
                        serde_json::Value::String(target_dir.clone());
                }
            }
        }
    }

    // Баг шаблонизатора: скаффолдеры (create-vite, create-tauri-app,
    // create-next-app...) называют package.json по имени папки, в которую
    // пишут (frontend/ или корень), а не по project_name из WizardContext.
    // Пост-шаг примешивается ПОСЛЕ сегментации — его рабочая директория
    // должна указывать на фактическое место package.json.
    if let Some(def) = framework_def(fw) {
        if def.scaffold.is_some() && PACKAGE_JSON_SCAFFOLDS.contains(&fw) {
            let workdir: Option<String> = if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw) {
                // package.json лежит в каталоге, куда скаффолдер положил проект
                Some(scaffold_target_dir(fw, seg))
            } else if fw == "tauri" {
                // Новый пайплайн tauri: фронтенд живёт в frontend/ — package.json там
                Some("frontend".to_string())
            } else {
                match def.scaffold.as_deref() {
                    // root-скаффолдеры (nest) создают package.json в корне проекта
                    Some("root") => None,
                    // subdir-скаффолдеры — внутри созданной подпапки (сегмент
                    // frontend/ в моно-репозитории или <project_name> в монолите)
                    _ => Some(seg.map(String::from).unwrap_or_else(|| project_name.to_string())),
                }
            };
            steps.push(package_name_patch_step(
                &format!("{}_pkg_name", fw),
                &format!("Fix package.json name for {}", fw),
                workdir.as_deref(),
                project_name,
            ));
        }
    }

    steps
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
    let root_present = context.frameworks.iter().any(|fw| {
        framework_def(fw).is_some_and(|def| def.scaffold.as_deref() == Some("root"))
    });
    let mut by_path: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for fw in &context.frameworks {
        // Тот же путь сегмента, что в compose_recipe: при root-скаффолде
        // компаньоны получают каталог по своему side (frontend/backend),
        // иначе — обычный SegLayout.
        let seg = if root_present {
            match framework_def(fw).map(|def| def.side.as_str()) {
                Some("backend") => Some("backend".to_string()),
                Some("frontend") => Some("frontend".to_string()),
                _ => None,
            }
        } else {
            layout.for_framework(fw)
        };
        let steps = steps_for_framework(fw, ".", &project_name, context, seg.as_deref(), root_present);
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

// ============================================================================
// Qt: генерация по UI-режиму (QML / Widgets / WebEngine / Kirigami).
// Режим выбирается в мастере (answers["qt_ui"] = ["qt-qml"] и т.п.) —
// он решает, какие модули Qt подключить и какой main.cpp сгенерировать.
// ============================================================================

fn qt_ui_mode(context: &WizardContext) -> &'static str {
    if let Some(modes) = context.answers.get("qt_ui") {
        if let Some(m) = modes.first() {
            return match m.as_str() {
                "qt-qml" => "qml",
                "qt-webengine" => "webengine",
                "qt-kirigami" => "kirigami",
                _ => "widgets",
            };
        }
    }
    // Ретро-совместимость: стек без ответов мастера (пресеты, старые сессии)
    if context.frameworks.iter().any(|f| f == "qt-qml") {
        return "qml";
    }
    if context.frameworks.iter().any(|f| f == "qt-webengine") {
        return "webengine";
    }
    if context.frameworks.iter().any(|f| f == "qt-kirigami") {
        return "kirigami";
    }
    "widgets"
}

fn qt_web_framework_label(context: &WizardContext) -> &'static str {
    if let Some(fws) = context.answers.get("qt_web_framework") {
        if let Some(f) = fws.first() {
            return match f.as_str() {
                "react" => "React",
                "vue" => "Vue",
                "svelte" => "Svelte",
                _ => "web UI",
            };
        }
    }
    if context.frameworks.iter().any(|f| f == "react") {
        "React"
    } else if context.frameworks.iter().any(|f| f == "vue") {
        "Vue"
    } else if context.frameworks.iter().any(|f| f == "svelte") {
        "Svelte"
    } else {
        "web UI"
    }
}

fn qt_step_write(id: &str, label: &str, path: &str, content: String) -> Step {
    Step::WriteFile {
        id: id.to_string(),
        label: label.to_string(),
        description: format!("Create {}", path),
        path: path.to_string(),
        content,
        overwrite: false,
        condition: None,
        on_error: ErrorMode::Skip,
    }
}

fn qt_step_note(id: &str, label: &str, text: String) -> Step {
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: text.clone(),
        command: "echo".into(),
        args: vec![text],
        working_dir: None,
        env: None,
        timeout_secs: Some(60),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    }
}

fn qt_steps_widgets(project_name: &str) -> Vec<Step> {
    vec![
        qt_step_write("qt_main", "Create Qt main", "src/main.cpp", format!(
            r#"#include <QApplication>
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
"#,
            project_name, project_name
        )),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", format!(
            r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

add_executable({p} src/main.cpp)
target_link_libraries({p} Qt6::Widgets)
"#,
            p = project_name
        )),
    ]
}

fn qt_steps_qml(project_name: &str) -> Vec<Step> {
    let main_cpp = r#"#include <QGuiApplication>
#include <QQmlApplicationEngine>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QQmlApplicationEngine engine;
    const QUrl url(QStringLiteral("qrc:/main.qml"));
    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreated,
        &app, [url](QObject *obj, const QUrl &objUrl) {
            if (!obj && url == objUrl)
                QCoreApplication::exit(-1);
        },
        Qt::QueuedConnection);
    engine.load(url);
    return app.exec();
}
"#;
    let main_qml = format!(
        r#"import QtQuick

Window {{
    width: 480
    height: 320
    visible: true
    title: "{}"

    Text {{
        anchors.centerIn: parent
        text: "Hello from {}!"
        font.pixelSize: 24
    }}
}}
"#,
        project_name, project_name
    );
    vec![
        qt_step_write("qt_main", "Create Qt main", "src/main.cpp", main_cpp.into()),
        qt_step_write("qt_qml_main", "Create QML view", "src/main.qml", main_qml),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", format!(
            r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Quick)

add_executable({p} src/main.cpp)

qt_add_resources({p} "qml"
    PREFIX "/"
    FILES src/main.qml
)

target_link_libraries({p} Qt6::Quick)
"#,
            p = project_name
        )),
    ]
}

fn qt_steps_kirigami(project_name: &str) -> Vec<Step> {
    let main_cpp = r#"#include <QGuiApplication>
#include <QQmlApplicationEngine>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QQmlApplicationEngine engine;
    const QUrl url(QStringLiteral("qrc:/main.qml"));
    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreated,
        &app, [url](QObject *obj, const QUrl &objUrl) {
            if (!obj && url == objUrl)
                QCoreApplication::exit(-1);
        },
        Qt::QueuedConnection);
    engine.load(url);
    return app.exec();
}
"#;
    let main_qml = format!(
        r#"import QtQuick
import org.kde.kirigami 2.20 as Kirigami

Kirigami.ApplicationWindow {{
    width: 600
    height: 450
    title: "{}"

    pageStack.initialPage: Kirigami.Page {{
        Kirigami.Heading {{
            text: "Hello from {}!"
        }}
    }}
}}
"#,
        project_name, project_name
    );
    vec![
        qt_step_write("qt_main", "Create Qt main", "src/main.cpp", main_cpp.into()),
        qt_step_write("qt_kirigami_main", "Create Kirigami view", "src/main.qml", main_qml),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", format!(
            r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Quick)
find_package(KF6 REQUIRED COMPONENTS Kirigami)

add_executable({p} src/main.cpp)

qt_add_resources({p} "qml"
    PREFIX "/"
    FILES src/main.qml
)

target_link_libraries({p} Qt6::Quick KF6::Kirigami)
"#,
            p = project_name
        )),
    ]
}

fn qt_steps_webengine(project_name: &str, context: &WizardContext) -> Vec<Step> {
    let web = qt_web_framework_label(context);
    // Расширенные шаблоны живут в TemplateEngine ({{ project_name }} и т.п.)
    // — см. engine/template.rs: qt_webengine_main_cpp / qt_webengine_cmake.
    let engine = template::TemplateEngine::new();
    let main_cpp = engine.qt_webengine_main_cpp(project_name);
    let cmake_lists = engine.qt_webengine_cmake(project_name);
    vec![
        qt_step_write("qt_main", "Create Qt main (WebEngine)", "src/main.cpp", main_cpp),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", cmake_lists),
        qt_step_note(
            "qt_webengine_hint",
            "Qt WebEngine: build the web part",
            format!(
                "Web UI is {} — build it first (npm run build in the web app folder), then: cmake -S . -B build && cmake --build build",
                web
            ),
        ),
    ]
}

/// Бандл-идентификатор для tauri init (--identifier): домен +
/// санитизированное имя проекта (только [a-zA-Z0-9-._], сегмент не
/// начинается с цифры — иначе CLI отвергает ввод).
fn tauri_identifier(project_name: &str) -> String {
    let mut base: String = project_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if base.is_empty() {
        base.push_str("app");
    }
    if base.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        base.insert(0, 'a');
    }
    format!("com.{}", base)
}

/// Шаг «scaffold»: CLI-генератор, который сам создаёт папку проекта.
/// ScaffoldGenerator разбирается с каталогом сам (см. generators/mod.rs):
/// dot-capable CLI работают с "." внутри target_dir, остальные — временная
/// папка + программный перенос содержимого. Матрёшек testapp/testapp нет.
#[allow(clippy::too_many_arguments)]
fn scaffold_step(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    name_arg: usize,
    target_dir: &str,
    dot_capable: bool,
) -> Step {
    Step::Generate {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        generator_id: "scaffold".into(),
        generator_config: serde_json::json!({
            "command": command,
            "args": args,
            "name_arg": name_arg,
            "target_dir": target_dir,
            "dot_capable": dot_capable,
        }),
        condition: None,
        on_error: ErrorMode::Skip,
    }
}

fn steps_for_framework_impl(fw: &str, project_path: &str, project_name: &str, context: &WizardContext) -> Vec<Step> {
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
            // Новый пайплайн: create-tauri-app НЕ используется — он скаффолдил
            // фронтенд ПО ШАБЛОНУ в корне проекта (структура была пустой без
            // node_modules/.svelte-kit и т.п.). Теперь фронтенд живёт в
            // frontend/ и скаффолдится отдельно (компаньон react/vue/svelte
            // или vite-vanilla без компаньона), а в корне выполняется
            // tauri init --ci + Rust-патч tauri.conf.json (см. генератор
            // "tauri-config") — пути на frontend/ и dev-сервер Vite.
            let identifier = tauri_identifier(project_name);
            let has_companion = context.frameworks.iter().any(|f| {
                framework_def("tauri").is_some_and(|def| def.companions.iter().any(|c| c == f))
            });
            let mut steps: Vec<Step> = Vec::new();
            if !has_companion {
                // Компаньон (react/vue/svelte) уже скаффолдит frontend/ —
                // без него фронтенд создаёт vite (vanilla).
                let template = if has_typescript { "vanilla-ts" } else { "vanilla" };
                steps.push(scaffold_step(
                    "tauri_web_scaffold",
                    "Create frontend for Tauri",
                    "Scaffold Vite frontend in frontend/",
                    "npx",
                    vec!["create-vite@latest", SCAFFOLD_TARGET, "--template", template],
                    1,
                    "frontend",
                    true,
                ));
            }
            // tauri init --ci: неинтерактивно, все пути — на frontend/.
            steps.push(Step::Command {
                id: "tauri_init".into(),
                label: "Initialize Tauri shell".into(),
                description: "Run tauri init (non-interactive, --ci)".into(),
                command: "npx".into(),
                args: vec![
                    "--yes".into(),
                    "@tauri-apps/cli@latest".into(),
                    "init".into(),
                    "--ci".into(),
                    "--app-name".into(),
                    project_name.into(),
                    "--window-title".into(),
                    project_name.into(),
                    "--frontend-dist".into(),
                    "../frontend/dist".into(),
                    "--dev-url".into(),
                    "http://localhost:5173".into(),
                    "--before-dev-command".into(),
                    "npm --prefix frontend run dev".into(),
                    "--before-build-command".into(),
                    "npm --prefix frontend run build".into(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            });
            // Rust-патч tauri.conf.json: пути на frontend/, identifier.
            steps.push(Step::Generate {
                id: "tauri_config_patch".into(),
                label: "Patch Tauri configuration".into(),
                description: "Adapt src-tauri/tauri.conf.json to the frontend/ layout".into(),
                generator_id: "tauri-config".into(),
                generator_config: serde_json::json!({
                    "frontend_dir": "frontend",
                    "dev_url": "http://localhost:5173",
                    "before_dev_command": "npm --prefix frontend run dev",
                    "before_build_command": "npm --prefix frontend run build",
                    "identifier": identifier,
                }),
                condition: None,
                on_error: ErrorMode::Skip,
            });
            steps
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
        // create-vite с --template работает без интерактива; каталог проекта
        // (frontend/) выбирает ScaffoldGenerator (dot-режим с "."); npm install
        // выполняется один раз в финальной фазе (steps_for_finalize).
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
                scaffold_step("vite_create", &format!("Create {fw} app"), "Scaffold Vite project",
                    "npx", vec!["create-vite@latest", SCAFFOLD_TARGET, "--template", template],
                    1, "frontend", true),
            ]
        }

        // ==================== JavaScript / TypeScript ====================
        "nextjs" => {
            // create-next-app с --yes работает без интерактива (--typescript/
            // --javascript фиксируют язык, остальное — флагами). Каталог
            // frontend/ выбирает ScaffoldGenerator (dot-режим с ".").
            let ts_flag = if has_typescript { "--typescript" } else { "--javascript" };
            vec![
                scaffold_step("nextjs_create", "Create Next.js app", "Scaffold Next.js project",
                    "npx", vec!["create-next-app@latest", SCAFFOLD_TARGET, ts_flag, "--tailwind", "--eslint", "--app", "--no-src-dir", "--import-alias", "@/*", "--use-npm", "--skip-install", "--yes"],
                    1, "frontend", true),
            ]
        }

        "sveltekit" => {
            // sv create полностью неинтерактивен с флагами: шаблон minimal,
            // типы фиксируются --types/--no-types, доп. инструменты не ставим.
            let mut sv_args = vec![
                "sv", "create", SCAFFOLD_TARGET,
                "--template", "minimal", "--no-add-ons", "--no-install",
            ];
            if has_typescript {
                sv_args.push("--types");
                sv_args.push("ts");
            } else {
                sv_args.push("--no-types");
            }
            vec![
                scaffold_step("sveltekit_create", "Create SvelteKit app", "Scaffold SvelteKit project",
                    "npx", sv_args, 2, "frontend", true),
            ]
        },

        "nuxt" => {
            // nuxi init неинтерактивен: менеджер пакетов и git фиксируются
            // флагами (--packageManager npm --gitInit false), установка
            // зависимостей откладывается в финальную фазу (--no-install).
            vec![
                scaffold_step("nuxt_create", "Create Nuxt app", "Scaffold Nuxt project",
                    "npx", vec!["--yes", "nuxi@latest", "init", SCAFFOLD_TARGET, "--packageManager", "npm", "--gitInit", "false", "--no-install"],
                    3, "frontend", true),
            ]
        },

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
                    "npx", vec!["@react-native-community/cli", "init", &rn_name, "--skip-install"],
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
            // create-expo-app НЕ принимает "." в качестве имени проекта
            // (dot_capable=false): ScaffoldGenerator скаффолдит во временную
            // папку и программно переносит содержимое в frontend/.
            let expo_template = if has_typescript {
                "blank-typescript"
            } else {
                "blank"
            };
            vec![
                scaffold_step("expo_init", "Init Expo", "Create Expo project",
                    "npx", vec!["create-expo-app", SCAFFOLD_TARGET, "--yes", "--no-install", "--template", expo_template],
                    1, "frontend", false),
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
            // --skip-install: зависимости корня ставятся ОДИН раз в финальной
            // фазе пайплайна (steps_for_finalize), а не сразу в каркасе —
            // иначе node_modules плодятся на каждом шаге.
            // --skip-git: git инициализирует сам движок (steps_for_git_init).
            cmd_i("nest_new", "Create NestJS project", "Scaffold NestJS application",
                "npx", vec!["@nestjs/cli", "new", ".", "--package-manager", "npm", "--skip-install", "--skip-git"],
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
                // Устаревший `go get github.com/spf13/cobra/cobra` больше не
                // работает (пакет разделён): генератор ставится как отдельный
                // бинарь cobra-cli, а проект инициализируется его командой.
                cmd("cobra_install", "Install Cobra CLI", "Install cobra-cli generator",
                    "go", vec!["install", "github.com/spf13/cobra-cli@latest"]),
                cmd("cobra_init", "Init Cobra CLI", "Initialize Cobra CLI project",
                    "cobra-cli", vec!["init"]),
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
            //
            // Вся работа идёт через генератор "spring-boot" (Rust): он
            // проверяет HTTP-ответ Initializr (status + тело ошибки) и
            // останавливает генерацию с реальной причиной («Несовместимые
            // модули» и т.п.), а не падает на распаковке мусорного
            // project.zip (раньше туда писался HTML/JSON ошибки).
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
            vec![Step::Generate {
                id: "spring_init".into(),
                label: "Generate Spring Boot project".into(),
                description: "Download Spring Boot starter from Initializr (validates HTTP response)".into(),
                generator_id: "spring-boot".into(),
                generator_config: serde_json::json!({
                    "project_name": project_name,
                    "dependencies": deps_str,
                }),
                condition: None,
                // HTTP-ошибка Initializr (400 «Несовместимые модули» и т.п.)
                // обязана остановить пайплайн и показать причину в UI.
                on_error: ErrorMode::Abort,
            }]
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

        // ==================== C++ / Qt ====================
        // Qt — фреймворк с собственным UI-стеком: режим (QML/Widgets/
        // WebEngine/Kirigami) выбирается в мастере (answers["qt_ui"]) и
        // определяет, какие модули Qt подключить и какой main.cpp написать.
        "qt" => {
            let mode = qt_ui_mode(context);
            match mode {
                "qml" => qt_steps_qml(project_name),
                "kirigami" => qt_steps_kirigami(project_name),
                "webengine" => qt_steps_webengine(project_name, context),
                _ => qt_steps_widgets(project_name),
            }
        }

        // Варианты UI Qt — генерируются внутри блока "qt" (см. qt_ui_mode);
        // отдельные шаги не нужны, чтобы не дублировать файлы.
        "qt-qml" | "qt-widgets" | "qt-webengine" | "qt-kirigami" => vec![],

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
            // Официальный путь — composer create-project (npm-пакет
            // @laravel/installer падал: «npm error 404 Not Found»). Работает
            // без интерактива: --no-interaction + --prefer-dist.
            scaffold_step("laravel_new", "Create Laravel project",
                "Scaffold Laravel application via composer",
                "composer", vec!["create-project", "laravel/laravel", SCAFFOLD_TARGET, "--no-interaction", "--prefer-dist"],
                2, ".", true),
        ],

        "symfony" => vec![
            // Symfony: composer create-project symfony/skeleton (без
            // интерактива; локальный бинарь symfony не требуется).
            scaffold_step("symfony_new", "Create Symfony project",
                "Scaffold Symfony application via composer",
                "composer", vec!["create-project", "symfony/skeleton", SCAFFOLD_TARGET, "--no-interaction", "--prefer-dist"],
                2, ".", true),
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


/// Каталог сегмента, где живёт python-код проекта: "backend" в
/// моно-репозитории (backend + frontend), "." — корень проекта.
/// Именно рядом с ним лежит requirements.txt и создаётся venv.
fn python_segment_dir(context: &WizardContext) -> String {
    if context.languages.iter().any(|l| l == "python") {
        if let Some(seg) = SegLayout::compute(context).for_language("python") {
            return seg;
        }
    }
    ".".to_string()
}

/// Путь к бинарю внутри venv проекта, ОТНОСИТЕЛЬНО КОРНЯ проекта:
/// `venv\Scripts\<name>.exe` на Windows, `venv/bin/<name>` на unix.
/// В моно-репозитории venv живёт в каталоге python-сегмента
/// (`backend\venv\Scripts\alembic.exe` / `backend/venv/bin/alembic`),
/// а не в корне.
///
/// Правило движка: Python-утилиты (alembic, pip) вызываются ТОЛЬКО через
/// бинарники виртуального окружения — глобальный `alembic` не используется.
fn python_venv_bin(python_dir: &str, name: &str) -> String {
    let dir = if python_dir.is_empty() || python_dir == "." {
        "venv".to_string()
    } else {
        python_dir.trim_end_matches(['/', '\\']).to_string()
    };
    if cfg!(target_os = "windows") {
        format!("{}\\venv\\Scripts\\{}.exe", dir, name)
    } else {
        format!("{}/venv/bin/{}", dir, name)
    }
}

fn steps_for_tools(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let tools = &context.tools;
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

    // Python-инструменты (alembic) требуют установленных зависимостей в
    // ОКРУЖЕНИИ: `alembic init` падает с «command not found», если пакет
    // не установлен в venv. Поэтому для Python-проектов с alembic сначала
    // создаётся venv и выполняется pip install -r requirements.txt (с
    // гарантированным alembic) — строго ДО шагов инструментов (порядок
    // в steps гарантирован: venv-шаги кладутся первыми в этот список).
    let has_python = context.languages.iter().any(|l| l == "python");
    let needs_python_venv = has_python && tools.iter().any(|t| t == "alembic");
    // Каталог python-кода (backend/ в моно-репозитории, иначе корень):
    // venv создаётся ВНУТРИ него, рядом с requirements.txt. Вычисляется
    // вне блока — нужен и шагам alembic ниже.
    let python_dir = python_segment_dir(context);
    if needs_python_venv {
        let venv_path = if python_dir == "." {
            "venv".to_string()
        } else {
            format!("{}/venv", python_dir)
        };
        let req_path = if python_dir == "." {
            "requirements.txt".to_string()
        } else {
            format!("{}/requirements.txt", python_dir)
        };
        steps.push(Step::Command {
            id: "py_venv_create".into(),
            label: "Create Python virtual environment".into(),
            description: format!("Run python -m venv {}", venv_path),
            command: "python".into(),
            args: vec!["-m".into(), "venv".into(), venv_path],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(120),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        });
        steps.push(Step::Command {
            id: "py_pip_install".into(),
            label: "Install Python dependencies".into(),
            description: "Run pip install -r requirements.txt (plus alembic) inside venv".into(),
            // Только бинарь venv; alembic ставится явно, даже если
            // requirements.txt его не содержит (requirements_txt создаётся
            // пустым, а fastapi/flask перезаписывают только своими пакетами).
            command: python_venv_bin(&python_dir, "pip"),
            args: vec!["install".into(), "-r".into(), req_path, "alembic".into()],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(600),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        });
    }

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
                    description: "Initialize Alembic migrations (inside project venv)".into(),
                    // Вызывается строго через бинарь виртуального окружения
                    // (venv\Scripts\alembic.exe / venv/bin/alembic внутри
                    // python-сегмента): py_pip_install (с гарантированным
                    // alembic) отрабатывает ДО этого шага — см. выше.
                    command: python_venv_bin(&python_dir, "alembic"),
                    args: vec!["init".into(), "migrations".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(60),
                    condition: None,
                    // Abort вместо Skip: alembic обязан быть в venv после
                    // py_pip_install, поэтому провал — реальная ошибка,
                    // а не «тихо провалившийся» шаг генерации.
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                });
            }
            "prisma" => {
                // Интерактивный `npx prisma init` спрашивает пакетный
                // менеджер и БД («Next, choose how you want to set up your
                // database») и повисает на вводе. Провайдер передаётся
                // флагом --datasource-provider, npx — с --yes, а CI=1
                // проставляется executor'ом для всех команд.
                //
                // Новые версии Prisma (6.16+) после init разворачивают в
                // проекте каталог AI-навыков (.agents/, .claude/,
                // .windsurf/ + skills-lock.json — десятки тысяч файлов).
                // --no-skills отключает установку.
                let provider = if context.tools.iter().any(|t| t == "postgresql") {
                    "postgresql"
                } else if context.tools.iter().any(|t| t == "mysql") {
                    "mysql"
                } else if context.tools.iter().any(|t| t == "mongodb") {
                    "mongodb"
                } else {
                    "sqlite"
                };
                steps.push(Step::Command {
                    id: "prisma_init".into(),
                    label: "Init Prisma".into(),
                    description: "Initialize Prisma ORM".into(),
                    command: "npx".into(),
                    args: vec![
                        "--yes".into(),
                        "prisma".into(),
                        "init".into(),
                        "--datasource-provider".into(),
                        provider.into(),
                        "--no-skills".into(),
                    ],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(120),
                    condition: None,
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
                // Подстраховка для версий Prisma без флага --no-skills
                // (или если флаг проигнорирован): движок на Rust принудительно
                // удаляет агентные артефакты из корня проекта. Папки
                // удаляются только при наличии маркера skills-lock.json —
                // пользовательские .claude/.windsurf не трогаются.
                steps.push(Step::Generate {
                    id: "prisma_cleanup".into(),
                    label: "Clean up Prisma AI skills".into(),
                    description: "Remove Prisma agent skill directories (.agents, .claude, .windsurf, skills-lock.json)".into(),
                    generator_id: "fs-cleanup".into(),
                    generator_config: serde_json::json!({
                        "paths": [".agents", ".claude", ".windsurf", "skills-lock.json"]
                    }),
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
            // Инструменты из local_infra_tools поставлены локально:
            // для них в .env.example — локальные адреса (localhost),
            // а из docker-compose.yaml они исключаются (steps_for_docker).
            "postgresql" | "redis" | "mongodb" | "mysql" | "kafka" | "clickhouse" | "rabbitmq" | "minio" | "mailpit" => {
                if context.local_infra_tools.contains(tool_id) {
                    infra_envs.push(content::get_local_env_example(tool_id));
                } else {
                    infra_envs.push(content::get_env_example(tool_id));
                }
            }
            "grafana" | "opentelemetry" => {
                if tool_id == "grafana" {
                    if context.local_infra_tools.contains(tool_id) {
                        infra_envs.push(content::get_local_env_example(tool_id));
                    } else {
                        infra_envs.push(content::get_env_example(tool_id));
                    }
                }
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

    // Локально установленные инфра-инструменты: инструкция по запуску
    // (LOCAL_INFRA.md) — как поднять сервис и куда он смотрит.
    if !context.local_infra_tools.is_empty() {
        steps.push(write_file(
            "local_infra_guide",
            "Create LOCAL_INFRA.md",
            "LOCAL_INFRA.md",
            &content::generate_local_infra_guide(&context.local_infra_tools),
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

    // Локально установленные инфра-инструменты исключаются из docker-compose:
    // их сервисы уже запущены на машине, контейнер просто займёт порт.
    let docker_tools: Vec<String> = context
        .tools
        .iter()
        .filter(|t| !context.local_infra_tools.contains(t))
        .cloned()
        .collect();
    let services = content::collect_docker_services(&docker_tools);
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

/// Фаза 4: git init. Выполняется ДО шаблонизации (фаза 5) — чтобы
/// README/конфиги, написанные позже, попали в стартовый коммит.
fn steps_for_git_init(context: &WizardContext, project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();

    if !context.git_init {
        return steps;
    }

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

    steps
}

/// Фаза 5 (часть): .gitignore пишется в самой поздней фазе шаблонизации,
/// ПОСЛЕ всех CLI-фреймворков (create-next-app создаёт свой .gitignore —
/// наш шаблон обязан перезаписать его, overwrite=true).
fn steps_for_gitignore(context: &WizardContext, _project_path: &str) -> Vec<Step> {
    let mut steps = Vec::new();

    if !context.git_init {
        return steps;
    }

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

    steps
}

/// Фаза 6: финализация — самый конец пайплайна.
///   1. npm install: РОВНО один раз на каждый JS-каталог проекта (корень,
///      сегменты, подпапки фронтенд-каркасов). Скаффолдеры запускались с
///      --skip-install/--no-install, поэтому node_modules не плодятся на
///      каждом шаге генерации.
///   2. git add + git commit: README/конфиги уже записаны (фаза 5) и
///      попадают в стартовый коммит.
fn steps_for_finalize(context: &WizardContext, js_dirs: &[String], project_path: &str, project_name: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    for (i, dir) in js_dirs.iter().enumerate() {
        // "." = корень проекта; остальные каталоги — относительные пути,
        // которые разрешаются от project_path (как vite_install раньше)
        let wd = if dir == "." {
            project_path.to_string()
        } else {
            format!("{}/{}", project_path.trim_end_matches(['/', '\\']), dir)
        };
        if seen.contains(&wd) {
            continue;
        }
        seen.push(wd.clone());
        steps.push(Step::Command {
            id: format!("npm_install_{}", i),
            label: format!("Install npm dependencies ({})", wd),
            description: "Run npm install once, after all scaffolding".into(),
            command: "npm".into(),
            args: vec!["install".into()],
            working_dir: Some(wd),
            env: None,
            timeout_secs: Some(600),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        });
    }

    if !context.git_init {
        return steps;
    }

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
            format!("Initial commit: {} project", project_name),
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
        // Шаг идёт в финальной фазе шаблонизации ПОСЛЕ всех CLI-фреймворков
        // и обязан перезаписать README, созданный самим CLI (create-next-app,
        // nest new...) — иначе наш шаблон молча теряется.
        overwrite: true,
        condition: None,
        on_error: ErrorMode::Skip,
    }]
}




fn steps_for_vscode(context: &WizardContext) -> Vec<Step> {
    if !context.vscode_config {
        return Vec::new();
    }

    let primary_lang = context.languages.first().map(|s| s.as_str()).unwrap_or("python");

    // Каталоги для слияния: корень всегда + сегменты моно-репозитория +
    // каталоги scaffold-генераторов (frontend/ в монолите и рядом с
    // root-скаффолдами). В не-корневых каталогах генератор "vscode-merge"
    // примешивает конфиг только если .vscode/settings.json уже создал сам
    // CLI (create-next-app и т.п.) — лишние папки не дублируются.
    let layout = SegLayout::compute(context);
    let mut dirs: Vec<String> = vec![".".to_string()];
    if let Some(dir) = &layout.backend {
        dirs.push(dir.clone());
    }
    if let Some(dir) = &layout.frontend {
        dirs.push(dir.clone());
    }
    for fw in &context.frameworks {
        if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw.as_str()) {
            let dir = scaffold_target_dir(fw, None);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }

    vec![Step::Generate {
        id: "vscode_merge".into(),
        label: "Merge VS Code settings".into(),
        description: "Merge StackPilot VS Code settings with the ones created by CLIs".into(),
        generator_id: "vscode-merge".into(),
        generator_config: serde_json::json!({
            "lang": primary_lang,
            "dirs": dirs,
        }),
        condition: None,
        on_error: ErrorMode::Skip,
    }]
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
        // Фронтенды скаффолдятся в frontend/ (ScaffoldGenerator), а не в
        // корне: иначе они перезапишут package.json бэкенда (express+nextjs
        // и т.п.). solidjs остаётся обычным Command, создающим подпапку
        // <project_name>.
        for fw_id in ["nextjs", "nuxt", "sveltekit", "expo", "react"] {
            let steps = steps_for_framework(fw_id, "C:\\dev\\myapp", "myapp", &context(), None, false);
            let scaffold = steps
                .iter()
                .find(|s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"))
                .unwrap_or_else(|| panic!("{fw_id}: scaffold-шаг должен быть в плане"));
            match scaffold {
                Step::Generate { generator_config, .. } => {
                    assert_eq!(
                        generator_config.get("target_dir").and_then(|v| v.as_str()),
                        Some("frontend"),
                        "{fw_id} должен скаффолдиться в frontend/: {generator_config}"
                    );
                }
                _ => panic!("{fw_id}: scaffold — Generate"),
            }
        }

        // solidjs — обычный Command, создаёт подпапку с именем проекта
        let steps = steps_for_framework("solidjs", "C:\\dev\\myapp", "myapp", &context(), None, false);
        let create = steps.iter().find(|s| s.id() == "solid_init")
            .unwrap_or_else(|| panic!("solidjs: шаг solid_init должен быть в плане"));
        let args = cmd_args(create);
        assert!(
            args.contains(&"myapp".to_string()),
            "solidjs не создаёт проект в подпапке: {args:?}"
        );
    }

    #[test]
    fn split_layout_puts_frameworks_into_segments() {
        // nextjs + fastapi: фронтенд — в frontend/, сервер — в backend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let next_steps = steps_for_framework("nextjs", "C:\\dev\\myapp", "myapp", &ctx, Some("frontend"), false);
        let scaffold = next_steps.iter().find(|s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"))
            .expect("nextjs: scaffold-шаг должен быть в плане");
        match scaffold {
            Step::Generate { generator_config, .. } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
            }
            _ => panic!("nextjs — Generate"),
        }

        let api_steps = steps_for_framework("fastapi", "C:\\dev\\myapp", "myapp", &ctx, Some("backend"), false);
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

        let api_steps = steps_for_framework("fastapi", "C:\\dev\\myapp", "myapp", &ctx, None, false);
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

        // nextjs (create-next-app) — scaffold-генератор: каталогом становится
        // сегмент frontend/ (ScaffoldGenerator выполнит CLI с "." внутри)
        let next = recipe.steps.iter().find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Generate { generator_config, .. } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
            }
            _ => panic!("nextjs_create — Generate"),
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
    fn tauri_pipeline_scaffolds_frontend_init_and_patches_config() {
        // tauri (scaffold="root") НЕ поглощает компаньонов: react скаффолдится
        // в frontend/ отдельным scaffold-шагом, затем tauri init --ci в корне,
        // затем Rust-патч tauri.conf.json. create-tauri-app убран из
        // пайплайна (его фронтенд-каркас в корне был пустым без node_modules).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // create-tauri-app создаёт Cargo.toml сам — language-скаффолд подавлен
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "cargo_init"),
            "cargo init не нужен: tauri создаёт каркас"
        );
        // Старый CLI-first шаг полностью убран
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_create"),
            "create-tauri-app убран из пайплайна"
        );
        // Segment-папка backend/ при root-скаффолде не создаётся; frontend/
        // создаёт движок ДО скаффолда компаньона (CLI работает с "." внутри)
        assert!(
            !recipe.steps.iter().any(|s| matches!(s, Step::CreateDirectory { path, .. }
                if path == "backend")),
            "root-скаффолд не должен создавать backend/"
        );
        let frontend_dir_idx = recipe.steps.iter().position(|s| matches!(s, Step::CreateDirectory { path, .. }
            if path == "frontend"))
            .expect("frontend/ должен создаваться движком");
        assert!(
            frontend_dir_idx < recipe.steps.iter().position(|s| s.id() == "vite_create").unwrap(),
            "frontend/ создаётся до скаффолда компаньона"
        );
        // Компаньон react скаффолдится отдельно (не подавляется tauri)
        let react_scaffold = recipe.steps.iter().find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match react_scaffold {
            Step::Generate { generator_config, on_error, .. } => {
                assert_eq!(generator_config.get("target_dir").and_then(|v| v.as_str()), Some("frontend"),
                    "react скаффолдится в frontend/: {generator_config}");
                assert_eq!(on_error, &ErrorMode::Skip);
            }
            _ => panic!("vite_create — Generate"),
        }

        // tauri — root-фреймворк: его шаги идут в фазе 1, ДО компаньонов.
        // tauri init только пишет конфиг (фронтенд ему не нужен), поэтому
        // порядок «init → vite-скаффолд компаньона» корректен.
        let init_idx = recipe.steps.iter().position(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        match &recipe.steps[init_idx] {
            Step::Command { command, args, working_dir, .. } => {
                assert_eq!(command, "npx");
                assert!(args.iter().any(|a| a == "--yes"), "{args:?}");
                assert!(args.iter().any(|a| a == "@tauri-apps/cli@latest"), "{args:?}");
                assert!(args.iter().any(|a| a == "--ci"), "init должен быть неинтерактивным: {args:?}");
                assert!(args.iter().any(|a| a == "--frontend-dist"), "{args:?}");
                assert!(args.iter().any(|a| a == "--before-dev-command"), "{args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp"), "tauri init работает в корне");
            }
            _ => panic!("tauri_init — Command"),
        }

        // Rust-патч tauri.conf.json — сразу после tauri init
        let patch_idx = recipe.steps.iter().position(|s| s.id() == "tauri_config_patch")
            .expect("tauri_config_patch должен быть в плане");
        assert!(init_idx < patch_idx, "патч конфига идёт после tauri init");
        match &recipe.steps[patch_idx] {
            Step::Generate { generator_id, generator_config, on_error, .. } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(generator_config.get("identifier").and_then(|v| v.as_str()), Some("com.myapp"));
                assert_eq!(generator_config.get("frontend_dir").and_then(|v| v.as_str()), Some("frontend"));
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // При компаньоне vite-vanilla-скаффолд tauri не нужен
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_web_scaffold"),
            "компаньон сам скаффолдит frontend/"
        );

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
            }
            _ => panic!("npm_install — Command"),
        }

        // Баг шаблонизатора: патч имени package.json работает в frontend/
        // (там лежит package.json), а не в корне
        let patch = recipe.steps.iter().find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command { working_dir, args, .. } => {
                assert_eq!(working_dir.as_deref(), Some("frontend"), "package.json лежит в frontend/");
                assert!(
                    args[1].contains("j.name='myapp'"),
                    "патч должен писать project_name: {:?}",
                    args
                );
            }
            _ => panic!("react_pkg_name — Command"),
        }
    }

    #[test]
    fn tauri_without_companion_scaffolds_vanilla_frontend() {
        // tauri без react/vue/svelte: vite (vanilla-ts) скаффолдит frontend/,
        // tauri init идёт следом. Generic js/ts-скаффолд в корне подавлен
        // (его заглушки конфликтовали бы с tauri-каркасом).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        assert!(
            !recipe.steps.iter().any(|s| s.id() == "package_json"),
            "generic js-скаффолд не нужен: фронтенд создаёт vite-vanilla"
        );
        let web = recipe.steps.iter().find(|s| s.id() == "tauri_web_scaffold")
            .expect("tauri_web_scaffold должен быть в плане");
        match web {
            Step::Generate { generator_config, .. } => {
                let args = generator_config.get("args").and_then(|a| a.as_array())
                    .cloned().unwrap_or_default();
                assert_eq!(args.get(3).and_then(|v| v.as_str()), Some("vanilla-ts"), "typescript → vanilla-ts");
                assert_eq!(generator_config.get("target_dir").and_then(|v| v.as_str()), Some("frontend"));
            }
            _ => panic!("tauri_web_scaffold — Generate"),
        }

        let scaffold_idx = recipe.steps.iter().position(|s| s.id() == "tauri_web_scaffold").unwrap();
        let init_idx = recipe.steps.iter().position(|s| s.id() == "tauri_init").unwrap();
        assert!(scaffold_idx < init_idx, "фронтенд скаффолдится до tauri init");

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
            }
            _ => panic!("npm_install — Command"),
        }

        // Патч имени package.json для tauri — в frontend/
        let patch = recipe.steps.iter().find(|s| s.id() == "tauri_pkg_name")
            .expect("tauri_pkg_name должен быть в плане");
        match patch {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("frontend"), "package.json лежит в frontend/");
            }
            _ => panic!("tauri_pkg_name — Command"),
        }
    }

    #[test]
    fn root_scaffold_runs_first_no_segments() {
        // Строгая очередь фаз: 1. Root CLI (django) → 2. Subdir (react) →
        // 3. Инструменты (prisma) → 4. Конфиги (docker-compose).
        //
        // Корнем владеет django — react получает собственный сегмент
        // frontend/ (движок создаёт папку, vite работает ВНУТРИ с "."),
        // иначе create-vite создал бы вложенную папку myapp/ — матрёшка.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        ctx.tools = vec!["prisma".into(), "postgresql".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // backend/ сегмент НЕ создаётся: корнем владеет django
        assert!(
            !recipe.steps.iter().any(|s| matches!(s, Step::CreateDirectory { path, .. }
                if path == "backend")),
            "при django (root) не должно быть backend/ — корнем владеет CLI"
        );
        // react (side=frontend) сегментируется: движок создаёт frontend/
        assert!(
            recipe.steps.iter().any(|s| matches!(s, Step::CreateDirectory { path, .. }
                if path == "frontend")),
            "компаньон root-скаффолда должен получить frontend/"
        );

        let idx = |id: &str| recipe.steps.iter().position(|s| s.id() == id)
            .unwrap_or_else(|| panic!("{id} должен быть в плане"));
        assert!(idx("django_start") < idx("vite_create"), "root CLI идёт до subdir-скаффолда");
        assert!(idx("vite_create") < idx("prisma_init"), "subdir-скаффолд идёт до инструментов");
        assert!(idx("prisma_init") < idx("docker_compose"), "инструменты идут до конфигов");

        // django-admin startproject работает в корне проекта, а не в backend/
        match &recipe.steps[idx("django_start")] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp"), "django стартует в корне");
            }
            _ => panic!("django_start — Command"),
        }

        // react (subdir) НЕ подавляется django (у django нет компаньонов)
        assert!(
            recipe.steps.iter().any(|s| s.id() == "vite_create"),
            "django не поглощает react — vite-скаффолд остаётся"
        );

        // НЕТ матрёшки: ScaffoldGenerator выполняет create-vite ВНУТРИ
        // frontend/ с "." — а не создаёт вложенную папку myapp/ в корне django
        let vite = &recipe.steps[idx("vite_create")];
        match vite {
            Step::Generate { generator_config, .. } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "vite скаффолдится в frontend/: {generator_config}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
            }
            _ => panic!("npm_install — Command"),
        }
        assert!(idx("npm_install_0") > idx("readme"), "npm install — в финальной фазе, после шаблонизации");
    }

    #[test]
    fn nest_plus_nextjs_has_no_matryoshka() {
        // P1: NestJS + Next.js создавали testapp2/testapp2 — nest скаффолдил
        // корень, а create-next-app внутри него ещё и вложенную папку с
        // именем проекта. Теперь: движок создаёт frontend/, nextjs выполняется
        // ВНУТРИ него с "."; оба CLI идут с --skip-install, npm install —
        // ровно 2 раза в финальной фазе (корень nest + frontend).
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "nextjs".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // frontend/ создаётся движком ДО запуска nextjs
        let create_idx = recipe.steps.iter().position(|s| matches!(s, Step::CreateDirectory { path, .. }
            if path == "frontend"))
            .expect("frontend/ должен создаваться движком");
        let next_idx = recipe.steps.iter().position(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        assert!(create_idx < next_idx, "frontend/ создаётся до запуска nextjs");

        // НЕТ матрёшки: ScaffoldGenerator выполняет create-next-app ВНУТРИ
        // frontend/ с "." (--skip-install — зависимости в финальной фазе)
        let next = &recipe.steps[next_idx];
        match next {
            Step::Generate { generator_config, on_error, .. } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
                assert_eq!(on_error, &ErrorMode::Skip);
                let args = generator_config.get("args").and_then(|a| a.as_array())
                    .cloned().unwrap_or_default();
                assert!(
                    args.iter().any(|a| a == "--skip-install"),
                    "create-next-app должен идти с --skip-install: {args:?}"
                );
            }
            _ => panic!("nextjs_create — Generate"),
        }

        // nest: "." + --skip-install + --skip-git, в корне
        let nest = recipe.steps.iter().find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        match nest {
            Step::Command { args, working_dir, .. } => {
                assert_eq!(args.get(2).map(String::as_str), Some("."), "{args:?}");
                assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
                assert!(args.contains(&"--skip-git".to_string()), "git инициализирует движок: {args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp"), "nest работает в корне");
            }
            _ => panic!("nest_new — Command"),
        }

        // npm install ровно 2 раза: корень (nest) + frontend (nextjs)
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 2, "install-шагов должно быть 2: {installs:?}");
        match (&installs[0], &installs[1]) {
            (Step::Command { working_dir: w0, .. }, Step::Command { working_dir: w1, .. }) => {
                assert_eq!(w0.as_deref(), Some("C:\\dev\\myapp"), "nest-корень ставится первым");
                assert_eq!(w1.as_deref(), Some("C:\\dev\\myapp/frontend"), "nextjs ставится вторым");
            }
            _ => panic!("npm_install — Command"),
        }
    }

    #[test]
    fn templating_runs_last_and_overwrites_cli_files() {
        // P4: README.md/.gitignore/docker-compose пишутся ПОСЛЕ всех
        // CLI-фреймворков и с overwrite=true — иначе create-next-app/nest new
        // перезаписывают/удаляют наш шаблон.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        ctx.git_init = true;
        ctx.docker = true;
        // postgres нужен, чтобы docker-compose сгенерировался (без сервисов
        // compose-шага в плане нет — это отдельный инвариант)
        ctx.tools = vec!["postgresql".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| recipe.steps.iter().position(|s| s.id() == id)
            .unwrap_or_else(|| panic!("{id} должен быть в плане"));

        // Шаблонизация ПОСЛЕ скаффолдинга...
        assert!(idx("nextjs_create") < idx("readme"), "README пишется после CLI");
        assert!(idx("nextjs_create") < idx("gitignore"), ".gitignore пишется после CLI");
        assert!(idx("nextjs_create") < idx("docker_compose"), "docker-compose пишется после CLI");
        // ...но до git add/commit и npm install
        assert!(idx("git_init") < idx("readme"), "git init до шаблонизации — README в коммите");
        assert!(idx("readme") < idx("git_add"), "README до стартового коммита");
        assert!(idx("readme") < idx("npm_install_0"), "npm install — после шаблонизации");

        for id in ["readme", "gitignore", "docker_compose"] {
            match &recipe.steps[idx(id)] {
                Step::WriteFile { overwrite, .. } => {
                    assert!(*overwrite, "{id} должен перезаписывать файлы CLI");
                }
                _ => panic!("{id} — WriteFile"),
            }
        }
    }

    #[test]
    fn spring_boot_generation_goes_through_generator() {
        // P3: Spring Boot НЕ качается curl'ом в project.zip (ошибка
        // Initializr писалась в файл и падала на распаковке с «Error opening
        // archive») — вместо этого Step::Generate с генератором
        // "spring-boot", который проверяет HTTP-статус и останавливает
        // пайплайн (Abort) с реальной причиной.
        let mut ctx = context();
        ctx.languages = vec!["java".into()];
        ctx.frameworks = vec!["spring-boot".into()];
        ctx.tools = vec!["postgresql".into(), "redis".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let gen = recipe.steps.iter().find(|s| s.id() == "spring_init")
            .expect("spring_init должен быть в плане");
        match gen {
            Step::Generate { generator_id, generator_config, on_error, .. } => {
                assert_eq!(generator_id, "spring-boot");
                assert_eq!(on_error, &ErrorMode::Abort);
                let deps = generator_config.get("dependencies").and_then(|d| d.as_str());
                assert_eq!(deps, Some("web,data-jpa,postgresql,data-redis"), "зависимости собираются из tools");
                let name = generator_config.get("project_name").and_then(|n| n.as_str());
                assert_eq!(name, Some("myapp"));
            }
            _ => panic!("spring_init — Generate"),
        }
        // Никаких curl/unzip шагов с project.zip
        assert!(!recipe.steps.iter().any(|s| s.id() == "unzip_spring"));
        assert!(!recipe.steps.iter().any(|s| s.id() == "cleanup_zip"));
    }

    #[test]
    fn prisma_init_is_non_interactive() {
        // Prisma не должен спрашивать «how to set up your database»:
        // провайдер передаётся флагом, npx — с --yes / CI=1 (executor).
        let mut ctx = context();
        ctx.tools = vec!["prisma".into(), "postgresql".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let step = recipe.steps.iter().find(|s| s.id() == "prisma_init")
            .expect("prisma_init должен быть в плане");
        match step {
            Step::Command { command, args, .. } => {
                assert_eq!(command, "npx");
                assert!(args.contains(&"--yes".to_string()), "{args:?}");
                let provider = args.iter().position(|a| a == "--datasource-provider")
                    .map(|i| args[i + 1].as_str());
                assert_eq!(provider, Some("postgresql"), "{args:?}");
                // Prisma 6.16+ разворачивает AI-навыки (.agents/.claude/...,
                // десятки тысяч файлов) — отключаем флагом
                assert!(args.contains(&"--no-skills".to_string()), "prisma init без --no-skills: {args:?}");
            }
            _ => panic!("prisma_init — Command"),
        }

        // Подстраховка: Rust-генератор принудительно чистит агентные папки
        let cleanup = recipe.steps.iter().find(|s| s.id() == "prisma_cleanup")
            .expect("prisma_cleanup должен быть в плане");
        match cleanup {
            Step::Generate { generator_id, generator_config, on_error, .. } => {
                assert_eq!(generator_id, "fs-cleanup");
                assert_eq!(on_error, &ErrorMode::Skip);
                let paths = generator_config.get("paths").and_then(|p| p.as_array());
                assert!(paths.is_some_and(|p| p.iter().any(|v| v == ".agents")));
            }
            _ => panic!("prisma_cleanup — Generate"),
        }
        let cleanup_idx = recipe.steps.iter().position(|s| s.id() == "prisma_cleanup").unwrap();
        let init_idx = recipe.steps.iter().position(|s| s.id() == "prisma_init").unwrap();
        assert!(init_idx < cleanup_idx, "очистка идёт после init");
    }

    #[test]
    fn segmented_vite_package_name_patched_to_project_name() {
        // Баг шаблонизатора: create-vite frontend → package.json name
        // = "frontend". Движок добавляет пост-шаг, переписывающий name
        // на project_name из WizardContext.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["react".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let patch = recipe.steps.iter().find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command { working_dir, args, .. } => {
                assert_eq!(working_dir.as_deref(), Some("frontend"), "патч работает в папке скаффолда");
                assert!(
                    args[1].contains("j.name='myapp'"),
                    "name берётся из project_name, а не из папки frontend: {:?}",
                    args
                );
            }
            _ => panic!("react_pkg_name — Command"),
        }
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
            let steps = steps_for_framework(fw, "C:\\dev\\myapp", "myapp", &context(), None, false);
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

        // react: ScaffoldGenerator кладёт vite-проект в frontend/
        let vite = recipe.steps.iter().find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match vite {
            Step::Generate { generator_config, .. } => {
                assert_eq!(generator_config.get("command").and_then(|v| v.as_str()), Some("npx"));
                let args = generator_config.get("args").and_then(|a| a.as_array())
                    .cloned().unwrap_or_default();
                assert_eq!(args.get(0).and_then(|v| v.as_str()), Some("create-vite@latest"));
                assert_eq!(args.get(3).and_then(|v| v.as_str()), Some("react-ts"),
                    "typescript → react-ts шаблон: {args:?}");
                assert_eq!(generator_config.get("target_dir").and_then(|v| v.as_str()), Some("frontend"),
                    "в монолите vite-проект живёт в frontend/: {generator_config}");
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install выполняется РОВНО один раз в финальной фазе пайплайна —
        // ВНУТРИ frontend/ (vite_install из середины пайплайна убран,
        // зависимости больше не плодятся на каждом шаге)
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, args, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
                assert_eq!(args, &vec!["install".to_string()]);
            }
            _ => panic!("npm_install — Command"),
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

    #[test]
    fn every_requires_docker_tool_lands_in_compose() {
        // Каждый инструмент мастера с requires_docker: true обязан давать
        // сервис в collect_docker_services: локально он не ставится
        // (toolchain не требует его для проверки окружения), а
        // docker-compose.yaml — единственный способ его развернуть.
        let raw = include_str!("../knowledge/wizard_tree.json");
        let tree: serde_json::Value =
            serde_json::from_str(raw).expect("wizard_tree.json должен быть корректным JSON");
        let docker_tools: Vec<String> = tree
            .get("tools")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter(|t| {
                        t.get("requires_docker")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
                    .filter_map(|t| t.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        assert!(
            !docker_tools.is_empty(),
            "в мастере должны быть docker-инструменты"
        );
        for tool in &docker_tools {
            let services = content::collect_docker_services(std::slice::from_ref(tool));
            assert!(
                !services.is_empty(),
                "requires_docker-инструмент {tool} не создаёт сервис в docker-compose"
            );
        }
    }

    #[test]
    fn alembic_init_runs_after_venv_setup_via_venv_binary() {
        // Проблема: `alembic init` вызывался ДО создания venv и pip install —
        // системная команда не находилась. Порядок обязан быть таким:
        // py_venv_create → py_pip_install → alembic_init, и сам alembic
        // вызывается строго через бинарь виртуального окружения.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["alembic".into(), "sqlalchemy".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };

        assert!(idx("py_venv_create") < idx("py_pip_install"), "venv создаётся до pip install");
        assert!(
            idx("py_pip_install") < idx("alembic_init"),
            "pip install обязан идти ДО alembic init"
        );

        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command { command, .. } => {
                assert!(
                    command.contains("venv"),
                    "alembic должен вызываться из venv, а не системно: {command}"
                );
            }
            _ => panic!("alembic_init — Command"),
        }

        // pip-шаг гарантированно ставит alembic (даже если requirements.txt
        // его не содержит — он создаётся пустым, fastapi/flask перезаписывают
        // только своими пакетами).
        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        let pip_args = cmd_args(pip);
        assert!(
            pip_args.iter().any(|a| a == "alembic"),
            "pip должен ставить alembic явно: {pip_args:?}"
        );
    }

    #[test]
    fn alembic_venv_lives_inside_backend_segment_in_monorepo() {
        // Моно-репозиторий (python backend + typescript frontend): python-код
        // и requirements.txt лежат в backend/, поэтому venv создаётся ВНУТРИ
        // backend/, а не в корне проекта. Путь к бинарю формируется как
        // backend/venv/Scripts/alembic.exe (Win) или backend/venv/bin/alembic.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["alembic".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        let venv_create = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create должен быть в плане");
        let venv_args = cmd_args(venv_create);
        assert!(
            venv_args.iter().any(|a| a.ends_with("backend/venv") || a.ends_with("backend\\venv")),
            "venv создаётся внутри backend/: {venv_args:?}"
        );

        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        match pip {
            Step::Command { command, args, .. } => {
                assert!(
                    command.contains("backend"),
                    "pip вызывается из venv внутри backend/: {command}"
                );
                assert!(
                    args.iter().any(|a| a.ends_with("backend/requirements.txt")
                        || a.ends_with("backend\\requirements.txt")),
                    "pip читает requirements.txt из backend/: {args:?}"
                );
            }
            _ => panic!("py_pip_install — Command"),
        }

        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command { command, on_error, .. } => {
                assert!(
                    command.contains("backend") && command.contains("venv"),
                    "alembic вызывается из backend/venv: {command}"
                );
                assert_eq!(
                    on_error,
                    &ErrorMode::Abort,
                    "alembic_init не должен проваливаться молча (Skip)"
                );
            }
            _ => panic!("alembic_init — Command"),
        }
    }

    #[test]
    fn python_venv_bin_resolves_inside_segment() {
        // Корень проекта: venv\Scripts\alembic.exe (Win) / venv/bin/alembic.
        let root_bin = python_venv_bin(".", "alembic");
        assert!(
            root_bin.contains("venv") && !root_bin.contains("backend"),
            "корневой venv без сегмента: {root_bin}"
        );
        assert!(
            root_bin.ends_with("alembic.exe") || root_bin.ends_with("/alembic"),
            "имя бинаря на конце: {root_bin}"
        );
        // Моно-репозиторий: backend/venv/Scripts/alembic.exe (Win) /
        // backend/venv/bin/alembic (unix).
        let seg_bin = python_venv_bin("backend", "alembic");
        assert!(
            seg_bin.starts_with("backend") && seg_bin.contains("venv"),
            "бинарь внутри backend/venv: {seg_bin}"
        );
        assert!(
            seg_bin.ends_with("alembic.exe") || seg_bin.ends_with("/alembic"),
            "имя бинаря на конце: {seg_bin}"
        );
    }

    #[test]
    fn venv_steps_are_only_created_for_python_with_alembic() {
        // Без alembic (или без Python) venv-шаги не плодятся.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["sqlalchemy".into()];
        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().all(|s| s.id() != "py_venv_create"),
            "venv не нужен без alembic"
        );

        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().all(|s| s.id() != "py_venv_create"),
            "venv не нужен без Python"
        );
    }

    #[test]
    fn qt_webengine_stack_generates_webengine_files() {
        // qt-webengine + react: main.cpp обязан содержать QWebEngineView-
        // бойлерплейт, CMakeLists.txt — WebEngineWidgets (не заглушки).
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "react".into()];
        ctx.answers.insert("qt_ui".into(), vec!["qt-webengine".into()]);

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        let main_cpp = recipe
            .steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "src/main.cpp"))
            .unwrap_or_else(|| panic!("qt должен писать src/main.cpp"));
        match main_cpp {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("#include <QWebEngineView>"), "{content}");
                assert!(content.contains("QWebEngineView view;"), "{content}");
                assert!(content.contains("qrc:/web/index.html"), "{content}");
                assert!(
                    content.contains("QApplication app(argc, argv)"),
                    "нужен <QApplication> бойлерплейт: {content}"
                );
            }
            _ => panic!("qt_main — WriteFile"),
        }

        let cmake = recipe
            .steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "CMakeLists.txt"))
            .unwrap_or_else(|| panic!("qt должен писать CMakeLists.txt"));
        match cmake {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("find_package(Qt6 REQUIRED COMPONENTS WebEngineWidgets)"),
                    "{content}"
                );
                assert!(
                    content.contains("target_link_libraries(myapp Qt6::WebEngineWidgets)"),
                    "{content}"
                );
                assert!(content.contains("qt_add_resources"), "{content}");
            }
            _ => panic!("qt_cmake — WriteFile"),
        }
    }

    #[test]
    fn laravel_and_symfony_use_composer_not_npm() {
        // P-баг: @laravel/installer падал с «npm error 404 Not Found», а
        // бинарь symfony не установлен. PHP-фреймворки создаются через
        // composer create-project --no-interaction (dot-режим в корне).
        for (fw_id, step_id, package) in [
            ("laravel", "laravel_new", "laravel/laravel"),
            ("symfony", "symfony_new", "symfony/skeleton"),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["php".into()];
            ctx.frameworks = vec![fw_id.into()];

            let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");
            assert!(
                !recipe.steps.iter().any(|s| s.id().contains("installer") || s.id().contains("@laravel")),
                "npm-путь @laravel/installer не должен использоваться"
            );

            let step = recipe.steps.iter().find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::Generate { generator_id, generator_config, on_error, .. } => {
                    assert_eq!(generator_id, "scaffold");
                    assert_eq!(on_error, &ErrorMode::Skip);
                    assert_eq!(generator_config.get("command").and_then(|v| v.as_str()), Some("composer"));
                    let args = generator_config.get("args").and_then(|a| a.as_array())
                        .cloned().unwrap_or_default();
                    assert_eq!(args.get(0).and_then(|v| v.as_str()), Some("create-project"));
                    assert_eq!(args.get(1).and_then(|v| v.as_str()), Some(package), "{args:?}");
                    assert!(args.iter().any(|a| a == "--no-interaction"), "{args:?}");
                    assert_eq!(generator_config.get("target_dir").and_then(|v| v.as_str()), Some("."),
                        "в монолите PHP-фреймворк живёт в корне");
                }
                _ => panic!("{step_id} — Generate"),
            }
        }
    }
}
