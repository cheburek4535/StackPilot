pub mod content;
pub mod executor;
pub mod template;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    // Strict Subdir Mandate: при обеих сторонах (backend + frontend)
    // scaffold="root" принудительно работает как "subdir" — фреймворк
    // попадает в rest_frameworks и сегментируется в ./backend или ./frontend.
    let layout = SegLayout::compute(context);
    let root_present = context.frameworks.iter().any(|fw| is_root_scaffold(fw, context));
    let mut rest_frameworks: Vec<String> = Vec::new();
    // Каталоги, в которых после всех CLI-каркасов нужен РОВНО ОДИН npm install
    // ("." = корень проекта). Скаффолдеры запускаются с --skip-install/
    // --no-install, поэтому node_modules не плодятся на каждом шаге.
    let mut js_dirs: Vec<String> = Vec::new();
    for fw in &context.frameworks {
        if is_root_scaffold(fw, context) {
            steps.extend(steps_for_framework(fw, project_path, project_name, context, None, false));
            // Root-JS-фреймворк (nest): работает в корне с --skip-install,
            // его package.json ставится один раз в финальной фазе.
            if is_js_framework(fw) {
                push_unique(&mut js_dirs, ".".to_string());
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

    // Django CLI должен запускаться из проектного Python-окружения. Раньше
    // venv создавался только в фазе инструментов (из-за Alembic), то есть
    // уже ПОСЛЕ `django-admin startproject`; на чистой машине команда тихо
    // падала, а все последующие шаги продолжали работать с пустым backend/.
    // Подготавливаем окружение сразу после записи requirements.txt и до
    // любого Python-фреймворка. Полная установка requirements/Alembic
    // выполняется позже, когда все framework-шаги уже записали зависимости.
    if context.languages.iter().any(|l| l == "python")
        && context.frameworks.iter().any(|f| f == "django")
    {
        let python_dir = python_segment_dir(context);
        let base = std::path::PathBuf::from(project_path);
        let venv_path = if python_dir == "." {
            base.join("venv")
        } else {
            base.join(&python_dir).join("venv")
        };
        let venv_str = venv_path.to_string_lossy().into_owned();
        steps.push(Step::Command {
            id: "django_venv_create".into(),
            label: "Create Python virtual environment".into(),
            description: format!("Run python -m venv {}", venv_str),
            command: "python".into(),
            args: vec!["-m".into(), "venv".into(), venv_str],
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(120),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        });
        // На этом этапе framework-specific шаги ещё не успели записать
        // requirements.txt (aiogram и инструменты добавляют его позже),
        // поэтому ставим только Django. Полная установка requirements и
        // Alembic выполняется штатной фазой tools после всех scaffold-шагов.
        let pip_args = vec!["install".into(), "django".into()];
        steps.push(Step::Command {
            id: "django_pip_install".into(),
            label: "Install Django dependencies".into(),
            description: "Install Python requirements in the project virtual environment".into(),
            command: python_venv_bin(project_path, &python_dir, "pip"),
            args: pip_args,
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(600),
            condition: None,
            on_error: ErrorMode::Abort,
            interactive: vec![],
        });
    }

    // Tauri-шаги откладываются в конец фазы Subdir Scaffolding: пайплайн
    // tauri обязан выполнять фронтенд-генератор ПЕРВЫМ (vite в frontend/ →
    // npm install → cargo tauri init), иначе init опережает каркас фронтенда.
    let mut deferred_tauri: Vec<Step> = Vec::new();
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
        if fw == "tauri" {
            // tauri инициализируется ПОСЛЕ всех фронтенд-каркасов
            // (см. выше: фронтенд-генератор FIRST → npm install → tauri init).
            deferred_tauri = fw_steps;
        } else {
            steps.extend(fw_steps);
        }
    }
    steps.extend(deferred_tauri);

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

    // Слияние вложенных .vscode (frontend/.vscode, backend/.vscode) в корневой
    // .vscode/ с удалением вложенных папок — ПОСЛЕ всех CLI-скаффолдеров и
    // vscode-merge, чтобы конфиги CLI (typescript.tsdk и т.п.) не потерялись.
    // Генератор "vscode-folders" — no-op, если вложенных .vscode нет.
    steps.push(Step::Generate {
        id: "merge_inner_vscode".into(),
        label: "Merge inner .vscode folders".into(),
        description: "Merge frontend/.vscode and backend/.vscode into the root .vscode/".into(),
        generator_id: "vscode-folders".into(),
        generator_config: serde_json::json!({}),
        condition: None,
        on_error: ErrorMode::Skip,
    });

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

// ============================================================================
// Strict Subdir Mandate: если в WizardContext есть И бэкенд, И фронтенд,
// любой scaffold="root" из wizard_tree.json принудительно работает как
// "subdir" — бэкенд живёт строго в ./backend, фронтенд строго в ./frontend.
// Без этого nest/django/spring-boot скаффолдили корень, а CLI-компаньоны
// (nextjs и т.п.) падали поверх них: testapp/testapp-инцепция, двойные
// node_modules, файлы бэкенда в глобальном корне.
// ============================================================================

/// Стороны проекта по контексту: явные назначения мастера
/// (backend_languages/frontend_languages), вывод по category языков — и side
/// фреймворков из wizard_tree.json. Фреймворк с жёсткой стороной (side=
/// "backend"/"frontend") — полноценная сторона: nest (backend) + nextjs
/// (frontend) включают сегментацию, даже если в контексте единственный язык
/// (typescript) или он не назначен бэкенд-стороне (aspnetcore + maui — оба
/// на csharp).
fn context_sides(context: &WizardContext) -> (bool, bool) {
    let mut lang_side: HashMap<String, &'static str> = HashMap::new();
    for l in &context.backend_languages {
        lang_side.insert(l.clone(), "backend");
    }
    for l in &context.frontend_languages {
        lang_side.insert(l.clone(), "frontend");
    }
    for l in &context.languages {
        lang_side
            .entry(l.clone())
            .or_insert_with(|| language_side_infer(l).unwrap_or("backend"));
    }
    let mut has_backend = lang_side.values().any(|s| *s == "backend");
    let mut has_frontend = lang_side.values().any(|s| *s == "frontend");
    for fw in &context.frameworks {
        match framework_def(fw).map(|def| def.side.as_str()) {
            Some("backend") => has_backend = true,
            Some("frontend") => has_frontend = true,
            _ => {}
        }
    }
    (has_backend, has_frontend)
}

/// Есть ли у контекста ОБЕ стороны (backend + frontend)? Стороны берутся
/// из явных назначений мастера (backend_languages/frontend_languages),
/// выводятся из category языка и из side фреймворков (см. context_sides).
fn context_has_both_sides(context: &WizardContext) -> bool {
    let (has_backend, has_frontend) = context_sides(context);
    has_backend && has_frontend
}

/// Эффективный режим скаффолдинга фреймворка (Strict Subdir Mandate):
///   - если в контексте есть обе стороны (backend + frontend), любой
///     scaffold="root" из wizard_tree.json считается "subdir";
///   - иначе — значение из wizard_tree.json как есть.
fn effective_scaffold(fw: &str, context: &WizardContext) -> Option<&'static str> {
    let def = framework_def(fw)?;
    // Tauri is an integrated desktop scaffold: its own frontend and
    // `src-tauri/` shell must be laid out by the Tauri pipeline at the project
    // root. Never reinterpret it as a backend subdirectory in a mixed stack.
    if fw == "tauri" {
        return def.scaffold.as_deref();
    }
    if def.scaffold.as_deref() == Some("root") && context_has_both_sides(context) {
        return Some("subdir");
    }
    def.scaffold.as_deref()
}

/// Фреймворк всё ещё владеет корнем проекта (root-скаффолд, и мандат
/// обеих сторон не перевёл его в subdir-режим)?
fn is_root_scaffold(fw: &str, context: &WizardContext) -> bool {
    effective_scaffold(fw, context) == Some("root")
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
        //
        // Исключение — Strict Subdir Mandate: когда в контексте есть ОБЕ
        // стороны (backend + frontend), root-скаффолды принудительно
        // переводятся в subdir-режим (effective_scaffold) и сегментация
        // backend//frontend/ ВКЛЮЧАЕТСЯ — бэкенд обязан лежать в ./backend,
        // фронтенд — в ./frontend.
        if context.frameworks.iter().any(|fw| is_root_scaffold(fw, context)) {
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

        // Стороны определяются так же, как в context_has_both_sides: и
        // языки, и side фреймворков. Без этого aspnetcore + maui (оба на
        // csharp, категория "both") или nest + nextjs (только typescript)
        // не включили бы сегментацию и столкнулись бы файлами в корне.
        let (has_backend, has_frontend) = context_sides(context);
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
                } else if id.starts_with("tauri_web_") {
                    // Веб-часть tauri живёт в frontend/ независимо от сегмента
                    // самого tauri (backend/ в моно-репозитории) — рабочая
                    // директория уже относительна корня проекта, сегментация
                    // её НЕ трогает.
                    Step::Command { id, label, description, command, args, working_dir, env, timeout_secs, condition, on_error, interactive }
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
            // Исключение — веб-скаффолд tauri (tauri_web_scaffold): его
            // каталог ВСЕГДА frontend/ (веб-часть рядом с сегментом tauri).
            // Spring Boot (генератор "spring-boot") распаковывает starter
            // внутри сегмента — каталог передаётся через target_dir.
            Step::Generate { id, label, description, generator_id, mut generator_config, condition, on_error } => {
                if generator_id == "scaffold" && id != "tauri_web_scaffold" {
                    generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                }
                if generator_id == "spring-boot" {
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

/// Root-фреймворк (scaffold="root" ПОСЛЕ Strict Subdir Mandate) со своим CLI
/// скаффолдит своих frontend-компаньонов сам (create-tauri-app → react-ts).
/// Шаги такого компаньона подавляются: второй фронтенд (лишняя vite-папка)
/// поверх каркаса root-фреймворка не нужен.
///
/// Исключение — tauri: с новым пайплайном (frontend/ + tauri init --ci)
/// компаньон react/vue/svelte скаффолдится ОТДЕЛЬНО в frontend/ и не
/// подавляется. При обеих сторонах (мандат) root-фреймворки тоже не
/// поглощают компаньонов — каждый сегмент живёт сам по себе.
fn root_scaffold_consumes_companion(fw: &str, context: &WizardContext) -> bool {
    context.frameworks.iter().any(|root| {
        root != "tauri"
            && is_root_scaffold(root, context)
            && framework_def(root).is_some_and(|def| def.companions.iter().any(|c| c == fw))
    })
}

fn steps_for_framework(fw: &str, project_path: &str, project_name: &str, context: &WizardContext, seg: Option<&str>, cli_inplace: bool) -> Vec<Step> {
    // Root-фреймворк со своим CLI сам скаффолдит фронтенд-компаньона —
    // отдельные шаги компаньона не нужны (см. root_scaffold_consumes_companion).
    if root_scaffold_consumes_companion(fw, context) {
        return Vec::new();
    }

    let mut steps = steps_for_framework_impl(fw, project_path, project_name, context, seg);

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
        let target_dir = if context.frameworks.iter().any(|f| f == "tauri") {
            ".".to_string()
        } else {
            scaffold_target_dir(fw, seg)
        };
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
    if framework_def(fw).is_some() {
        let scaffold = effective_scaffold(fw, context);
        if scaffold.is_some() && PACKAGE_JSON_SCAFFOLDS.contains(&fw) {
            let workdir: Option<String> = if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw) {
                // package.json лежит в каталоге, куда скаффолдер положил проект
                Some(if context.frameworks.iter().any(|f| f == "tauri") {
                    ".".to_string()
                } else {
                    scaffold_target_dir(fw, seg)
                })
            } else if fw == "tauri" {
                Some(".".to_string())
            } else {
                match scaffold {
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
    let root_present = context.frameworks.iter().any(|fw| is_root_scaffold(fw, context));
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

/// Раскладка tauri-путей в зависимости от сегмента:
///   - seg=None (root-режим, tauri в корне): src-tauri/tauri.conf.json рядом
///     с frontend/ — frontendDist "../frontend/dist", npm --prefix frontend;
///   - seg=Some("backend") (Strict Subdir Mandate): tauri живёт в backend/,
///     конфиг лежит в backend/src-tauri/ — frontendDist "../../frontend/dist",
///     команды npm --prefix ../frontend (cwd tauri-корня = backend/).
/// Возвращает (frontend_dist, before_dev_command, before_build_command).
fn tauri_layout(seg: Option<&str>) -> (String, String, String) {
    if seg == Some("backend") {
        (
            "../../frontend/dist".to_string(),
            "npm --prefix ../frontend run dev".to_string(),
            "npm --prefix ../frontend run build".to_string(),
        )
    } else {
        (
        "dist".to_string(),
        "npm run dev".to_string(),
        "npm run build".to_string(),
        )
    }
}

/// Шаг «scaffold» через Composer: command/args резолвятся через
/// composer_launch() — глобальный `composer` или `php <абс. composer.phar>`,
/// поэтому name_arg вычисляется по фактическому положению плейсхолдера.
fn composer_scaffold_step(
    id: &str,
    label: &str,
    desc: &str,
    package: &str,
) -> Step {
    let (command, prefix) = composer_launch();
    let mut args: Vec<String> = prefix;
    args.extend([
        "create-project".to_string(),
        package.to_string(),
        SCAFFOLD_TARGET.to_string(),
        "--no-interaction".to_string(),
            // Composer's dist downloader requires PHP's zip extension (or
            // unzip/7z). Prefer source so Laravel/Symfony still scaffold on
            // minimal Windows PHP installations.
            "--prefer-source".to_string(),
    ]);
    let name_arg = args
        .iter()
        .position(|a| a == SCAFFOLD_TARGET)
        .expect("SCAFFOLD_TARGET всегда в args composer create-project");
    scaffold_step(id, label, desc, &command, args.iter().map(String::as_str).collect(), name_arg, ".")
}

/// Шаг «scaffold»: CLI-генератор, который сам создаёт папку проекта.
/// ScaffoldGenerator разбирается с каталогом сам (см. generators/mod.rs):
/// ВСЕГДА temp-to-target — CLI выполняется во временной папке temp_<target>,
/// содержимое (включая скрытые файлы) программно переносится в target_dir,
/// временная папка удаляется. Матрёшек testapp/testapp и пустых каркасов
/// без node_modules нет.
#[allow(clippy::too_many_arguments)]
fn scaffold_step(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    name_arg: usize,
    target_dir: &str,
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
        }),
        condition: None,
        on_error: ErrorMode::Skip,
    }
}

/// CRC-32 (IEEE 802.3): poly 0x04C11DB7 (отражённый 0xEDB88320),
/// init/xorout 0xFFFFFFFF — тот же алгоритм, что у std.hash.Crc32 в Zig.
/// Используется для поля `.fingerprint` в build.zig.zon (Zig 0.14+):
/// верхние 32 бита обязаны равняться crc32(имя_пакета).
fn crc32(data: &[u8]) -> u32 {
    const POLY: u32 = 0xEDB88320;
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// Ключевые слова Zig — их нельзя использовать как имя пакета (`.name = .fn`
/// не распарсится). Зарезервированные слова, отсутствующие в этом списке,
/// в имя попасть не могут (список полный для 0.14).
const ZIG_KEYWORDS: &[&str] = &[
    "addrspace", "align", "allowzero", "and", "anyframe", "anytype", "asm",
    "async", "await", "break", "callconv", "catch", "comptime", "const",
    "continue", "defer", "else", "enum", "errdefer", "error", "export",
    "extern", "fn", "for", "if", "inline", "noalias", "noinline", "nosuspend",
    "opaque", "or", "orelse", "packed", "pub", "resume", "return",
    "linksection", "struct", "suspend", "switch", "test", "threadlocal",
    "try", "union", "unreachable", "usingnamespace", "var", "volatile",
    "while",
];

/// Валидное имя пакета Zig из имени проекта: нижний регистр, не-буквенно-
/// цифровые символы → '_', ≤32 байт (ограничение build.zig.zon), не ключевое
/// слово (иначе enum-literal `.name = .<keyword>` не распарсится).
fn zig_package_name(project_name: &str) -> String {
    let mut name: String = project_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    // обрезаем по байтам (имена пакетов — ASCII после санитизации)
    name.truncate(32);
    if name.trim_matches('_').is_empty() {
        name = "app".to_string();
    }
    if name.as_bytes()[0].is_ascii_digit() {
        name.insert(0, '_');
    }
    if ZIG_KEYWORDS.contains(&name.as_str()) {
        name.push('_');
    }
    name
}

/// Экранирование для строковых литералов Java/Kotlin в генерируемом коде
/// (кавычки, бэкслеш, `$` — шаблоны Kotlin).
fn string_literal_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Экранирование для XML-атрибутов (android:label в манифесте).
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Полный каркас Android-приложения (Gradle + MainActivity). Может
/// создаваться фреймворком "android" (java/kotlin, с Compose при связке
/// с jetpack-compose) или самим "jetpack-compose" (kotlin + Compose).
/// Все пути относительны — сегментация (frontend/) применяется движком.
fn android_steps(project_name: &str, compose: bool, java_lang: bool) -> Vec<Step> {
    // Compose доступен только в Kotlin-модуле; при java-языке — обычный
    // Activity (связка android+compose+java не возникает в мастере).
    let compose = compose && !java_lang;
    let safe_name: String = project_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let label = xml_escape(project_name);
    let text = string_literal_escape(project_name);

    let wf = |id: &str, label: &str, path: &str, content: String| -> Step {
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
    };

    // Корневой build.gradle.kts: AGP + Kotlin (и Compose-плагин при
    // compose-проекте). Всё с apply false — приложения подключают плагины
    // в app/build.gradle.kts.
    let root_build = if compose {
        r#"plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
}
"#
        .to_string()
    } else {
        r#"plugins {
    id("com.android.application") version "8.7.3" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
}
"#
        .to_string()
    };

    let app_build = if compose {
        r#"plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
}
"#
        .to_string()
    } else if java_lang {
        r#"plugins {
    id("com.android.application")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
"#
        .to_string()
    } else {
        r#"plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.example.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.example.app"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}
"#
        .to_string()
    };

    // MainActivity: Compose-проект — ComponentActivity + setContent; иначе —
    // обычный Activity с TextView (без внешних зависимостей).
    let (main_path, main_content) = if compose {
        (
            "app/src/main/kotlin/com/example/app/MainActivity.kt".to_string(),
            format!(r#"package com.example.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

class MainActivity : ComponentActivity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)
        setContent {{
            MaterialTheme {{
                Surface(modifier = Modifier.fillMaxSize()) {{
                    Box(contentAlignment = Alignment.Center) {{
                        Text("Hello from {text}!")
                    }}
                }}
            }}
        }}
    }}
}}
"#),
        )
    } else if java_lang {
        (
            "app/src/main/java/com/example/app/MainActivity.java".to_string(),
            format!(r#"package com.example.app;

import android.app.Activity;
import android.os.Bundle;
import android.widget.TextView;

public class MainActivity extends Activity {{
    @Override
    protected void onCreate(Bundle savedInstanceState) {{
        super.onCreate(savedInstanceState);
        TextView textView = new TextView(this);
        textView.setText("Hello from {text}!");
        setContentView(textView);
    }}
}}
"#),
        )
    } else {
        (
            "app/src/main/kotlin/com/example/app/MainActivity.kt".to_string(),
            format!(r#"package com.example.app

import android.app.Activity
import android.os.Bundle
import android.widget.TextView

class MainActivity : Activity() {{
    override fun onCreate(savedInstanceState: Bundle?) {{
        super.onCreate(savedInstanceState)
        val textView = TextView(this)
        textView.text = "Hello from {text}!"
        setContentView(textView)
    }}
}}
"#),
        )
    };

    vec![
        wf("android_settings", "Create settings.gradle.kts", "settings.gradle.kts",
            format!(r#"pluginManagement {{
    repositories {{
        google()
        mavenCentral()
        gradlePluginPortal()
    }}
}}

dependencyResolutionManagement {{
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {{
        google()
        mavenCentral()
    }}
}}

rootProject.name = "{safe_name}"
include(":app")
"#)),
        wf("android_root_build", "Create build.gradle.kts", "build.gradle.kts", root_build),
        wf("android_gradle_props", "Create gradle.properties", "gradle.properties",
            "org.gradle.jvmargs=-Xmx2048m -Dfile.encoding=UTF-8\nandroid.useAndroidX=true\nandroid.nonTransitiveRClass=true\nkotlin.code.style=official\n".to_string()),
        wf("android_app_build", "Create app/build.gradle.kts", "app/build.gradle.kts", app_build),
        wf("android_manifest", "Create AndroidManifest.xml", "app/src/main/AndroidManifest.xml",
            format!(r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application
        android:label="{label}"
        android:theme="@android:style/Theme.Material.Light.NoActionBar">
        <activity
            android:name=".MainActivity"
            android:exported="true">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#)),
        wf("android_main", "Create MainActivity", &main_path, main_content),
    ]
}

/// Раскрывает %VAR% в пути через переменные текущего процесса
/// (неизвестная переменная остаётся как есть).
fn expand_env_path(raw: &str) -> PathBuf {
    let mut out = raw.to_string();
    let mut guard = 0;
    while let Some(start) = out.find('%') {
        if guard > 10 {
            break;
        }
        guard += 1;
        let Some(end_rel) = out[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_rel;
        let name = &out[start + 1..end];
        if let Ok(value) = std::env::var(name) {
            out.replace_range(start..=end, &value);
        } else {
            break;
        }
    }
    PathBuf::from(out)
}

/// Есть ли команда в PATH (where/which)?
fn command_on_path(name: &str) -> bool {
    let (prog, arg) = if cfg!(target_os = "windows") {
        ("where", name)
    } else {
        ("which", name)
    };
    std::process::Command::new(prog)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Способ запуска Composer: (команда, префикс аргументов).
///
/// 1. Глобальный `composer` в PATH — используем его напрямую.
/// 2. Иначе — абсолютный путь к скачанному composer.phar в Toolchain store
///    (`%LOCALAPPDATA%\StackPilot\tools\php\composer.phar`) или в каталогах
///    установки (`%APPDATA%\Composer`, `%LOCALAPPDATA%\Programs\php`) и
///    запуск через `php <абсолютный путь>`.
///
/// Относительный `composer.phar` НЕ используется никогда: `php composer.phar`
/// ищет файл в рабочем каталоге и падает с «Could not open input file:
/// composer.phar» (движок выполняет CLI во временной папке, где phar нет).
fn composer_launch() -> (String, Vec<String>) {
    if command_on_path("composer") {
        return ("composer".to_string(), Vec::new());
    }
    for dir in [
        "%LOCALAPPDATA%\\StackPilot\\tools\\php",
        "%APPDATA%\\Composer",
        "%LOCALAPPDATA%\\Programs\\php",
    ] {
        let phar = expand_env_path(dir).join("composer.phar");
        if phar.is_file() {
            return ("php".to_string(), vec![phar.to_string_lossy().into_owned()]);
        }
    }
    // Ничего не нашли — честный fallback: php с абсолютным путём в Toolchain
    // store (ошибка установки будет явной, а не «Could not open input file»).
    let phar = expand_env_path("%LOCALAPPDATA%\\StackPilot\\tools\\php").join("composer.phar");
    ("php".to_string(), vec![phar.to_string_lossy().into_owned()])
}

fn steps_for_framework_impl(fw: &str, project_path: &str, project_name: &str, context: &WizardContext, seg: Option<&str>) -> Vec<Step> {
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
            // Пайплайн tauri (порядок шагов гарантируется движком):
            //   1. Фронтенд-генератор ПЕРВЫМ: vite скаффолдит frontend/
            //      (компаньон react/vue/svelte или vanilla-ts без компаньона).
            //   2. npm install внутри frontend/ — до tauri init (явный шаг
            //      только когда фронтенд скаффолдит сам tauri; при компаньоне
            //      установку делает финальная фаза после его каркаса).
            //   3. cargo tauri init — frontendDist указывает на frontend/dist
            //      (../frontend/dist из корня, ../../frontend/dist из backend/).
            // create-tauri-app НЕ используется: он скаффолдил фронтенд по
            // шаблону в корне (структура была пустой без node_modules).
            let identifier = tauri_identifier(project_name);
            let has_companion = context.frameworks.iter().any(|f| {
                framework_def("tauri").is_some_and(|def| def.companions.iter().any(|c| c == f))
            });
            let (frontend_dist, dev_cmd, build_cmd) = tauri_layout(seg);
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
                ));
                // npm install внутри frontend/ — до tauri init.
                steps.push(Step::Command {
                    id: "tauri_web_install".into(),
                    label: "Install Tauri frontend dependencies".into(),
                    description: "Run npm install inside frontend/".into(),
                    command: "npm".into(),
                    args: vec!["install".into()],
                    working_dir: Some("frontend".into()),
                    env: None,
                    timeout_secs: Some(600),
                    condition: None,
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
            }
            // cargo tauri init --ci: неинтерактивно, все пути — на frontend/dist.
            steps.push(Step::Command {
                id: "tauri_init".into(),
                label: "Initialize Tauri shell".into(),
                description: "Run cargo tauri init (non-interactive, --ci)".into(),
                // Use the package-local/global npm CLI as a fallback instead
                // of requiring `cargo-tauri` to be preinstalled. `npx --yes`
                // downloads the official CLI when necessary and works on
                // Windows where `cargo tauri` otherwise reports "no such
                // command".
                command: "npx".into(),
                args: vec![
                    "@tauri-apps/cli".into(),
                    "init".into(),
                    "--ci".into(),
                    "--app-name".into(),
                    project_name.into(),
                    "--window-title".into(),
                    project_name.into(),
                    "--frontend-dist".into(),
                    frontend_dist.clone().into(),
                    "--dev-url".into(),
                    "http://localhost:5173".into(),
                    "--before-dev-command".into(),
                    dev_cmd.clone().into(),
                    "--before-build-command".into(),
                    build_cmd.clone().into(),
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
                    "tauri_dir": seg.unwrap_or(""),
                    "frontend_dist": frontend_dist,
                    "dev_url": "http://localhost:5173",
                    "before_dev_command": dev_cmd,
                    "before_build_command": build_cmd,
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
            // При наличии Python-проекта запускаем CLI из его venv. Это
            // устраняет зависимость от глобальной установки Django и
            // согласует команду с ранним django_pip_install в compose_recipe.
            let django_command = if context.languages.iter().any(|l| l == "python") {
                python_venv_bin(project_path, seg.unwrap_or("."), "django-admin")
            } else {
                "django-admin".to_string()
            };
            vec![
                cmd("django_start", "Start Django project", "Create Django project structure",
                    &django_command, vec!["startproject", &safe_name, "."]),
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
        // (frontend/) выбирает ScaffoldGenerator (temp+move: CLI выполняется
        // во временной папке, содержимое переносится в frontend/); npm install
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
                    1, "frontend"),
            ]
        }

        // ==================== JavaScript / TypeScript ====================
        "nextjs" => {
            // create-next-app с --yes работает без интерактива (--typescript/
            // --javascript фиксируют язык, остальное — флагами). Каталог
            // frontend/ выбирает ScaffoldGenerator (temp+move: скаффолд во
            // временной папке, программный перенос в frontend/).
            let ts_flag = if has_typescript { "--typescript" } else { "--javascript" };
            vec![
                scaffold_step("nextjs_create", "Create Next.js app", "Scaffold Next.js project",
                    "npx", vec!["create-next-app@latest", SCAFFOLD_TARGET, ts_flag, "--tailwind", "--eslint", "--app", "--no-src-dir", "--import-alias", "@/*", "--use-npm", "--skip-install", "--yes"],
                    1, "frontend"),
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
                    "npx", sv_args, 2, "frontend"),
            ]
        },

        "nuxt" => {
            // nuxi init неинтерактивен: менеджер пакетов и git фиксируются
            // флагами (--packageManager npm --gitInit false), установка
            // зависимостей откладывается в финальную фазу (--no-install).
            vec![
                scaffold_step("nuxt_create", "Create Nuxt app", "Scaffold Nuxt project",
                    "npx", vec!["--yes", "nuxi@latest", "init", SCAFFOLD_TARGET, "--packageManager", "npm", "--gitInit", "false", "--no-install"],
                    3, "frontend"),
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
            // create-expo-app НЕ принимает "." в качестве имени проекта —
            // ScaffoldGenerator (temp+move) скаффолдит во временную папку и
            // программно переносит содержимое в frontend/.
            let expo_template = if has_typescript {
                "blank-typescript"
            } else {
                "blank"
            };
            vec![
                scaffold_step("expo_init", "Init Expo", "Create Expo project",
                    "npx", vec!["create-expo-app", SCAFFOLD_TARGET, "--yes", "--no-install", "--template", expo_template],
                    1, "frontend"),
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

        "android" => {
            // Настоящий компилируемый каркас Android-приложения (Gradle +
            // MainActivity), а не заглушка. В связке с jetpack-compose
            // android генерирует compose-проект, а jetpack-compose отдаёт
            // пустой план (иначе два фреймворка писали бы одни файлы —
            // duplicate_framework_write_paths).
            let compose = context.frameworks.iter().any(|f| f == "jetpack-compose");
            let java_lang = context.languages.iter().any(|l| l == "java");
            android_steps(project_name, compose, java_lang)
        },

        // ==================== C# ====================
        // -o .: проект создаётся НЕ в вложенной папке <project_name>/,
        // а прямо в рабочей директории (backend/ или frontend/ при
        // сегментации) — иначе aspnetcore + maui давали test16/test16.
        "aspnetcore" => vec![
            cmd("aspnet_new", "Create ASP.NET Core Web API", "Scaffold Web API project",
                "dotnet", vec!["new", "webapi", "-n", project_name, "-o", ".", "--force"]),
        ],

        "maui" => vec![
            cmd("maui_new", "Create MAUI app", "Scaffold .NET MAUI project",
                "dotnet", vec!["new", "maui", "-n", project_name, "-o", ".", "--force"]),
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
        "jetpack-compose" => {
            // С android-фреймворком compose-проект генерирует сам android
            // (см. «android»); standalone — Jetpack Compose — создаёт
            // полноценный compose-проект (kotlin) без Android-фреймворка.
            if context.frameworks.iter().any(|f| f == "android") {
                vec![]
            } else {
                android_steps(project_name, true, false)
            }
        },

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
        "laravel" => {
            // CLI Override (PHP Composer): НИКАКОГО npm. npm-путь
            // (@laravel/installer) падал с «npm error 404 Not Found».
            // Composer запускается с АБСОЛЮТНЫМ путём (composer_launch):
            // глобальный `composer` в PATH или `php <абс. путь к
            // composer.phar> в Toolchain store» — относительный composer.phar
            // не существует в рабочем каталоге CLI. Target подставляет
            // ScaffoldGenerator (temp+move): временная папка → программный
            // перенос в backend/ или корень.
            vec![composer_scaffold_step("laravel_new", "Create Laravel project",
                "Scaffold Laravel application via PHP Composer",
                "laravel/laravel")]
        },

        "symfony" => {
            // CLI Override (PHP Composer): локальный бинарь symfony не
            // требуется, npm не используется — только composer с
            // абсолютным путём (см. composer_launch).
            vec![composer_scaffold_step("symfony_new", "Create Symfony project",
                "Scaffold Symfony application via PHP Composer",
                "symfony/skeleton")]
        },

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
            // Имя пакета — валидный zig-идентификатор (enum literal в zon).
            // Фингерпринт (Zig 0.14+): верхние 32 бита = crc32(имя_пакета),
            // нижние — произвольный id из (0, 0xffffffff). Без совпадающего
            // crc32 `zig build`/`zig fetch` отвергает zon («invalid
            // fingerprint»), а без поля fingerprint вовсе — «missing top-level
            // 'fingerprint' field».
            let pkg_name = zig_package_name(project_name);
            let fingerprint = (u64::from(crc32(pkg_name.as_bytes())) << 32) | 0xCAFE_BABE;
            vec![
                write_file("zap_zon", "Create build.zig.zon", "build.zig.zon",
                    &format!(r#".{{
    .name = .{pkg_name},
    .version = "0.1.0",
    .minimum_zig_version = "0.14.0",
    .paths = .{{""}},
    .fingerprint = 0x{fingerprint:016x},
    .dependencies = .{{}},
}}
"#)),
                // `zig init` оставляет build.zig без модуля zap — проект с
                // @import("zap") в main.zig не собрался бы. Переписываем
                // build.zig (inplace-фреймворк → overwrite=true) с
                // подключением зависимости.
                write_file("zap_build", "Create build.zig", "build.zig",
                    &format!(r#"const std = @import("std");

pub fn build(b: *std.Build) !void {{
    const target = b.standardTargetOptions(.{{}});
    const optimize = b.standardOptimizeOption(.{{}});

    const zap = b.dependency("zap", .{{
        .target = target,
        .optimize = optimize,
    }});

    const exe = b.addExecutable(.{{
        .name = "{}",
        .root_module = b.createModule(.{{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{{
                .{{ .name = "zap", .module = zap.module("zap") }},
            }},
        }}),
    }});

    b.installArtifact(exe);
}}
"#, project_name)),
                cmd("zap_fetch", "Add Zap dependency", "Fetch zap and save to build.zig.zon",
                    "zig", vec!["fetch", "--save", "https://github.com/zigzap/zap/archive/refs/tags/v0.10.1.tar.gz"]),
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

/// АБСОЛЮТНЫЙ путь к бинарю внутри venv проекта:
/// `<project_path>\venv\Scripts\<name>.exe` на Windows,
/// `<project_path>/venv/bin/<name>` на unix. В моно-репозитории venv живёт
/// в каталоге python-сегмента (`<project_path>\backend\venv\Scripts\alembic.exe`
/// / `<project_path>/backend/venv/bin/alembic`), а не в корне.
///
/// Правило движка: Python-утилиты (alembic, pip) вызываются ТОЛЬКО через
/// бинарники виртуального окружения — глобальный `alembic` не используется.
/// Путь обязан быть АБСОЛЮТНЫМ: относительный `<project>/backend/venv/...`
/// зависел бы от рабочего каталога процесса (executor запускает команды из
/// project_path, но venv создаётся ВНУТРИ каталога python-сегмента).
fn python_venv_bin(project_path: &str, python_dir: &str, name: &str) -> String {
    let base = PathBuf::from(project_path);
    let venv = if python_dir.is_empty() || python_dir == "." {
        base.join("venv")
    } else {
        base.join(python_dir).join("venv")
    };
    let bin = if cfg!(target_os = "windows") {
        venv.join("Scripts").join(format!("{}.exe", name))
    } else {
        venv.join("bin").join(name)
    };
    bin.to_string_lossy().into_owned()
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
    // Для Django базовый пакет ставится до CLI-скаффолда, но полная
    // requirements-фаза всё равно нужна ПОСЛЕ framework-шагов: именно там
    // aiogram/SQLAlchemy и прочие генераторы дописывают requirements.txt.
    // Повторный вызов `python -m venv` безопасен и лишь переиспользует готовое
    // окружение, зато устраняет гонку порядка шагов.
    let needs_python_venv = has_python && tools.iter().any(|t| t == "alembic");
    // Каталог python-кода (backend/ в моно-репозитории, иначе корень):
    // venv создаётся ВНУТРИ него, рядом с requirements.txt. Вычисляется
    // вне блока — нужен и шагам alembic ниже.
    let python_dir = python_segment_dir(context);
    if needs_python_venv {
        // Абсолютный путь к venv: python -m venv выполняется из project_path,
        // но каталог окружения обязан лежать ТОЧНО внутри python-сегмента
        // (backend/venv или ./venv) — относительный путь в шаге + другой
        // working_dir давали venv не там, где его ищут pip/alembic.
        let venv_path = if python_dir == "." {
            PathBuf::from(project_path).join("venv")
        } else {
            PathBuf::from(project_path).join(&python_dir).join("venv")
        };
        let venv_str = venv_path.to_string_lossy().into_owned();
        let req_path = if python_dir == "." {
            PathBuf::from(project_path).join("requirements.txt")
        } else {
            PathBuf::from(project_path).join(&python_dir).join("requirements.txt")
        };
        let req_str = req_path.to_string_lossy().into_owned();
        steps.push(Step::Command {
            id: "py_venv_create".into(),
            label: "Create Python virtual environment".into(),
            description: format!("Run python -m venv {}", venv_str),
            command: "python".into(),
            args: vec!["-m".into(), "venv".into(), venv_str],
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
            // Абсолютный путь: команда исполняется из project_path, но venv
            // живёт внутри python-сегмента (backend/venv).
            command: python_venv_bin(project_path, &python_dir, "pip"),
            args: vec!["install".into(), "-r".into(), req_str, "alembic".into()],
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
                    // (абсолютный путь: <project>\backend\venv\Scripts\alembic.exe
                    // / <project>/backend/venv/bin/alembic внутри python-
                    // сегмента): py_pip_install (с гарантированным alembic)
                    // отрабатывает ДО этого шага — см. выше.
                    command: python_venv_bin(project_path, &python_dir, "alembic"),
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
    fn crc32_matches_zig_std_hash() {
        // Значения сверены с `std.hash.Crc32.hash(...)` (Zig 0.16.0) — те же,
        // что Zig использует для fingerprint в build.zig.zon.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"my_app"), 0x542B_89B0);
        assert_eq!(crc32(b"other_name"), 0x3767_28D4);
        assert_eq!(crc32(b"ziginit"), 0x36AD_9EBC);
    }

    #[test]
    fn zig_package_name_is_valid_identifier() {
        assert_eq!(zig_package_name("my-app"), "my_app");
        assert_eq!(zig_package_name("My App"), "my_app");
        assert_eq!(zig_package_name("myApp"), "myapp");
        // имя-ключевое слово получает суффикс (`.name = .fn` не парсится)
        assert_eq!(zig_package_name("fn"), "fn_");
        assert_eq!(zig_package_name("switch"), "switch_");
        // не начинается с цифры, не пустое, не длиннее 32 байт
        assert_eq!(zig_package_name("123"), "_123");
        assert_eq!(zig_package_name("!!!"), "app");
        let long = zig_package_name(&"a".repeat(50));
        assert!(long.len() <= 32, "{long}");
    }

    #[test]
    fn zap_zon_has_valid_fingerprint_and_paths() {
        // Зон с interpolated-именем: fingerprint обязан содержать crc32 имени
        // в верхних 32 битах (Zig 0.14+), paths — присутствовать.
        let steps = steps_for_framework("zap", "C:\\dev\\myapp", "my_app", &context(), None, false);
        let zon = steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "build.zig.zon"))
            .unwrap_or_else(|| panic!("должен быть шаг записи build.zig.zon"));
        match zon {
            Step::WriteFile { content, .. } => {
                let pkg_name = zig_package_name("my_app");
                let expected = format!(
                    ".name = .{pkg_name},\n    .version = \"0.1.0\",\n    .minimum_zig_version = \"0.14.0\",\n    .paths = .{{\"\"}},\n    .fingerprint = 0x{:016x},",
                    (u64::from(crc32(pkg_name.as_bytes())) << 32) | 0xCAFE_BABE
                );
                assert!(content.contains(&expected), "zon: {content}");
            }
            other => panic!("ожидался WriteFile, получили {:?}", other.id()),
        }
    }

    #[test]
    fn zap_fetch_uses_zap_v0101() {
        let steps = steps_for_framework("zap", "C:\\dev\\myapp", "my_app", &context(), None, false);
        let fetch = steps
            .iter()
            .find(|s| matches!(s, Step::Command { id, .. } if id == "zap_fetch"))
            .unwrap_or_else(|| panic!("должен быть шаг zap_fetch"));
        let args = cmd_args(fetch);
        assert!(
            args.iter().any(|a| a.contains("v0.10.1.tar.gz")),
            "zap_fetch должен ссылаться на v0.10.1 (старые теги не парсятся Zig 0.13+): {args:?}"
        );
    }

    fn android_context(frameworks: &[&str], languages: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn write_paths(steps: &[Step]) -> Vec<&str> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::WriteFile { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    fn write_content<'a>(steps: &'a [Step], path: &str) -> &'a str {
        steps
            .iter()
            .find_map(|s| match s {
                Step::WriteFile { path: p, content, .. } if p == path => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("нет шага записи {path}"))
    }

    #[test]
    fn android_generates_full_gradle_project() {
        let ctx = android_context(&["android"], &["kotlin"]);
        let steps = steps_for_framework("android", "C:\\dev\\myapp", "myapp", &ctx, None, false);
        for expected in [
            "settings.gradle.kts",
            "build.gradle.kts",
            "gradle.properties",
            "app/build.gradle.kts",
            "app/src/main/AndroidManifest.xml",
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        ] {
            assert!(write_paths(&steps).contains(&expected), "нет файла {expected}");
        }
        // без jetpack-compose — никакого compose-плагина и compose-импортов
        assert!(!write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(&steps, "app/src/main/kotlin/com/example/app/MainActivity.kt");
        assert!(!main.contains("androidx.compose"), "{main}");
        assert!(main.contains("android.app.Activity"), "{main}");
        // манифест: метка проекта экранирована, тема — платформенная (без res/)
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(manifest.contains("android:label=\"myapp\""), "{manifest}");
    }

    #[test]
    fn android_with_java_language_generates_java_activity() {
        let ctx = android_context(&["android"], &["java"]);
        let steps = steps_for_framework("android", "C:\\dev\\myapp", "myapp", &ctx, None, false);
        let main = write_content(&steps, "app/src/main/java/com/example/app/MainActivity.java");
        assert!(main.contains("public class MainActivity extends Activity"), "{main}");
        let app_build = write_content(&steps, "app/build.gradle.kts");
        assert!(!app_build.contains("org.jetbrains.kotlin"), "{app_build}");
    }

    #[test]
    fn android_with_jetpack_compose_generates_compose_project() {
        let ctx = android_context(&["android", "jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework("android", "C:\\dev\\myapp", "myapp", &ctx, None, false);
        let root = write_content(&steps, "build.gradle.kts");
        assert!(root.contains("org.jetbrains.kotlin.plugin.compose"), "{root}");
        let main = write_content(&steps, "app/src/main/kotlin/com/example/app/MainActivity.kt");
        assert!(main.contains("setContent"), "{main}");
        assert!(main.contains("androidx.compose.material3"), "{main}");
        // jetpack-compose рядом с android не пишет свои файлы (иначе —
        // дубликаты путей, см. duplicate_framework_write_paths)
        let compose_steps = steps_for_framework("jetpack-compose", "C:\\dev\\myapp", "myapp", &ctx, None, false);
        assert!(compose_steps.is_empty(), "{compose_steps:?}");
    }

    #[test]
    fn jetpack_compose_standalone_generates_full_project() {
        let ctx = android_context(&["jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework("jetpack-compose", "C:\\dev\\myapp", "myapp", &ctx, None, false);
        assert!(write_paths(&steps).contains(&"app/build.gradle.kts"));
        assert!(write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(&steps, "app/src/main/kotlin/com/example/app/MainActivity.kt");
        assert!(main.contains("setContent"), "{main}");
    }

    #[test]
    fn android_files_escape_user_text() {
        let ctx = WizardContext {
            project_name: Some("My \"App\" $v1".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: vec!["kotlin".into()],
            frameworks: vec!["android".into()],
            ..Default::default()
        };
        let steps = steps_for_framework("android", "C:\\dev\\myapp", "My \"App\" $v1", &ctx, None, false);
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(manifest.contains("android:label=\"My &quot;App&quot; $v1\""), "{manifest}");
        let main = write_content(&steps, "app/src/main/kotlin/com/example/app/MainActivity.kt");
        assert!(main.contains("Hello from My \\\"App\\\" \\$v1!"), "{main}");
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
    fn aspnetcore_plus_maui_isolated_in_segments_no_matryoshka() {
        // ASP.NET Core (backend) + MAUI (frontend): оба на csharp (category
        // "both"), стороны дают side фреймворков (aspnetcore=backend,
        // maui=frontend). dotnet new webapi/maui выполняются ВНУТРИ своих
        // сегментов с -o . — проект ложится прямо в backend/ или frontend/,
        // без вложенной папки test16/test16.
        let mut ctx = context();
        ctx.languages = vec!["csharp".into()];
        ctx.frameworks = vec!["aspnetcore".into(), "maui".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // Обе стороны: backend/ и frontend/ создаются движком
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "aspnetcore обязан получить backend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "maui обязан получить frontend/: {mkdirs:?}");

        // dotnet new webapi: работает в backend/ с -o . — без вложенной папки
        let asp = recipe.steps.iter().find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command { args, working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "webapi обязан работать внутри ./backend");
                assert!(args.contains(&"-o".to_string()) && args.contains(&".".to_string()),
                    "webapi обязан идти с -o . (проект прямо в backend/, без test16/test16): {args:?}");
                assert!(args.contains(&"-n".to_string()), "{args:?}");
            }
            _ => panic!("aspnet_new — Command"),
        }

        // dotnet new maui: работает в frontend/ с -o .
        let maui = recipe.steps.iter().find(|s| s.id() == "maui_new")
            .expect("maui_new должен быть в плане");
        match maui {
            Step::Command { args, working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"),
                    "maui обязан работать внутри ./frontend");
                assert!(args.contains(&"-o".to_string()) && args.contains(&".".to_string()),
                    "maui обязан идти с -o . (проект прямо в frontend/, без test16/test16): {args:?}");
            }
            _ => panic!("maui_new — Command"),
        }

        // Generic csharp-скаффолд (dotnet new console) подавлен обоими
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "dotnet_new"),
            "dotnet new console не нужен: каркасы создают aspnetcore/maui"
        );
    }

    #[test]
    fn django_sanitizes_name_and_isolates_in_backend() {
        // Django-фикс: django-admin startproject получает санитизированное
        // имя (дефис → подчёркивание — Python-пакет), а при обеих сторонах
        // работает в backend/ с "." — manage.py не появляется в корне.
        let mut ctx = context();
        ctx.project_name = Some("my-test-app".into());
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];

        let recipe = compose_recipe(&ctx, "my-test-app").expect("recipe must build");

        let django = recipe.steps.iter().find(|s| s.id() == "django_start")
            .expect("django_start должен быть в плане");
        match django {
            Step::Command { args, working_dir, .. } => {
                assert!(args.contains(&"my_test_app".to_string()),
                    "имя проекта санитизируется (дефис → подчёркивание): {args:?}");
                assert!(args.contains(&".".to_string()),
                    "startproject создаёт проект в текущем каталоге: {args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)");
            }
            _ => panic!("django_start — Command"),
        }
    }

    #[test]
    fn tauri_pipeline_scaffolds_frontend_init_and_patches_config() {
        // Strict Subdir Mandate: rust (backend) + typescript (frontend) — обе
        // стороны, tauri (scaffold="root" в JSON) принудительно работает как
        // subdir. React скаффолдится в frontend/ отдельным scaffold-шагом,
        // tauri живёт в backend/; затем cargo tauri init (пути на
        // ../../frontend/dist) и Rust-патч backend/src-tauri/tauri.conf.json.
        // create-tauri-app убран из пайплайна (его фронтенд-каркас в корне
        // был пустым без node_modules).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.backend_languages = vec!["rust".into()];
        ctx.frontend_languages = vec!["typescript".into()];
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
        // Мандат: backend/ И frontend/ создаются движком
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "нужны backend/ и frontend/: {mkdirs:?}");
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

        // Фронтенд-генератор выполняется ПЕРВЫМ — tauri init откладывается
        // в конец фазы скаффолдинга (движок откладывает tauri-шаги).
        let vite_idx = recipe.steps.iter().position(|s| s.id() == "vite_create").unwrap();
        let init_idx = recipe.steps.iter().position(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        assert!(vite_idx < init_idx, "фронтенд скаффолдится ДО tauri init");
        match &recipe.steps[init_idx] {
            Step::Command { command, args, working_dir, .. } => {
                assert_eq!(command, "cargo");
                assert!(args.iter().any(|a| a == "tauri"), "{args:?}");
                assert!(args.iter().any(|a| a == "--ci"), "init должен быть неинтерактивным: {args:?}");
                let dist_idx = args.iter().position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(args[dist_idx + 1], "../../frontend/dist",
                    "из backend/ путь на frontend/dist — ../../frontend/dist: {args:?}");
                assert!(args.iter().any(|a| a == "--before-dev-command"), "{args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "tauri init работает в backend/ (Strict Subdir Mandate)");
            }
            _ => panic!("tauri_init — Command"),
        }

        // Rust-патч tauri.conf.json — сразу после tauri init, внутри backend/
        let patch_idx = recipe.steps.iter().position(|s| s.id() == "tauri_config_patch")
            .expect("tauri_config_patch должен быть в плане");
        assert!(init_idx < patch_idx, "патч конфига идёт после tauri init");
        match &recipe.steps[patch_idx] {
            Step::Generate { generator_id, generator_config, on_error, .. } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(generator_config.get("identifier").and_then(|v| v.as_str()), Some("com.myapp"));
                assert_eq!(generator_config.get("frontend_dir").and_then(|v| v.as_str()), Some("frontend"));
                assert_eq!(generator_config.get("tauri_dir").and_then(|v| v.as_str()), Some("backend"),
                    "конфиг живёт в backend/src-tauri/: {generator_config}");
                assert_eq!(generator_config.get("frontend_dist").and_then(|v| v.as_str()), Some("../../frontend/dist"),
                    "frontendDist из backend/ — ../../frontend/dist: {generator_config}");
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // При компаньоне vite-vanilla-скаффолд и явный npm install tauri не нужны
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_web_scaffold"),
            "компаньон сам скаффолдит frontend/"
        );
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "tauri_web_install"),
            "компаньон: npm install делает финальная фаза"
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
        // npm install внутри frontend/, затем cargo tauri init в backend/
        // (rust+typescript — обе стороны, Strict Subdir Mandate). Generic
        // js/ts-скаффолд в корне подавлен (его заглушки конфликтовали бы
        // с tauri-каркасом).
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

        // npm install — явный шаг внутри frontend/ (компаньона нет, финальная
        // фаза про tauri-фронтенд не знает)
        let install = recipe.steps.iter().find(|s| s.id() == "tauri_web_install")
            .expect("tauri_web_install должен быть в плане");
        match install {
            Step::Command { working_dir, command, .. } => {
                assert_eq!(command, "npm");
                assert_eq!(working_dir.as_deref(), Some("frontend"),
                    "install работает внутри frontend/ без join-сегмента: {working_dir:?}");
            }
            _ => panic!("tauri_web_install — Command"),
        }

        // Фронтенд-генератор → npm install → tauri init → патч конфига
        let idx = |id: &str| recipe.steps.iter().position(|s| s.id() == id)
            .unwrap_or_else(|| panic!("{id} должен быть в плане"));
        assert!(idx("tauri_web_scaffold") < idx("tauri_web_install"), "скаффолд до install");
        assert!(idx("tauri_web_install") < idx("tauri_init"), "install до tauri init");

        match &recipe.steps[idx("tauri_init")] {
            Step::Command { command, args, working_dir, .. } => {
                assert_eq!(command, "cargo");
                let dist_idx = args.iter().position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(args[dist_idx + 1], "../../frontend/dist",
                    "из backend/ путь на frontend/dist — ../../frontend/dist: {args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "tauri init работает в backend/ (Strict Subdir Mandate)");
            }
            _ => panic!("tauri_init — Command"),
        }

        match &recipe.steps[idx("tauri_config_patch")] {
            Step::Generate { generator_id, generator_config, on_error, .. } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(generator_config.get("tauri_dir").and_then(|v| v.as_str()), Some("backend"));
                assert_eq!(generator_config.get("frontend_dist").and_then(|v| v.as_str()), Some("../../frontend/dist"));
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // Финальная фаза про tauri-фронтенд не знает: install уже сделан
        // явным шагом, новых npm_install в финале нет
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 0, "фронтенд tauri ставится явным шагом, финальных npm_install быть не должно");

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
    fn root_scaffold_forced_to_subdir_still_runs_first() {
        // Strict Subdir Mandate: python (backend) + typescript (frontend) —
        // обе стороны, поэтому django (scaffold="root" в wizard_tree.json)
        // принудительно работает как subdir: бэкенд в backend/, фронтенд в
        // frontend/. Порядок фаз сохраняется: 1. Root CLI (django) →
        // 2. Subdir (react) → 3. Инструменты (prisma) → 4. Конфиги
        // (docker-compose).
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        ctx.tools = vec!["prisma".into(), "postgresql".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "django (root→subdir) получает backend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "react получает frontend/: {mkdirs:?}");

        let idx = |id: &str| recipe.steps.iter().position(|s| s.id() == id)
            .unwrap_or_else(|| panic!("{id} должен быть в плане"));
        assert!(idx("django_start") < idx("vite_create"), "root CLI идёт до subdir-скаффолда");
        assert!(idx("vite_create") < idx("prisma_init"), "subdir-скаффолд идёт до инструментов");
        assert!(idx("prisma_init") < idx("docker_compose"), "инструменты идут до конфигов");

        // django-admin startproject работает в backend/, а не в корне
        match &recipe.steps[idx("django_start")] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)");
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
    fn nest_plus_nextjs_decoupled_twin_isolates_backend_and_frontend() {
        // Strict Decoupled Twin Rule: nest (backend, scaffold="root" в JSON) +
        // nextjs (frontend) — обе стороны (nest = side=backend, nextjs =
        // side=frontend даже при единственном языке typescript), поэтому
        // nest принудительно работает как subdir: бэкенд целиком в backend/
        // (nest-cli.json, tsconfig.build.json, package.json — только там),
        // фронтенд — в frontend/. В корне нет package.json и node_modules —
        // только оркестрационные файлы движка.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "nextjs".into()];

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком ДО запуска CLI
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "nest обязан получить backend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "nextjs обязан получить frontend/: {mkdirs:?}");
        let create_idx = |p: &str| recipe.steps.iter()
            .position(|s| matches!(s, Step::CreateDirectory { path, .. } if path == p))
            .unwrap_or_else(|| panic!("{p}/ должен создаваться движком"));
        let nest_idx = recipe.steps.iter().position(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        assert!(create_idx("backend") < nest_idx, "backend/ создаётся до запуска nest");
        assert!(create_idx("frontend") < recipe.steps.iter().position(|s| s.id() == "nextjs_create").unwrap(),
            "frontend/ создаётся до запуска nextjs");

        // nest: "." + --skip-install + --skip-git, работает ВНУТРИ backend/
        let nest = recipe.steps.iter().find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        match nest {
            Step::Command { args, working_dir, .. } => {
                assert_eq!(args.get(2).map(String::as_str), Some("."), "{args:?}");
                assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
                assert!(args.contains(&"--skip-git".to_string()), "git инициализирует движок: {args:?}");
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/backend"),
                    "nest обязан работать в backend/, а не в корне (Decoupled Twin)");
            }
            _ => panic!("nest_new — Command"),
        }

        // nextjs: ScaffoldGenerator выполняет create-next-app ВНУТРИ frontend/
        // (--skip-install — зависимости в финальной фазе)
        let next = recipe.steps.iter().find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
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

        // npm install ровно 2 раза: backend (nest) + frontend (nextjs) —
        // корневой install НЕ появляется (в корне нет package.json)
        let installs: Vec<_> = recipe.steps.iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 2, "install-шагов должно быть 2: {installs:?}");
        match (&installs[0], &installs[1]) {
            (Step::Command { working_dir: w0, .. }, Step::Command { working_dir: w1, .. }) => {
                assert_eq!(w0.as_deref(), Some("C:\\dev\\myapp/backend"), "nest-бэкенд ставится первым");
                assert_eq!(w1.as_deref(), Some("C:\\dev\\myapp/frontend"), "nextjs-фронтенд ставится вторым");
            }
            _ => panic!("npm_install — Command"),
        }

        // В корне нет generic js-скаффолда: package.json пишется только
        // nest'ом (в backend/) и nextjs (в frontend/)
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "package_json"),
            "package.json не должен создаваться в корне"
        );
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
    fn complex_dual_side_stack_segments_backend_and_frontend() {
        // python + typescript на бэкенд-стороне + react (frontend-фреймворк,
        // side=frontend) — обе стороны, поэтому сегментация ВКЛЮЧЕНА:
        // fastapi живёт в backend/, vite-скаффолд — в frontend/. Корневые
        // файлы оркестрации (dags/, docker-compose, .env.example) — в корне.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into(), "typescript".into()];
        ctx.frontend_languages = vec![];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["airflow".into(), "postgresql".into()];
        ctx.docker = true;

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        // react (frontend-фреймворк) даёт обе стороны → backend/ и frontend/
        let mkdirs: Vec<String> = recipe.steps.iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(mkdirs.contains(&"backend".to_string()), "fastapi должен получить backend/: {mkdirs:?}");
        assert!(mkdirs.contains(&"frontend".to_string()), "react должен получить frontend/: {mkdirs:?}");

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
                    "vite-проект живёт в frontend/: {generator_config}");
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install выполняется РОВНО один раз в финальной фазе пайплайна —
        // ВНУТРИ frontend/
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

        // fastapi (inplace): entry-файлы в backend/
        let main = recipe.steps.iter().find(|s| s.id() == "fastapi_main")
            .expect("fastapi_main должен быть в плане");
        match main {
            Step::WriteFile { path, .. } => assert_eq!(path, "backend/src/main.py"),
            _ => panic!("fastapi_main — WriteFile"),
        }

        // airflow: dags/ директория + пример DAG — в корне (оркестрация)
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
        // Корень проекта: <project>\venv\Scripts\alembic.exe (Win) /
        // <project>/venv/bin/alembic.
        let root_bin = python_venv_bin("C:\\dev\\myapp", ".", "alembic");
        assert!(
            root_bin.contains("venv") && !root_bin.contains("backend"),
            "корневой venv без сегмента: {root_bin}"
        );
        assert!(
            root_bin.ends_with("alembic.exe") || root_bin.ends_with("/alembic"),
            "имя бинаря на конце: {root_bin}"
        );
        assert!(
            root_bin.starts_with("C:\\dev\\myapp"),
            "путь АБСОЛЮТНЫЙ (не зависит от рабочего каталога): {root_bin}"
        );
        // Моно-репозиторий: <project>\backend\venv\Scripts\alembic.exe (Win)
        // / <project>/backend/venv/bin/alembic.
        let seg_bin = python_venv_bin("C:\\dev\\myapp", "backend", "alembic");
        assert!(
            seg_bin.contains("backend") && seg_bin.contains("venv"),
            "бинарь внутри backend/venv: {seg_bin}"
        );
        assert!(
            seg_bin.starts_with("C:\\dev\\myapp"),
            "путь АБСОЛЮТНЫЙ: {seg_bin}"
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
        // qt-webengine + react: react (side=frontend) + cpp (backend) — обе
        // стороны, поэтому qt (side=either, язык cpp → backend/) живёт в
        // backend/. main.cpp обязан содержать QWebEngineView-бойлерплейт,
        // CMakeLists.txt — WebEngineWidgets (не заглушки).
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "react".into()];
        ctx.answers.insert("qt_ui".into(), vec!["qt-webengine".into()]);

        let recipe = compose_recipe(&ctx, "myapp").expect("recipe must build");

        let main_cpp = recipe
            .steps
            .iter()
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "backend/src/main.cpp"))
            .unwrap_or_else(|| panic!("qt должен писать backend/src/main.cpp"));
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
            .find(|s| matches!(s, Step::WriteFile { path, .. } if path == "backend/CMakeLists.txt"))
            .unwrap_or_else(|| panic!("qt должен писать backend/CMakeLists.txt"));
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
        // `composer create-project ... --no-interaction --prefer-dist`:
        // запускается глобальный `composer` из PATH ИЛИ `php <АБСОЛЮТНЫЙ
        // путь к composer.phar>` (composer_launch — Toolchain store
        // %LOCALAPPDATA%\StackPilot\tools\php, %APPDATA%\Composer...).
        // Относительный `composer.phar` запрещён: php ищет его в рабочем
        // каталоге CLI и падает с «Could not open input file: composer.phar».
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
                    let command = generator_config.get("command").and_then(|v| v.as_str())
                        .expect("command обязан быть");
                    assert!(
                        command == "composer" || command == "php",
                        "composer запускается как composer или php, а не npm-клиент: {generator_config}"
                    );
                    let args = generator_config.get("args").and_then(|a| a.as_array())
                        .cloned().unwrap_or_default();
                    let name_arg = generator_config.get("name_arg").and_then(|v| v.as_u64())
                        .expect("name_arg обязан быть") as usize;

                    // create-project идёт сразу после префикса (путь к phar
                    // в режиме php, ничего в режиме composer), за ним —
                    // пакет, а плейсхолдер target стоит на name_arg.
                    let cp_idx = args.iter().position(|a| a.as_str() == Some("create-project"))
                        .expect("create-project обязан быть в args");
                    if command == "php" {
                        let phar = args.get(0).and_then(|v| v.as_str()).unwrap_or_default();
                        assert!(
                            phar.ends_with("composer.phar"),
                            "php-режим: первым аргументом — АБСОЛЮТНЫЙ путь к composer.phar: {args:?}"
                        );
                        assert_ne!(
                            phar, "composer.phar",
                            "относительный composer.phar запрещён (рабочий каталог CLI ≠ каталог phar): {args:?}"
                        );
                        assert_eq!(cp_idx, 1, "php <phar> create-project ...: {args:?}");
                    } else {
                        assert_eq!(cp_idx, 0, "composer create-project ...: {args:?}");
                    }
                    assert_eq!(cp_idx + 2, name_arg,
                        "имя проекта — аргумент сразу после пакета create-project: {generator_config}");
                    assert_eq!(
                        args.get(name_arg).and_then(|v| v.as_str()), Some("__TARGET__"),
                        "плейсхолдер стоит на name_arg: {generator_config}"
                    );
                    assert_eq!(args.get(cp_idx + 1).and_then(|v| v.as_str()), Some(package),
                        "пакет сразу после create-project: {generator_config}");
                    assert!(args.iter().any(|a| a == "--no-interaction"), "{args:?}");
                    assert!(args.iter().any(|a| a == "--prefer-dist"), "{args:?}");
                    assert_eq!(generator_config.get("target_dir").and_then(|v| v.as_str()), Some("."),
                        "в монолите PHP-фреймворк живёт в корне");
                }
                _ => panic!("{step_id} — Generate"),
            }
        }
    }
}
