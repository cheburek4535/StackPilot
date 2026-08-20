pub mod content;
pub mod executor;
pub mod paths;
pub mod preflight;
pub mod process;
pub mod template;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use chrono::Local;
use tokio::sync::mpsc;

use crate::modules::project_creator::generators::GeneratorRegistry;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::*;

/// Полный движок рецептов:
///   1. plan() — составить план выполнения на основе WizardContext
///   2. execute() — выполнить план с отправкой событий
#[async_trait::async_trait]
pub trait RecipeEngine: Send + Sync {
    /// Составить ExecutionPlan (что делаем, в каком порядке)
    fn plan(&self, context: &WizardContext, project_path: &Path) -> Result<ExecutionPlan, String>;

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

/// Файл-цель шага уже существует в проекте (реальная проверка fs, пути
/// трактуются как относительные к корню проекта). Возвращает true только
/// для WriteFile/Generate — шагов, чья идемпотентность зависит от файла.
fn step_existing_file(plan: &ExecutionPlan, step: &Step) -> bool {
    match step {
        Step::WriteFile { path, .. } => paths::resolve_in_root(&plan.project_path, path)
            .map(|p| p.exists())
            .unwrap_or(false),
        Step::Generate {
            generator_id,
            generator_config,
            ..
        } => {
            if generator_id != "scaffold" {
                return false;
            }
            let outputs = generator_config
                .get("expected_outputs")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if outputs.is_empty() {
                return false;
            }
            let target_dir = generator_config
                .get("target_dir")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            let Some(target) = (if target_dir == "." {
                Some(plan.project_path.clone())
            } else {
                paths::resolve_in_root(&plan.project_path, target_dir)
            }) else {
                return false;
            };
            outputs.iter().all(|output| {
                paths::resolve_in_root(&target, output)
                    .map(|p| p.exists())
                    .unwrap_or(false)
            })
        }
        _ => false,
    }
}

/// Для WriteFile с политикой SkipIfExists/CreateOnly и существующей целью —
/// шаг в предпросмотре помечается невыполняемым.
fn step_skipped_by_scaffold_outputs(
    plan: &ExecutionPlan,
    step: &Step,
) -> Option<(bool, Option<String>)> {
    match step.file_policy() {
        Some(FilePolicy::SkipIfExists) | Some(FilePolicy::CreateOnly) => {
            if step_existing_file(plan, step) {
                return Some((true, None));
            }
            None
        }
        _ => None,
    }
}

#[async_trait::async_trait]
impl RecipeEngine for DefaultRecipeEngine {
    fn plan(&self, context: &WizardContext, project_path: &Path) -> Result<ExecutionPlan, String> {
        let folder_name = project_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("app");
        // Каноническая раскладка проекта вычисляется ОДИН раз и пронизывает
        // compose_recipe (каталоги фреймворков/языков), предпросмотр (LayoutSummary)
        // и шаблонизацию (README, Docker). Никаких повторных эвристик.
        let layout = ProjectLayout::compute(context);
        let recipe = compose_recipe(&layout, context, folder_name)?;
        // context.project_name (если задан) будет использован внутри compose_recipe
        let steps = flatten_and_filter(&recipe, context, project_path);
        // Явные предусловия: декларации рецепта сужаются до шагов, реально
        // оставшихся после фильтрации условий (ветки, отсечённые контекстом,
        // не порождают «висячих» предшественников), затем план нормализуется
        // стабильной топологической сортировкой: рецепт может декларировать
        // шаги в любом порядке — предшественники всегда встают строго до
        // зависимых, независимые шаги сохраняют порядок декларации. Зависимости
        // НЕ выводятся из порядка шагов — только из деклараций
        // (Recipe.dependencies).
        let dependencies = build_dependencies(&steps, &recipe.dependencies);
        let steps = topo_order_steps(&steps, &dependencies)?;
        Ok(ExecutionPlan {
            recipe,
            context: context.clone(),
            project_path: project_path.to_path_buf(),
            steps,
            dependencies,
            layout_summary: layout.to_summary(context),
        })
    }

    fn preview(&self, plan: &ExecutionPlan) -> RecipePreview {
        let mut previews = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            let action = match step {
                Step::Command { command, .. } => format!("$ {}", command),
                Step::WriteFile {
                    path,
                    policy,
                    overwrite,
                    ..
                } => {
                    let effective = policy.unwrap_or(if *overwrite {
                        FilePolicy::Overwrite
                    } else {
                        FilePolicy::SkipIfExists
                    });
                    match effective {
                        FilePolicy::MergeJson => format!("merge json {}", path),
                        _ => format!("write {}", path),
                    }
                }
                // Step::RenderTemplate { path, .. } => format!("render {}", path),
                Step::CreateDirectory { path, .. } => format!("mkdir {}", path),
                Step::Generate { generator_id, .. } => format!("generate[{}]", generator_id),
                Step::Parallel { steps, .. } => format!("parallel ({} steps)", steps.len()),
            };
            // Реальный fs-статус файлов проекта (если проект уже существует):
            // предпросмотр показывает, что шаг фактически пропустится из-за
            // политики идемпотентности (skip_if_exists/create_only) или
            // состояния, созданного прошлым запуском.
            let existing_file = step_existing_file(plan, step);
            let file_policy = step.file_policy();
            let mut will_execute = true;
            let mut skip_reason: Option<String> = None;
            if existing_file {
                match file_policy {
                    Some(FilePolicy::SkipIfExists) | Some(FilePolicy::CreateOnly) => {
                        will_execute = false;
                        skip_reason = Some(format!(
                            "File already exists — step will be skipped (policy {:?})",
                            file_policy.unwrap()
                        ));
                    }
                    _ => {}
                }
            } else if let Some((skip, reason)) = step_skipped_by_scaffold_outputs(plan, step) {
                will_execute = !skip;
                skip_reason = reason;
            }
            // Зависимости шага (явные предусловия) и вытекающие из них
            // возможные причины пропуска — для предпросмотра UI.
            let mut prerequisites: Vec<String> = Vec::new();
            let mut possible_skip_reasons: Vec<String> = Vec::new();
            for dep in &plan.dependencies {
                if dep.step_id != step.id() {
                    continue;
                }
                if !dep.prereq_id.is_empty() {
                    if !prerequisites.contains(&dep.prereq_id) {
                        prerequisites.push(dep.prereq_id.clone());
                    }
                    possible_skip_reasons.push(format!(
                        "Skipped if prerequisite '{}' fails or is skipped",
                        dep.prereq_id
                    ));
                    if !dep.expects_file.is_empty() {
                        possible_skip_reasons.push(format!(
                            "Skipped if '{}' is not created by '{}'",
                            dep.expects_file, dep.prereq_id
                        ));
                    }
                } else if !dep.expects_file.is_empty() {
                    possible_skip_reasons.push(format!(
                        "Skipped if required file '{}' does not exist (project root)",
                        dep.expects_file
                    ));
                }
            }
            match step.condition() {
                Some(StepCondition::FileExists { path }) => possible_skip_reasons
                    .push(format!("Skipped if '{}' does not exist at runtime", path)),
                Some(StepCondition::FileNotExists { path }) => {
                    possible_skip_reasons.push(format!("Skipped if '{}' exists at runtime", path))
                }
                _ => {}
            }
            previews.push(StepPreview {
                id: step.id(),
                label: step.label(),
                description: step.description(),
                action,
                will_execute,
                skip_reason,
                prerequisites,
                possible_skip_reasons,
                existing_file,
                file_policy,
            });
        }
        let total = previews.len();
        let will_execute_count = previews.iter().filter(|p| p.will_execute).count();
        RecipePreview {
            recipe_id: plan.recipe.id.clone(),
            recipe_name: plan.recipe.name.clone(),
            step_previews: previews,
            total_steps: total,
            will_execute_count,
            will_skip_count: total - will_execute_count,
            layout: plan.layout_summary.clone(),
        }
    }

    async fn execute(
        &self,
        mut plan: ExecutionPlan,
        tx: mpsc::Sender<ExecutionEvent>,
    ) -> ExecutionResult {
        plan.steps = flatten_steps(&plan.steps, &plan.context);
        // План может быть построен не через plan() (прямая конструкция
        // ExecutionPlan). Гарантируем те же инварианты: шаги нормализуются
        // стабильной топологической сортировкой; планы с настоящими циклами,
        // самозависимостями, дубликатами id или висячими предшественниками
        // не выполняются вовсе.
        match topo_order_steps(&plan.steps, &plan.dependencies) {
            Ok(ordered) => plan.steps = ordered,
            Err(err) => {
                let result = ExecutionResult {
                    recipe_id: plan.recipe.id.clone(),
                    total_duration_ms: 0,
                    step_results: Vec::new(),
                    overall: OverallStatus::Aborted {
                        last_step: None,
                        reason: err.clone(),
                    },
                };
                let _ = tx
                    .send(ExecutionEvent {
                        event_type: ExecutionEventType::AllCompleted {
                            result: result.clone(),
                        },
                        step_id: String::new(),
                        step_index: plan.steps.len(),
                        total_steps: plan.steps.len(),
                        step_name: "Complete".into(),
                        step_description: String::new(),
                        timestamp: chrono_event_time(),
                    })
                    .await;
                return result;
            }
        }
        let start = Instant::now();
        let total = plan.steps.len();
        let mut results = Vec::with_capacity(total);
        let mut aborted = false;

        for (i, step) in plan.steps.iter().enumerate() {
            // 1. Явные предусловия — ВСЕГДА до запуска команды (правило 5):
            //    провал/пропуск предшественника или отсутствие его файлового
            //    пост-условия → шаг пропускается с ТОЧНОЙ причиной, команда
            //    не выполняется (нет вторичных ENOENT-ошибок). Проверка идёт
            //    до abort-обёртки, чтобы зависимые шаги получали причину
            //    именно предшественника, а не generic «Previous step failed».
            if let Some(reason) =
                dependency_skip_reason(step, &plan.dependencies, &mut results, &plan.project_path)
            {
                results.push(StepResult {
                    step_id: step.id(),
                    label: step.label(),
                    status: StepStatus::Skipped { reason },
                    duration_ms: 0,
                });
                continue;
            }
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

            // Runtime condition: FileExists/FileNotExists проверяются по
            // ФАКТИЧЕСКОМУ состоянию файловой системы проекта (пост-условия
            // скаффолда). Если скаффолд провалил валидацию expected_outputs,
            // зависимые шаги (патчи package.json, tauri-config, npm install)
            // ПРОПУСКАЮТСЯ — вторичных ENOENT-ошибок нет. Контекстные условия
            // уже отфильтрованы в flatten_steps (plan-time evaluate_condition).
            if !runtime_condition(step.condition(), &plan.project_path) {
                let reason = match step.condition() {
                    Some(StepCondition::FileExists { path }) => format!(
                        "Skipped: expected output '{}' was not created by the scaffolding step",
                        path
                    ),
                    Some(StepCondition::FileNotExists { path }) => format!(
                        "Skipped: file '{}' unexpectedly exists (scaffolding did not run cleanly)",
                        path
                    ),
                    _ => "Skipped: runtime condition not met".into(),
                };
                results.push(StepResult {
                    step_id: step.id(),
                    label: step.label(),
                    status: StepStatus::Skipped { reason },
                    duration_ms: 0,
                });
                continue;
            }

            // Emit StepStarted
            let _ = tx
                .send(ExecutionEvent {
                    event_type: ExecutionEventType::StepStarted,
                    step_id: step.id(),
                    step_index: i,
                    total_steps: total,
                    step_name: step.label(),
                    step_description: step.description(),
                    timestamp: chrono_event_time(),
                })
                .await;

            let step_start = Instant::now();

            let result = match step {
                Step::Command { .. } => self.executor.run_command(step, &plan, &tx, i).await,
                Step::WriteFile { .. } => self.executor.write_file(step, &plan, &tx, i).await,
                // Step::RenderTemplate { .. } => {
                //     self.executor.render_template(step, &plan, &tx, i, &self.template_engine).await
                // }
                Step::CreateDirectory { .. } => {
                    self.executor.create_directory(step, &plan, &tx, i).await
                }
                Step::Generate { .. } => self.executor.run_generate(step, &plan, &tx, i).await,
                Step::Parallel { .. } => {
                    // Parallel steps are flattened before execution. Reaching
                    // this branch means a caller supplied an invalid plan.
                    StepResult {
                        step_id: step.id(),
                        label: step.label(),
                        status: StepStatus::Failed {
                            error: "Parallel step was not normalized before execution".into(),
                        },
                        duration_ms: 0,
                    }
                }
            };

            let duration = step_start.elapsed().as_millis() as u64;

            let is_failure = matches!(&result.status, StepStatus::Failed { .. });
            let _ = tx
                .send(ExecutionEvent {
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
                })
                .await;

            results.push(StepResult {
                duration_ms: duration,
                ..result
            });

            if is_failure {
                let abort = matches!(
                    &step,
                    Step::Command { on_error: ErrorMode::Abort, .. }
                    | Step::WriteFile { on_error: ErrorMode::Abort, .. }
                    // | Step::RenderTemplate { on_error: ErrorMode::Abort, .. }
                    | Step::CreateDirectory { on_error: ErrorMode::Abort, .. }
                    | Step::Generate { on_error: ErrorMode::Abort, .. }
                    | Step::Parallel { on_error: ErrorMode::Abort, .. }
                );
                if abort {
                    aborted = true;
                }
            }
        }

        let total_duration = start.elapsed().as_millis() as u64;
        let failed: Vec<String> = results
            .iter()
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
            OverallStatus::PartialFailure {
                failed_steps: failed,
            }
        };

        let result = ExecutionResult {
            recipe_id: plan.recipe.id.clone(),
            total_duration_ms: total_duration,
            step_results: results,
            overall: overall.clone(),
        };

        let _ = tx
            .send(ExecutionEvent {
                event_type: ExecutionEventType::AllCompleted {
                    result: result.clone(),
                },
                step_id: String::new(),
                step_index: total,
                total_steps: total,
                step_name: "Complete".into(),
                step_description: String::new(),
                timestamp: chrono_event_time(),
            })
            .await;

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
fn compose_recipe(
    layout: &ProjectLayout,
    context: &WizardContext,
    folder_name: &str,
) -> Result<Recipe, String> {
    // project_name может отличаться от folder_name (при auto-rename папки)
    let project_name = context.project_name.as_deref().unwrap_or(folder_name);
    let mut steps: Vec<Step> = Vec::new();
    let project_path = context
        .project_path
        .as_ref()
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

    // Фаза 1: Root Scaffolding. Корнем владеет ровно тот, кому это назначила
    // каноническая раскладка (ProjectLayout::owns_root):
    //   - Integrated: обёртки над проектом (tauri — ScaffoldOwnership
    //     wraps_existing_project) — их шаги ОТЛОЖЕНЫ (см. deferred_wrappers
    //     ниже: фронтенд обязан скаффолдиться первым);
    //   - BackendOnly/FrontendOnly: root-скаффолдер (django, nest, spring-boot).
    // В Split корнем не владеет никто — даже scaffold="root" работает внутри
    // своего сегмента (django/nest в backend/).
    // Каталоги, в которых после всех CLI-каркасов нужен РОВНО ОДИН npm install
    // ("." = корень проекта). Скаффолдеры запускаются с --skip-install/
    // --no-install, поэтому node_modules не плодятся на каждом шаге.
    let mut js_dirs: Vec<String> = Vec::new();
    let mut rest_frameworks: Vec<String> = Vec::new();
    for fw in &context.frameworks {
        if layout.owns_root(fw) && !ScaffoldOwnership::for_framework(fw).wraps_existing_project {
            steps.extend(steps_for_framework(
                fw,
                project_path,
                project_name,
                context,
                layout,
            ));
            // Root-JS-фреймворк (nest): работает в корне с --skip-install,
            // его package.json ставится один раз в финальной фазе.
            if is_js_framework(fw) {
                push_unique(&mut js_dirs, ".".to_string());
            }
        } else {
            rest_frameworks.push(fw.clone());
        }
    }

    // Фаза 2: Subdir Scaffolding. Сегменты моно-репозитория (backend + frontend)
    // создаются ТОЛЬКО в split-раскладке (eager-папки); integrated и
    // одно-сторонние раскладки папки не предсоздают — их создают сами
    // генераторы (ScaffoldGenerator) или WriteFile.
    for dir in layout.eager_dirs() {
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
        let lang_seg = layout.language_dir(lang);
        let mut lang_steps = steps_for_language(lang, project_name, project_path, context);
        if let Some(dir) = &lang_seg {
            lang_steps = into_segment(lang_steps, dir);
        }
        steps.extend(lang_steps);
        // JS-язык без фреймворка-каркаса: package.json ляжет в этот каталог —
        // там нужен финальный npm install.
        if matches!(lang.to_lowercase().as_str(), "typescript" | "javascript") {
            push_unique(&mut js_dirs, lang_seg.unwrap_or_else(|| ".".to_string()));
        }
    }

    // ========================================================================
    // Generic toolchain preflight (preflight.rs): единый слой проверки
    // инструментария вместо пер-фреймворковых патчей.
    //
    // Node: node/npm-префлайт ДО любого npm-скаффолда — отсутствие node/npm
    // останавливает пайплайн, каркасы не «молча пропускаются».
    // ========================================================================
    if context_needs_npm(context) {
        steps.extend(preflight::node_preflight_steps(project_path));
    }

    // ========================================================================
    // Python: ЕДИНСТВЕННЫЙ канонический venv (по python_segment_dir →
    // ProjectLayout) для всех Python-проектов. Создаётся один раз, маркер
    // проверяется, pip бутстрапится интерпретатором venv, манифест
    // (requirements.txt, записан language-скаффолдом выше) устанавливается
    // РОВНО один раз ДО любых framework-шагов — django-admin/alembic идут
    // только через этот venv. Отдельного «django-venv» больше нет.
    // ========================================================================
    if context.languages.iter().any(|l| l == "python") {
        let python_dir = python_segment_dir(context);
        steps.push(preflight::python_preflight_step(project_path));
        steps.extend(preflight::python_environment_steps(
            project_path,
            &python_dir,
        ));
        // Пост-валидация манифеста: requirements.txt обязан содержать все
        // выбранные зависимости фреймворков и инструментов.
        let manifest_entries = preflight::python_manifest_entries(context);
        let manifest_refs: Vec<&str> = manifest_entries.iter().map(String::as_str).collect();
        steps.push(preflight::manifest_check_step(
            "py_requirements_check",
            "Validate Python requirements manifest",
            &preflight::requirements_path(&python_dir),
            "requirements_txt",
            &manifest_refs,
        ));
    }

    // Обёртки над проектом (ScaffoldOwnership::wraps_existing_project: tauri)
    // откладываются в конец фазы Subdir Scaffolding: пайплайн обёртки обязан
    // выполнять фронтенд-генератор ПЕРВЫМ (vite в frontend/ → npm install →
    // tauri init), иначе init опережает каркас фронтенда.
    // Побочные фреймворки (kind="side": telegraf, aiogram...) выполняются
    // ПОСЛЕ главных (nest, django...): их шаги пишут поверх/патчат каркас
    // главного фреймворка (telegraf → dep-патч package.json, созданного
    // nest), поэтому стабильная перестановка «app-сначала, side-в-конец»
    // обязательна независимо от порядка карточек в мастере.
    let mut main_fws: Vec<String> = Vec::new();
    let mut side_fws: Vec<String> = Vec::new();
    for fw in rest_frameworks {
        if framework_def(&fw).is_some_and(|d| d.kind == "side") {
            side_fws.push(fw);
        } else {
            main_fws.push(fw);
        }
    }
    // Каркас с СОБСТВЕННЫМ фронтендом (electron/renderer, flutter/dart-ui):
    // его UI встроен в каркас, поэтому универсальные UI-компаньоны
    // (react/vue/svelte) рядом с ним не скаффолдятся отдельно — иначе
    // получились бы два несвязанных фронтенда в frontend/ (запрещено).
    // Обёртки (tauri) и C++-каркасы (qt) фронтенд НЕ создают — компаньоны
    // для них скаффолдятся штатно.
    let frontend_shell_present = context
        .frameworks
        .iter()
        .any(|fw| creates_frontend_shell(fw));
    let mut deferred_wrappers: Vec<Step> = Vec::new();
    for fw in main_fws.iter().chain(side_fws.iter()) {
        let ownership = ScaffoldOwnership::for_framework(fw);
        // UI-компаньоны подавляются только при наличии ЧУЖОГО владельца
        // фронтенда: сам по себе react/vue/svelte — обычный каркас.
        if frontend_shell_present && ownership.is_ui_companion {
            continue;
        }
        let fw_steps = steps_for_framework(fw, project_path, project_name, context, layout);
        if is_js_framework(fw) && !fw_steps.is_empty() {
            // Scaffold-фреймворки (Step::Generate "scaffold") кладут
            // package.json в scaffold_target_dir (frontend/), остальные —
            // в сегмент или подпапку <project_name>. Проверка СТРУКТУРНАЯ:
            // по фактическим шагам фреймворка, а не по списку id.
            let dir = if fw_steps.iter().any(
                |s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"),
            ) {
                scaffold_target_dir(fw, layout.framework_dir(fw).as_deref())
            } else {
                layout
                    .framework_dir(fw)
                    .unwrap_or_else(|| project_name.to_string())
            };
            push_unique(&mut js_dirs, dir);
        }
        if ownership.wraps_existing_project {
            // Обёртка инициализируется ПОСЛЕ всех фронтенд-каркасов
            // (см. выше: фронтенд-генератор FIRST → npm install → обёртка).
            deferred_wrappers = fw_steps;
        } else {
            steps.extend(fw_steps);
        }
    }
    steps.extend(deferred_wrappers);

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
    steps.extend(steps_for_docker(
        layout,
        context,
        project_path,
        project_name,
    ));
    steps.extend(steps_for_gitignore(context, project_path));
    steps.extend(steps_for_ci(context, project_path, project_name));
    steps.extend(steps_for_readme(
        layout,
        context,
        project_path,
        project_name,
    ));
    steps.extend(steps_for_vscode(layout, context));

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
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    // Фаза 6: финализация — единственная установка зависимостей в самом
    // конце (npm install ровно один раз на JS-каталог) и стартовый
    // git add/commit со всеми готовыми файлами.
    steps.extend(steps_for_finalize(
        context,
        &js_dirs,
        project_path,
        project_name,
    ));

    // ========================================================================
    // Явные предусловия шагов (см. StepDependency). Зависимости НЕ выводятся
    // из порядка шагов — каждая пара декларируется явно; план нормализуется
    // в plan() (topo_order_steps: стабильная топологическая сортировка,
    // циклы/самозависимости/висячие предшественники отклоняются).
    // Присутствие пар фильтруется в build_dependencies по фактическому плану:
    // ветки, отсечённые контекстом (компаньон вместо tauri_web_scaffold,
    // отсутствующий nest и т.п.), не дают «висячих» предшественников.
    // ========================================================================
    let has_python = context.languages.iter().any(|l| l == "python");
    let has_django = context.frameworks.iter().any(|f| f == "django");
    let py_dir = python_segment_dir(context);
    // Маркер канонического venv — ТОЧНО как в py_venv_create (preflight.rs):
    // создаётся один раз, зависимые verify/pip-шаги выполняются всегда
    // (пост-условие на месте, даже когда venv переиспользован).
    let venv_marker = preflight::venv_marker_rel(&py_dir);
    let dep = |step: &str, prereq: &str| StepDependency {
        step_id: step.to_string(),
        prereq_id: prereq.to_string(),
        expects_file: String::new(),
    };
    let dep_file = |step: &str, prereq: &str, file: &str| StepDependency {
        step_id: step.to_string(),
        prereq_id: prereq.to_string(),
        expects_file: file.to_string(),
    };
    let mut dependencies: Vec<StepDependency> = Vec::new();

    // NestJS: пост-патч имени package.json (nest_pkg_name) и telegraf-патч
    // (telegraf_pkg_patch) читают package.json, созданный nest_new.
    if context.frameworks.iter().any(|f| f == "nest") {
        dependencies.push(dep("nest_pkg_name", "nest_new"));
        dependencies.push(dep("telegraf_pkg_patch", "nest_new"));
    }

    // Tauri: config-патч — после tauri init; пост-патч имени package.json и
    // npm install — после фронтенд-каркаса. Каркас создаёт ИЛИ
    // tauri_web_scaffold (без компаньона), ИЛИ vite_create (с компаньоном) —
    // декларируются оба, build_dependencies оставит выжившего.
    if context.frameworks.iter().any(|f| f == "tauri") {
        dependencies.push(dep("tauri_config_patch", "tauri_init"));
        dependencies.push(dep("tauri_pkg_name", "tauri_web_scaffold"));
        dependencies.push(dep("tauri_pkg_name", "vite_create"));
        dependencies.push(dep("tauri_web_install", "tauri_web_scaffold"));
    }

    // Qt WebEngine: сборка веб-части — после vite_create; cmake-конфигурация —
    // после CMakeLists.txt (qt_cmake); cmake-сборка — после конфигурации и
    // собранного фронтенда. У qt_cmake_build ДВА предшественника, поэтому
    // expects_file для qt_web_build задаётся явно (условие шага — пост-условие
    // только этого предшественника, авто-вывод в build_dependencies отключён
    // для множественных деклараций).
    if qt_ui_mode(context) == "webengine" {
        dependencies.push(dep("qt_web_build", "vite_create"));
        dependencies.push(dep("qt_cmake_configure", "qt_cmake"));
        dependencies.push(dep_file(
            "qt_cmake_build",
            "qt_web_build",
            "frontend/dist/index.html",
        ));
        dependencies.push(dep("qt_cmake_build", "qt_cmake_configure"));
    }

    // Go: go get / cobra-cli init работают в каталоге модуля — строго после
    // go mod init (language-фаза), иначе «go.mod file not found».
    if context.frameworks.iter().any(|f| f == "gin") {
        dependencies.push(dep("get_gin", "go_mod_init"));
    }
    if context.frameworks.iter().any(|f| f == "cobra") {
        dependencies.push(dep("cobra_init", "go_mod_init"));
    }

    // Python: pip/alembic/django-admin обязаны видеть готовое каноническое
    // окружение. py_venv_create создаёт маркер один раз; py_venv_verify
    // проверяет его; pip-шаги ВЫПОЛНЯЮТСЯ даже когда venv переиспользован
    // (маркер на месте — FileNotExists-условие скипает только создание).
    // django_start (root-скаффолд, объявлен раньше по фазе) топологически
    // переносится ПОСЛЕ установки манифеста.
    if has_python {
        dependencies.push(dep_file("py_venv_verify", "py_venv_create", &venv_marker));
        dependencies.push(dep_file("py_pip_upgrade", "py_venv_create", &venv_marker));
        dependencies.push(dep_file("py_pip_check", "py_venv_create", &venv_marker));
        dependencies.push(dep("py_pip_install", "py_pip_upgrade"));
        dependencies.push(dep("py_pip_install", "py_pip_check"));
        dependencies.push(dep("py_requirements_check", "py_pip_install"));
        if context.tools.iter().any(|t| t == "alembic") {
            dependencies.push(dep("alembic_init", "py_pip_install"));
        }
        if has_django {
            dependencies.push(dep("django_start", "py_pip_install"));
        }
    }

    // Пост-валидация package.json: check-шаги читают каркас, созданный
    // scaffold-генератором соответствующего JS-фреймворка (nest_new,
    // vite_create и т.п.). Пары фильтруются по фактическому плану в
    // build_dependencies — ветки без каркаса не дают висячих зависимостей.
    for fw in &context.frameworks {
        if framework_npm_dependency(fw).is_some() {
            if let Some(scaffold_id) = scaffold_step_id_for(fw) {
                dependencies.push(dep(&format!("{}_pkg_check", fw), scaffold_id));
            }
        }
    }
    // Prisma: dep-патч package.json строго до prisma init (init читает
    // манифест и добавляет собственные записи).
    if context.tools.iter().any(|t| t == "prisma") {
        dependencies.push(dep("prisma_init", "prisma_deps"));
    }

    Ok(Recipe {
        id: format!("recipe_{}", project_name),
        name: format!("{:?} project", context.project_type),
        description: format!("Full setup for {} project", project_name),
        tags: context.languages.clone(),
        steps,
        dependencies,
    })
}

/// Проекту нужен node/npm (язык JS/TS или фреймворк на них) — перед любым
/// npm-скаффолдом выполняется общий node/npm-префлайт.
fn context_needs_npm(context: &WizardContext) -> bool {
    context
        .languages
        .iter()
        .any(|l| matches!(l.as_str(), "typescript" | "javascript"))
        || context.frameworks.iter().any(|f| is_js_framework(f))
}

/// npm-пакет, которым обязан обладать package.json каркаса фреймворка
/// (для пост-валидации manifest-check). None — пакет неочевиден/нет каркаса
/// (tauri, react-native, plasmo).
fn framework_npm_dependency(fw: &str) -> Option<&'static str> {
    match fw {
        "react" => Some("react"),
        "vue" => Some("vue"),
        "svelte" => Some("svelte"),
        "nextjs" => Some("next"),
        "sveltekit" => Some("@sveltejs/kit"),
        "nuxt" => Some("nuxt"),
        "solidjs" => Some("@solidjs/start"),
        "electron" => Some("electron"),
        "expo" => Some("expo"),
        "nest" => Some("@nestjs/core"),
        "express" => Some("express"),
        "fastify" => Some("fastify"),
        "telegraf" => Some("telegraf"),
        _ => None,
    }
}

/// Step-id скаффолда, создающего package.json для фреймворка (для
/// декларации зависимости `<fw>_pkg_check` после него).
fn scaffold_step_id_for(fw: &str) -> Option<&'static str> {
    match fw {
        "nest" => Some("nest_new"),
        "nextjs" => Some("nextjs_create"),
        "nuxt" => Some("nuxt_create"),
        "sveltekit" => Some("sveltekit_create"),
        "react" | "vue" | "svelte" => Some("vite_create"),
        "electron" => Some("electron_init"),
        "expo" => Some("expo_init"),
        "solidjs" => Some("solid_init"),
        _ => None,
    }
}

/// Путь package.json относительно корня проекта для рабочего каталога
/// пост-шага (package_name_patch_step / pkg_check).
fn package_json_rel_path(workdir: Option<&str>) -> String {
    match workdir {
        Some(wd) if !wd.is_empty() && wd != "." => format!("{}/package.json", wd),
        _ => "package.json".to_string(),
    }
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

/// Рекурсивно развернуть Parallel и отфильтровать шаги по condition.
fn flatten_and_filter(recipe: &Recipe, context: &WizardContext, _project_path: &Path) -> Vec<Step> {
    flatten_steps(&recipe.steps, context)
}

fn flatten_steps(steps: &[Step], context: &WizardContext) -> Vec<Step> {
    let mut result = Vec::new();
    fn append(steps: &[Step], context: &WizardContext, result: &mut Vec<Step>) {
        for step in steps {
            if !evaluate_condition(step.condition(), context) {
                continue;
            }
            if let Step::Parallel { steps: inner, .. } = step {
                append(inner, context, result);
            } else {
                result.push(step.clone());
            }
        }
    }
    append(steps, context, &mut result);
    result
}

fn evaluate_condition(cond: Option<&StepCondition>, context: &WizardContext) -> bool {
    let Some(cond) = cond else { return true };
    match cond {
        StepCondition::Always => true,
        StepCondition::ContextHas { key, value } => match key.as_str() {
            "language" => context.languages.iter().any(|l| l == value),
            "framework" => context.frameworks.iter().any(|f| f == value),
            "tool" => context.tools.iter().any(|t| t == value),
            _ => false,
        },
        StepCondition::ContextMissing { key } => !evaluate_condition(
            Some(&StepCondition::ContextHas {
                key: key.clone(),
                value: String::new(),
            }),
            context,
        ),
        StepCondition::FileExists { .. } | StepCondition::FileNotExists { .. } => {
            true // план: файл ещё не создан — шаг показывается в превью;
                 // фактическая проверка — в runtime_condition() при выполнении
        }
        StepCondition::TechnologyDetected { name } => {
            context.tools.contains(name) || context.frameworks.contains(name)
        }
        StepCondition::TechnologyNotDetected { name } => {
            !context.tools.contains(name) && !context.frameworks.contains(name)
        }
        StepCondition::FeatureEnabled { feature } => match feature.as_str() {
            "docker" => context.docker,
            "testing" => context.testing,
            "git_init" => context.git_init,
            "vscode_config" => context.vscode_config,
            "ci" => context.ci,
            _ => false,
        },
    }
}

/// Runtime-проверка условия по ФАКТИЧЕСКОМУ состоянию файловой системы
/// проекта (пост-условия скаффолда). Контекстные условия сюда не попадают:
/// они отфильтрованы plan-time в flatten_steps. Пути условий — относительные
/// к корню проекта; небезопасный путь (абсолютный, `..` вне корня) трактуется
/// как невыполненное условие (шаг пропускается — писать/запускать нечего).
fn runtime_condition(cond: Option<&StepCondition>, project_path: &Path) -> bool {
    let Some(cond) = cond else { return true };
    match cond {
        StepCondition::FileExists { path } => paths::resolve_in_root(project_path, path)
            .map(|p| p.exists())
            .unwrap_or(false),
        StepCondition::FileNotExists { path } => {
            // Безопасный путь: файл не существует (проверка по нормализованному
            // пути). Недопустимый путь — условие невыполнимо (skip).
            match paths::resolve_in_root(project_path, path) {
                Some(p) => !p.exists(),
                None => false,
            }
        }
        _ => true,
    }
}

/// Привести декларации зависимостей рецепта (Recipe.dependencies) к плану:
/// 1. Отбросить пары, чьи шаги отсутствуют в развёрнутом (flattened) плане —
///    ветки, отсечённые контекстными условиями (компаньон вместо
///    tauri_web_scaffold, отсутствующий nest и т.п.), не порождают
///    «висячих» предшественников.
/// 2. Вывести expects_file из собственного FileExists-условия зависимого
///    шага, если пост-условие не задано явно — НО только когда у зависимого
///    шага ровно ОДНА выжившая декларация: одно условие нельзя однозначно
///    приписать конкретному предшественнику из нескольких (qt_cmake_build
///    зависит и от qt_web_build, и от qt_cmake_configure, а его условие —
///    пост-условие только первого). Дублирующиеся пары (одна и та же
///    dependent→prereq) схлопываются в одну.
fn build_dependencies(steps: &[Step], declared: &[StepDependency]) -> Vec<StepDependency> {
    let ids: std::collections::HashSet<String> = steps.iter().map(|s| s.id()).collect();
    let mut survivors: Vec<StepDependency> = Vec::new();
    for dep in declared {
        if !ids.contains(&dep.step_id) {
            continue;
        }
        if !dep.prereq_id.is_empty() && !ids.contains(&dep.prereq_id) {
            continue;
        }
        if survivors
            .iter()
            .any(|s| s.step_id == dep.step_id && s.prereq_id == dep.prereq_id)
        {
            continue;
        }
        survivors.push(dep.clone());
    }
    let mut per_dependent: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for dep in &survivors {
        *per_dependent.entry(dep.step_id.clone()).or_insert(0) += 1;
    }
    survivors
        .into_iter()
        .map(|mut dep| {
            if dep.expects_file.is_empty() && per_dependent.get(&dep.step_id).copied() == Some(1) {
                dep.expects_file = steps
                    .iter()
                    .find(|s| s.id() == dep.step_id)
                    .and_then(|s| s.condition())
                    .and_then(|c| match c {
                        StepCondition::FileExists { path } => Some(path.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
            }
            dep
        })
        .collect()
}

/// Построить план в топологическом порядке: каждый предшественник (явное
/// предусловие) идёт СТРОГО до зависимого шага. Сортировка СТАБИЛЬНА —
/// независимые шаги сохраняют порядок декларации рецепта, поэтому рецепт
/// может декларировать шаги в любом порядке. Отклоняются только
/// по-настоящему невалидные планы: дубликаты id шагов, висячие
/// предшественники, самозависимости и циклы (с полным путём цикла в
/// ошибке). Зависимости НЕ выводятся из порядка шагов — только из
/// деклараций (Recipe.dependencies).
fn topo_order_steps(steps: &[Step], deps: &[StepDependency]) -> Result<Vec<Step>, String> {
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, step) in steps.iter().enumerate() {
        if index.insert(step.id(), i).is_some() {
            return Err(format!(
                "Dependency plan error: duplicate step id '{}' in the plan",
                step.id()
            ));
        }
    }
    // Граф «зависимый → предшественник». Чисто файловые предусловия
    // (prereq_id пуст) порядок не определяют — они проверяются только
    // в dependency_skip_reason во время выполнения.
    let mut indegree = vec![0usize; steps.len()];
    let mut prereqs_of: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    let mut dependents_of: Vec<Vec<usize>> = vec![Vec::new(); steps.len()];
    for dep in deps {
        let dep_idx = *index.get(&dep.step_id).ok_or_else(|| {
            format!(
                "Dependency plan error: step '{}' is not in the plan",
                dep.step_id
            )
        })?;
        if dep.prereq_id.is_empty() {
            continue;
        }
        let pre_idx = *index.get(&dep.prereq_id).ok_or_else(|| {
            format!(
                "Dependency plan error: prerequisite '{}' of '{}' is not in the plan",
                dep.prereq_id, dep.step_id
            )
        })?;
        if pre_idx == dep_idx {
            return Err(format!(
                "Dependency plan error: step '{}' cannot depend on itself",
                dep.step_id
            ));
        }
        indegree[dep_idx] += 1;
        prereqs_of[dep_idx].push(pre_idx);
        dependents_of[pre_idx].push(dep_idx);
    }

    // Стабильный алгоритм Кана: на каждом шаге берётся ПЕРВЫЙ (в порядке
    // декларации) шаг без неудовлетворённых предшественников.
    let mut remaining: Vec<usize> = (0..steps.len()).collect();
    let mut ordered: Vec<Step> = Vec::with_capacity(steps.len());
    while let Some(pos) = remaining.iter().position(|&i| indegree[i] == 0) {
        let i = remaining.remove(pos);
        ordered.push(steps[i].clone());
        for &dependent in &dependents_of[i] {
            indegree[dependent] -= 1;
        }
    }

    if !remaining.is_empty() {
        // Остались только шаги, чьи предшественники не были разблокированы:
        // среди них гарантированно есть настоящий цикл. Извлекаем его полный
        // путь и замыкаем на первом узле: «A -> B -> A».
        let cycle = cycle_path(steps, &remaining, &prereqs_of);
        let mut closed = cycle.clone();
        if let Some(first) = cycle.first() {
            closed.push(first.clone());
        }
        return Err(format!(
            "Dependency cycle detected: {}",
            closed.join(" -> ")
        ));
    }
    Ok(ordered)
}

/// Найти фактический цикл среди оставшихся (не упорядоченных) шагов:
/// глубина по рёбрам «зависимый → предшественник» до возврата в узел,
/// уже находящийся на стеке. Возвращает путь цикла в порядке следования
/// рёбер (без замыкания на первый узел).
fn cycle_path(steps: &[Step], remaining: &[usize], prereqs_of: &[Vec<usize>]) -> Vec<String> {
    let in_remaining: std::collections::HashSet<usize> = remaining.iter().copied().collect();
    let mut state: Vec<u8> = vec![0; steps.len()];
    let mut stack: Vec<usize> = Vec::new();
    fn dfs(
        node: usize,
        steps: &[Step],
        prereqs_of: &[Vec<usize>],
        in_remaining: &std::collections::HashSet<usize>,
        state: &mut Vec<u8>,
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[node] = 1;
        stack.push(node);
        for &pre in &prereqs_of[node] {
            if !in_remaining.contains(&pre) {
                continue;
            }
            match state[pre] {
                0 => {
                    if let Some(cycle) = dfs(pre, steps, prereqs_of, in_remaining, state, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    let pos = stack.iter().position(|&x| x == pre).unwrap();
                    return Some(stack[pos..].to_vec());
                }
                _ => {}
            }
        }
        state[node] = 2;
        stack.pop();
        None
    }
    for &node in remaining {
        if state[node] == 0 {
            if let Some(cycle) = dfs(
                node,
                steps,
                prereqs_of,
                &in_remaining,
                &mut state,
                &mut stack,
            ) {
                return cycle.iter().map(|&i| steps[i].id()).collect();
            }
        }
    }
    // Недостижимо: раз сортировка не завершилась, среди оставшихся шагов
    // гарантированно есть цикл.
    remaining.iter().map(|&i| steps[i].id()).collect()
}

/// Причина пропуска шага из-за явных предусловий (dependency rules).
/// Возвращает None, когда шаг может выполняться. Проверяет ТОЛЬКО статусы
/// предшественников (в результатах) и файловые пост-условия по фактической
/// файловой системе проекта:
///   - предшественник провалился (Failed) → пропуск с его ошибкой;
///   - предшественник пропущен и пост-условия нет (или оно отсутствует на
///     диске) → пропуск; ИСКЛЮЧЕНИЕ: django-ранний venv — py_venv_create
///     пропущен (маркер venv/pyvenv.cfg уже есть), а py_pip_upgrade
///     выполняется: пост-условие НА МЕСТЕ;
///   - предшественник успешен, но пост-условие отсутствует → предшественник
///     ретроактивно помечается Failed (с путём и рабочей директорией),
///     зависимый шаг пропускается;
///   - чистое файловое предусловие (без предшественника): файл отсутствует
///     → пропуск.
fn dependency_skip_reason(
    step: &Step,
    dependencies: &[StepDependency],
    results: &mut Vec<StepResult>,
    project_path: &Path,
) -> Option<String> {
    for dep in dependencies.iter().filter(|d| d.step_id == step.id()) {
        let prereq_result = if dep.prereq_id.is_empty() {
            None
        } else {
            results.iter().rev().find(|r| r.step_id == dep.prereq_id)
        };
        // Файловое пост-условие проверяется по безопасному пути строго
        // внутри корня проекта (те же правила, что у WriteFile/условий):
        // недопустимый путь (абсолютный, `..` за корень) трактуется как
        // отсутствующий файл — шаг пропускается.
        let file_missing = !dep.expects_file.is_empty()
            && !paths::resolve_in_root(project_path, &dep.expects_file)
                .map(|p| p.exists())
                .unwrap_or(false);
        match prereq_result {
            Some(r) => match &r.status {
                StepStatus::Failed { error } => {
                    return Some(format!(
                        "Skipped: prerequisite '{}' failed: {}",
                        dep.prereq_id, error
                    ));
                }
                StepStatus::Skipped { .. } => {
                    if dep.expects_file.is_empty() {
                        return Some(format!(
                            "Skipped: prerequisite '{}' was skipped",
                            dep.prereq_id
                        ));
                    }
                    if file_missing {
                        return Some(format!(
                            "Skipped: '{}' was not created by skipped prerequisite '{}'",
                            dep.expects_file, dep.prereq_id
                        ));
                    }
                    // Предшественник пропущен, но пост-условие на месте
                    // (django-ранний venv) — зависимый шаг выполняется.
                }
                StepStatus::Success { .. } => {
                    if file_missing {
                        // Ретроактивная пометка: команда «успешно» завершилась,
                        // но обещанного файла нет — это ошибка предшественника.
                        if let Some(prereq) = results
                            .iter_mut()
                            .rev()
                            .find(|r| r.step_id == dep.prereq_id)
                        {
                            prereq.status = StepStatus::Failed {
                                error: format!(
                                    "Command completed but expected file '{}' was not created (working directory: {})",
                                    dep.expects_file,
                                    project_path.display()
                                ),
                            };
                        }
                        return Some(format!(
                            "Skipped: expected output '{}' was not created by '{}'",
                            dep.expects_file, dep.prereq_id
                        ));
                    }
                }
                _ => {}
            },
            None => {
                // Чистое файловое предусловие (нет предшественника): файл
                // обязан существовать на момент запуска шага.
                if file_missing {
                    return Some(format!(
                        "Skipped: required file '{}' does not exist (project root)",
                        dep.expects_file
                    ));
                }
            }
        }
    }
    None
}

fn chrono_event_time() -> String {
    let local_time = Local::now();
    local_time.format("%H::%M:%S").to_string()
}

fn steps_for_language(
    lang: &str,
    project_name: &str,
    project_path: &str,
    context: &WizardContext,
) -> Vec<Step> {
    let split_command = |line: &str| -> (String, Vec<String>) {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut quote = None;
        for ch in line.chars() {
            match (quote, ch) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => current.push(c),
                (None, '\'' | '"') => quote = Some(ch),
                (None, c) if c.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                (None, c) => current.push(c),
            }
        }
        if !current.is_empty() {
            words.push(current);
        }
        let mut iter = words.into_iter();
        let program = iter.next().unwrap_or_else(|| "echo".to_string());
        (program, iter.collect())
    };

    // Вспомогательная функция для команды с рабочей директорией
    let cmd = |id: &str, label: &str, desc: &str, command: &str| -> Step {
        let (program, args) = split_command(command);
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: program,
            args,
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(60),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    };

    // Команда инициализации языка + FileNotExists-условие на маркерный файл,
    // который она создаёт: повторный запуск рецепта НЕ пересоздаёт проект
    // (cargo init, go mod init и т.п. уже отработали в прошлый раз).
    let cmd_once = |id: &str, label: &str, desc: &str, command: &str, marker: &str| -> Step {
        let mut step = cmd(id, label, desc, command);
        match &mut step {
            Step::Command { condition, .. } => {
                *condition = Some(StepCondition::FileNotExists {
                    path: marker.to_string(),
                });
            }
            _ => unreachable!("cmd() always builds Step::Command"),
        }
        step
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
            cmd_once("cargo_init", "Init Cargo project",
                &format!("Initialize new Rust project '{}'", project_name),
                &format!("cargo init --name {}", project_name),
                "Cargo.toml"),
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
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ];

            // Для TypeScript добавляем tsconfig.json и инициализацию
            if lang == "typescript" {
                steps.push(cmd_once("tsc_init", "Init TypeScript",
                    "Generate tsconfig.json",
                    "npx -p typescript tsc --init --target ES2022 --module commonjs --outDir dist --rootDir src",
                    "tsconfig.json"));
                steps.push(Step::WriteFile {
                    id: "ts_src_index".into(),
                    label: "Create src/index.ts".into(),
                    description: "Create entry point for TypeScript".into(),
                    path: "src/index.ts".into(),
                    content: "console.log('Hello from TypeScript!');\n".into(),
                    overwrite: false,
                    policy: None,
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
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
            }

            steps
        },

        "python" => {
            // ЕДИНЫЙ детерминированный requirements.txt для всего стека
            // (preflight::python_manifest): все Python-фреймворки и
            // Python-инструменты пишутся ОДНИМ шагом ДО venv/pip-фазы.
            // Установка читает ровно этот файл (py_pip_install) — никакие
            // framework-шаги ничего не дописывают и не перезаписывают.
            let requirements = preflight::python_manifest(context);
            vec![
                mkdir("create_src", "src"),
                Step::WriteFile {
                    id: "pyproject_toml".into(),
                    label: "Create pyproject.toml".into(),
                    description: "Initialize Python project configuration".into(),
                    path: "pyproject.toml".into(),
                    content: format!("[project]\nname = \"{}\"\nversion = \"0.1.0\"\ndescription = \"\"\nrequires-python = \">=3.10\"\n", project_name),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
                Step::WriteFile {
                    id: "requirements_txt".into(),
                    label: "Create requirements.txt".into(),
                    description: "Initialize requirements file".into(),
                    path: "requirements.txt".into(),
                    content: requirements,
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ]
        },

        "go" => vec![
            cmd_once("go_mod_init", "Init Go module",
                &format!("Initialize Go module '{}'", project_name),
                &format!("go mod init {}", project_name),
                "go.mod"),
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
                policy: None,
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
                // Повторный запуск: pom.xml уже создан прошлым запуском.
                condition: Some(StepCondition::FileNotExists { path: "pom.xml".into() }),
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
                    policy: None,
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
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                },
            ]
        },

        "zig" => {
            // `zig init` раскладывает shell (build.zig, build.zig.zon,
            // src/) В ТЕКУЩЕМ каталоге — способность generates_root_shell
            // с пост-условиями: провал/неполный вывод виден как ошибка шага,
            // а не тихо скипнутая команда (раньше был plain Command).
            // Сегментация: каталогом становится сегмент (backend/ в
            // mono-репозитории), куда и валидируются build.zig + src/.
            vec![scaffold_step(
                "zig_init",
                "Init Zig project",
                "Initialize Zig project (build.zig, build.zig.zon, src/)",
                "zig",
                vec!["init"],
                ScaffoldCapability::GeneratesRootShell,
                ".",
                ScaffoldExtras::default()
                    .expects(&["build.zig", "build.zig.zon"])
                    .policy(FilePolicy::SkipIfExists),
            )]
        },

        "dart" => {
            // Dart-пакеты не принимают дефис в имени — приводим к подчёркиванию
            let safe_name = project_name.replace('-', "_");
            vec![
                cmd_once("dart_create", "Create Dart project",
                    &format!("Create new Dart project '{}'", project_name),
                    &format!("dart create {}", safe_name),
                    // dart create создаёт ПОДПАПКУ с именем пакета
                    &format!("{}/pubspec.yaml", safe_name)),
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
                policy: None,
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
                policy: None,
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
            cmd_once("mix_new", "Create Elixir project",
                &format!("Create new Elixir project '{}'", project_name),
                &format!("mix new {}", project_name),
                "mix.exs"),
        ],

        "gleam" => vec![
            cmd_once("gleam_new", "Create Gleam project",
                &format!("Create new Gleam project '{}'", project_name),
                &format!("gleam new {}", project_name),
                "gleam.toml"),
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
    matches!(
        l,
        "typescript" | "javascript" | "dart" | "kotlin" | "swift" | "csharp"
    )
}
fn _is_backend_lang(l: &str) -> bool {
    matches!(
        l,
        "rust"
            | "python"
            | "go"
            | "java"
            | "csharp"
            | "php"
            | "elixir"
            | "zig"
            | "gleam"
            | "cpp"
            | "c"
    )
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
// Каноническая раскладка проекта (ProjectLayout)
//
// ЕДИНСТВЕННОЕ решение о структуре каталогов проекта: класс раскладки,
// владелец корня, каталоги фреймворков и языков. Вычисляется ОДИН раз из
// WizardContext (plan → compose_recipe) и пронизывает все потребители —
// compose_recipe, steps_for_framework, python-сегментацию, steps_for_vscode,
// README/Docker и duplicate_framework_write_paths. Никаких локальных
// эвристик в потребителях: все спрашивают у ProjectLayout.
//
// Классы:
//   - Integrated — фреймворк side="either" && scaffold="root" (сегодня это
//     только tauri). Оболочка владеет корнем, веб-фронтенд живёт в frontend/,
//     бэкенд-компаньоны — в backend/, языки-компаньоны — по своим сторонам.
//   - Split — есть И бэкенд, И фронтенд (языки или фреймворки). Жёсткие
//     сегменты backend/ + frontend/, корнем не владеет никто: даже
//     scaffold="root" (django, nest, spring-boot) работает ВНУТРИ backend/.
//   - BackendOnly — только бэкенд: всё в корне.
//   - FrontendOnly — только фронтенд: скаффолдеры с output_subdir="frontend"
//     в frontend/, остальное в корне.
// ============================================================================

/// Стороны проекта по контексту: явные назначения мастера
/// (backend_languages/frontend_languages), вывод по category языков — и side
/// фреймворков из wizard_tree.json. Фреймворк с жёсткой стороной (side=
/// "backend"/"frontend") — полноценная сторона: nest (backend) + nextjs
/// (frontend) включают сегментацию, даже если в контексте единственный язык
/// (typescript) или он не назначен бэкенд-стороне (aspnetcore + maui — оба
/// на csharp).
fn context_sides(context: &WizardContext) -> (bool, bool) {
    let lang_side = lang_side_map(context);
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

/// Язык → сторона: явные назначения мастера (backend_languages /
/// frontend_languages) имеют приоритет; языки без назначения — по category
/// (обратная совместимость со старыми сессиями), "both"-языки по умолчанию
/// считаются бэкендом (csharp, dart, kotlin...).
fn lang_side_map(context: &WizardContext) -> HashMap<String, &'static str> {
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
    lang_side
}

/// Каноническая раскладка проекта (см. шапку секции выше).
struct ProjectLayout {
    class: LayoutClass,
    /// Каталоги, которые движок создаёт ДО всех скаффолдеров (только split).
    eager_dirs: Vec<String>,
    /// Каталоги сегментов по сторонам (None = корень/стороны нет).
    backend_dir: Option<String>,
    frontend_dir: Option<String>,
    /// Владелец корня при integrated (tauri) — None иначе.
    root_owner: Option<String>,
    /// Явные назначения сторон мастера (backend_languages/frontend_languages).
    explicit_side: HashMap<String, &'static str>,
    /// Все выбранные фреймворки (для framework-ассоциаций языков).
    frameworks: Vec<String>,
}

enum LayoutClass {
    /// Оболочка (tauri) владеет корнем; веб в frontend/, серверные
    /// компаньоны в backend/.
    Integrated,
    /// backend/ + frontend/ — сегменты моно-репозитория.
    Split,
    /// Только бэкенд — всё в корне.
    BackendOnly,
    /// Только фронтенд — скаффолдеры в frontend/, остальное в корне.
    FrontendOnly,
}

impl ProjectLayout {
    /// Единственная точка решения о раскладке проекта. Никакие другие
    /// функции не догадываются о каталогах сами.
    pub fn compute(context: &WizardContext) -> ProjectLayout {
        // Явные назначения сторон мастера — только они, без выводов по category
        // (см. side_for_language: явное > ассоциация фреймворка > category).
        let mut explicit_side: HashMap<String, &'static str> = HashMap::new();
        for l in &context.backend_languages {
            explicit_side.insert(l.clone(), "backend");
        }
        for l in &context.frontend_languages {
            explicit_side.insert(l.clone(), "frontend");
        }

        // Integrated-оболочка: side="either" && scaffold="root" (tauri).
        // Проверяется ДО сторон: tauri делает стек integrated независимо
        // от того, есть ли рядом бэкенд и фронтенд.
        let integrated_shell = context.frameworks.iter().find(|fw| {
            framework_def(fw)
                .is_some_and(|def| def.side == "either" && def.scaffold.as_deref() == Some("root"))
        });
        if let Some(shell) = integrated_shell {
            let mut has_backend_companion = false;
            let mut has_frontend_companion = false;
            for fw in &context.frameworks {
                if fw == shell {
                    continue;
                }
                match framework_def(fw).map(|def| def.side.as_str()) {
                    Some("backend") => has_backend_companion = true,
                    Some("frontend") => has_frontend_companion = true,
                    _ => {}
                }
            }
            return ProjectLayout {
                class: LayoutClass::Integrated,
                eager_dirs: Vec::new(),
                backend_dir: has_backend_companion.then(|| "backend".to_string()),
                frontend_dir: has_frontend_companion.then(|| "frontend".to_string()),
                root_owner: Some(shell.clone()),
                explicit_side,
                frameworks: context.frameworks.clone(),
            };
        }

        // Обе стороны (языки И фреймворки) → Split. Иначе одно-сторонняя
        // раскладка: всё в корне (BackendOnly / FrontendOnly).
        let (has_backend, has_frontend) = context_sides(context);
        let class = if has_backend && has_frontend {
            LayoutClass::Split
        } else if has_backend {
            LayoutClass::BackendOnly
        } else {
            LayoutClass::FrontendOnly
        };
        let is_split = matches!(class, LayoutClass::Split);
        ProjectLayout {
            eager_dirs: if is_split {
                vec!["backend".to_string(), "frontend".to_string()]
            } else {
                Vec::new()
            },
            backend_dir: is_split.then(|| "backend".to_string()),
            frontend_dir: is_split.then(|| "frontend".to_string()),
            root_owner: None,
            explicit_side,
            frameworks: context.frameworks.clone(),
            class,
        }
    }

    /// Фреймворк владеет корнем проекта?
    ///
    ///   - Integrated: только сама оболочка (tauri) — её CLI и shell живут
    ///     в корне рядом с frontend/ и backend/;
    ///   - Split: корнем не владеет никто — даже scaffold="root" (django,
    ///     nest, spring-boot) работает ВНУТРИ backend/;
    ///   - BackendOnly/FrontendOnly: root-скаффолдер остаётся в корне.
    pub fn owns_root(&self, fw: &str) -> bool {
        match &self.class {
            LayoutClass::Integrated => self.root_owner.as_deref() == Some(fw),
            LayoutClass::Split => false,
            _ => framework_def(fw).is_some_and(|d| d.scaffold.as_deref() == Some("root")),
        }
    }

    /// Сторона языка (None = корень/не определена). Приоритет:
    ///   1. явное назначение мастера (backend_languages/frontend_languages);
    ///   2. язык самой integrated-оболочки — остаётся с ней в корне;
    ///   3. язык, требуемый фреймворком с жёсткой стороной (dart+flutter →
    ///      frontend — решает кейс zig+flutter; если язык нужен фреймворкам
    ///      ОБЕИХ сторон, правило неоднозначно — уступает category);
    ///   4. вывод по category (обратная совместимость со старыми сессиями).
    pub fn side_for_language(&self, lang: &str) -> Option<&'static str> {
        if let Some(side) = self.explicit_side.get(lang).copied() {
            return Some(side);
        }
        if let LayoutClass::Integrated = &self.class {
            if let Some(shell) = &self.root_owner {
                if framework_def(shell).is_some_and(|def| def.languages.iter().any(|l| l == lang)) {
                    return None; // корень оболочки
                }
            }
        }
        let mut required_by: Vec<&'static str> = Vec::new();
        for fw in &self.frameworks {
            let def = framework_def(fw);
            if def.is_some_and(|d| d.languages.iter().any(|l| l == lang)) {
                match def.map(|d| d.side.as_str()) {
                    Some("backend") if !required_by.contains(&"backend") => {
                        required_by.push("backend")
                    }
                    Some("frontend") if !required_by.contains(&"frontend") => {
                        required_by.push("frontend")
                    }
                    _ => {}
                }
            }
        }
        if required_by.len() == 1 {
            return Some(required_by[0]);
        }
        language_side_infer(lang)
    }

    /// Каталог сегмента для стороны (None = корень).
    fn dir_for_side(&self, side: &str) -> Option<String> {
        match (&self.class, side) {
            (LayoutClass::Split, "backend") => self.backend_dir.clone(),
            (LayoutClass::Split, "frontend") => self.frontend_dir.clone(),
            // Integrated: каталог существует только когда на этой стороне
            // есть компаньон (backend/ при fastapi, frontend/ при react).
            // Языки и фреймворки без компаньона остаются в корне рядом с
            // оболочкой.
            (LayoutClass::Integrated, "backend") => self.backend_dir.clone(),
            (LayoutClass::Integrated, "frontend") => self.frontend_dir.clone(),
            _ => None,
        }
    }

    /// Каталог языка (None = корень проекта).
    pub fn language_dir(&self, lang: &str) -> Option<String> {
        self.side_for_language(lang)
            .and_then(|side| self.dir_for_side(side))
    }

    /// Каталог фреймворка (None = корень проекта). Приоритет — жёсткая
    /// сторона фреймворка (side в wizard_tree.json); для side="either" —
    /// сторона требуемого языка. В одно-сторонних раскладках всё остаётся
    /// в корне, кроме frontend-скаффолдеров FrontendOnly (output_subdir =
    /// "frontend": react, nextjs, flutter... в frontend/).
    pub fn framework_dir(&self, fw: &str) -> Option<String> {
        if let Some(def) = framework_def(fw) {
            match def.side.as_str() {
                "backend" => return self.dir_for_side("backend"),
                "frontend" => {
                    return match &self.class {
                        LayoutClass::FrontendOnly
                            if SCAFFOLD_GENERATOR_FRAMEWORKS.contains(&fw) =>
                        {
                            Some("frontend".to_string())
                        }
                        _ => self.dir_for_side("frontend"),
                    };
                }
                _ => {}
            }
            if def.side == "either" {
                for lang in &def.languages {
                    if let Some(side) = self.side_for_language(lang) {
                        return self.dir_for_side(side);
                    }
                }
            }
        }
        None
    }

    /// Каталоги, которые движок создаёт ДО всех CLI-скаффолдеров
    /// (только split: backend/ + frontend/). Integrated и одно-сторонние
    /// раскладки папок не предсоздают — их создают сами генераторы
    /// (ScaffoldGenerator.resolve_target) или WriteFile.
    pub fn eager_dirs(&self) -> &[String] {
        &self.eager_dirs
    }

    /// Человекочитаемый снимок решения для предпросмотра (RecipePreview).
    pub fn to_summary(&self, context: &WizardContext) -> LayoutSummary {
        let class = match &self.class {
            LayoutClass::Split => "split",
            LayoutClass::Integrated => "integrated",
            LayoutClass::BackendOnly => "backend-only",
            LayoutClass::FrontendOnly => "frontend-only",
        };
        let framework_placement = context
            .frameworks
            .iter()
            .map(|fw| FrameworkPlacement {
                framework: fw.clone(),
                directory: self.framework_dir(fw).unwrap_or_else(|| ".".to_string()),
            })
            .collect();
        LayoutSummary {
            class: class.to_string(),
            generated_directories: self.eager_dirs.clone(),
            root_owner: self.root_owner.clone(),
            framework_placement,
        }
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

/// Шаги CLI-генераторов, которые сами создают подпапку с именем проекта
/// (mix phx.new <name>, vapor new <name> и т.п.).
/// Индекс — позиция имени создаваемой подпапки в args.
/// При сегментации в такой команде подменяется имя создаваемой подпапки
/// (индекс в args), а рабочая директория остаётся корнем проекта;
/// остальные шаги (write_file, остальные команды) переносятся в сегмент.
/// Фронтенд-скаффолдеры (create-vite, create-next-app, nuxi init, expo,
/// electron, laravel/symfony через composer, solidjs, flutter) из этого
/// списка УБРАНЫ — их каталогом управляет ScaffoldGenerator
/// (Step::Generate "scaffold", см. генератор), который знает способность
/// каждого CLI.
const FOLDER_MAKER_STEPS: &[(&str, usize)] = &[
    ("rn_init", 2),     // npx @react-native-community/cli init <имя>
    ("plasmo_init", 2), // npx plasmo init <имя>
    ("phoenix_new", 1), // mix phx.new <имя>
    ("vapor_new", 1),   // vapor new <имя>
];

/// Степ-иды языковых инициализаций, чьи FileNotExists-маркеры
/// относительны РАБОЧЕЙ ДИРЕКТОРИИ шага (cargo init создаёт
/// Cargo.toml рядом с собой и т.п.). При сегментации (into_segment)
/// маркер должен переехать вместе с рабочей директорией в
/// сегмент: иначе повторный запуск рецепта в mono-репо проверяет
/// корень проекта, и init заново запускается, падая с
/// "already exists" (молчаливый skip).
const WORKDIR_MARKER_STEPS: &[&str] = &[
    "cargo_init",
    "tsc_init",
    "go_mod_init",
    "maven_init",
    "mix_new",
    "gleam_new",
    "dart_create",
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
/// префикс. Генераторы подпапок (FOLDER_MAKER_STEPS) получают имя сегмента
/// вместо имени проекта (create-electron-app frontend из корня).
fn into_segment(steps: Vec<Step>, dir: &str) -> Vec<Step> {
    steps
        .into_iter()
        .map(|step| {
            let folder_maker_index = FOLDER_MAKER_STEPS
                .iter()
                .find(|(sid, _)| matches!(&step, Step::Command { id, .. } if *sid == id.as_str()))
                .map(|(_, idx)| *idx);
            let replaces = folder_maker_index.is_some();
            match step {
                Step::Command {
                    id,
                    label,
                    description,
                    command,
                    mut args,
                    working_dir,
                    env,
                    timeout_secs,
                    condition,
                    on_error,
                    interactive,
                } => {
                    if replaces {
                        if let Some(idx) = folder_maker_index {
                            if let Some(arg) = args.get_mut(idx) {
                                // генератор сам создаст подпапку — рабочая директория остаётся корневой
                                *arg = dir.to_string();
                            }
                        }
                        Step::Command {
                            id,
                            label,
                            description,
                            command,
                            args,
                            working_dir,
                            env,
                            timeout_secs,
                            condition,
                            on_error,
                            interactive,
                        }
                    } else if id.starts_with("tauri_web_") || id.starts_with("qt_web_") {
                        // Веб-часть tauri (tauri_web_*) живёт в frontend/
                        // независимо от сегмента самого tauri (backend/ в
                        // моно-репозитории); qt_web_* — сборка веб-части Qt
                        // WebEngine (frontend/) рядом с qt-сегментом (backend/).
                        // Рабочая директория уже относительна корня проекта,
                        // сегментация её НЕ трогает.
                        Step::Command {
                            id,
                            label,
                            description,
                            command,
                            args,
                            working_dir,
                            env,
                            timeout_secs,
                            condition,
                            on_error,
                            interactive,
                        }
                    } else {
                        // Маркеры языковых init-шагов относительны рабочей
                        // директории — при сегментации переезжают в сегмент.
                        let condition = if WORKDIR_MARKER_STEPS.contains(&id.as_str()) {
                            condition.map(|c| match c {
                                StepCondition::FileNotExists { path } => {
                                    StepCondition::FileNotExists {
                                        path: join_seg(dir, &path),
                                    }
                                }
                                other => other,
                            })
                        } else {
                            condition
                        };
                        Step::Command {
                            id,
                            label,
                            description,
                            command,
                            args,
                            working_dir: working_dir.map(|wd| join_seg(&wd, dir)),
                            env,
                            timeout_secs,
                            condition,
                            on_error,
                            interactive,
                        }
                    }
                }
                Step::WriteFile {
                    id,
                    label,
                    description,
                    path,
                    content,
                    overwrite,
                    policy,
                    condition,
                    on_error,
                } => Step::WriteFile {
                    id,
                    label,
                    description,
                    path: format!("{}/{}", dir, path),
                    content,
                    overwrite,
                    policy,
                    condition,
                    on_error,
                },
                Step::CreateDirectory {
                    id,
                    label,
                    description,
                    path,
                    condition,
                    on_error,
                } => Step::CreateDirectory {
                    id,
                    label,
                    description,
                    path: format!("{}/{}", dir, path),
                    condition,
                    on_error,
                },
                // Scaffold-генератор (Step::Generate "scaffold") сам кладёт проект
                // в target_dir: при сегментации каталогом становится сегмент.
                // Шаги с явным каталогом, не зависящим от раскладки, не трогаем
                // (см. scaffold_lands_in_target_dir: tauri_web_scaffold —
                // веб-часть всегда frontend/, tauri_init — оболочка в корне,
                // does_not_create_a_project — CLI без каталога проекта).
                // Spring Boot (генератор "spring-boot") распаковывает starter
                // внутри сегмента — каталог передаётся через target_dir.
                Step::Generate {
                    id,
                    label,
                    description,
                    generator_id,
                    mut generator_config,
                    policy,
                    condition,
                    on_error,
                } => {
                    if scaffold_lands_in_target_dir(&id, &generator_id, &generator_config) {
                        generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                    }
                    if generator_id == "spring-boot" {
                        generator_config["target_dir"] = serde_json::Value::String(dir.to_string());
                    }
                    Step::Generate {
                        id,
                        label,
                        description,
                        generator_id,
                        generator_config,
                        policy,
                        condition,
                        on_error,
                    }
                }
                other => other,
            }
        })
        .collect()
}

/// Scaffold-генератор кладёт проект в target_dir: при сегментации — сегмент
/// (into_segment), в монолите — scaffold_target_dir (output_subdir фреймворка).
/// ЕДИНСТВЕННОЕ правило, по которому движок решает, можно ли переопределять
/// каталог scaffold-шага; используется и в into_segment, и в
/// steps_for_framework (иначе логика расходится). Шаги с ЯВНЫМ каталогом,
/// не зависящим от раскладки, не трогаются:
///   - tauri_web_scaffold: веб-часть tauri ВСЕГДА в frontend/;
///   - tauri_init: integrated-оболочка работает в корне (src-tauri/ в корне);
///   - does_not_create_a_project: CLI не создаёт каталог проекта (zig init —
///     раскладывает shell в текущем каталоге, пост-условия валидируются
///     относительно рабочей директории).
fn scaffold_lands_in_target_dir(
    id: &str,
    generator_id: &str,
    generator_config: &serde_json::Value,
) -> bool {
    generator_id == "scaffold"
        && id != "tauri_web_scaffold"
        && id != "tauri_init"
        && generator_config
            .get("capability")
            .and_then(|v| v.as_str())
            .map_or(true, |c| c != "does_not_create_a_project")
}

/// Скаффолдеры, которые генерируют package.json и называют его по имени
/// папки (frontend/, <project_name>/) вместо project_name из WizardContext.
/// Для них движок добавляет пост-шаг, переписывающий поле name
/// (см. package_name_patch_step) — чинит баг «frontend/package.json
/// называется frontend».
const PACKAGE_JSON_SCAFFOLDS: &[&str] = &[
    "react",
    "vue",
    "svelte",
    "nextjs",
    "sveltekit",
    "nuxt",
    "solidjs",
    "electron",
    "expo",
    "react-native",
    "plasmo",
    "tauri",
    "nest",
];

/// Фреймворки, чей каркас создаёт ScaffoldGenerator (Step::Generate
/// "scaffold", см. generators/mod.rs): CLI-генератор вызывается по ЯВНОЙ
/// способности (creates_named_directory / creates_project_and_may_prompt) —
/// во временную папку с программным переносом (temp+move) или в текущий
/// каталог — без матрёшек testapp/testapp.
/// Для них каталог проекта — scaffold_target_dir(...), а не подпапка
/// <project_name>/ (см. также js_dirs и pkg-name patch).
const SCAFFOLD_GENERATOR_FRAMEWORKS: &[&str] = &[
    "react",
    "vue",
    "svelte",
    "nextjs",
    "sveltekit",
    "nuxt",
    "expo",
    "solidjs",
    "flutter",
    "electron",
    "laravel",
    "symfony",
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
fn package_name_patch_step(
    id: &str,
    label: &str,
    workdir: Option<&str>,
    project_name: &str,
) -> Step {
    // Апостроф в имени проекта ломает JS-строку — экранируем.
    let safe_name = project_name.replace('\'', "\\'");
    // Патч выполняется ТОЛЬКО если скаффолдер действительно создал
    // package.json (postcondition) — при провале скаффолда шаг пропускается
    // вместо вторичной ENOENT-ошибки.
    let pkg_path = match workdir {
        Some(wd) if !wd.is_empty() && wd != "." => format!("{}/package.json", wd),
        _ => "package.json".to_string(),
    };
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
        condition: Some(StepCondition::FileExists { path: pkg_path }),
        on_error: ErrorMode::Abort,
        interactive: vec![],
    }
}

/// Шаги фреймворка с учётом канонической раскладки (ProjectLayout):
/// сегментация, целевые каталоги scaffold-генераторов и пост-патч имени
/// package.json. Все каталоги берутся ТОЛЬКО из layout — никаких локальных
/// эвристик (root_rest_seg и т.п. больше нет).
/// Метаданные владения каркасом: что именно создаёт CLI-скаффолдер и как
/// движок может с ним работать. ЕДИНСТВЕННАЯ таблица — поведение каждого
/// скаффолдера описывается здесь, а не разрозненными проверками по id
/// фреймворка в compose_recipe / steps_for_framework / into_segment.
/// Проверки «fw == "tauri"», «fw == "electron"» вне этой таблицы — ошибка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScaffoldOwnership {
    /// CLI создаёт полный каркас приложения (create-vite, create-electron-app,
    /// flutter create, tauri init).
    pub creates_app_shell: bool,
    /// Каркас включает СОБСТВЕННЫЙ фронтенд (react/vite, electron/renderer,
    /// flutter/dart-ui). UI-компаньоны (react/vue/svelte) рядом с таким
    /// владельцем не скаффолдятся отдельно — их UI уже встроен в каркас.
    pub creates_frontend: bool,
    /// Встраиваемый UI-компаньон (react/vue/svelte): UI-библиотека, которую
    /// мастера выбирают ПОД оболочку (tauri, electron, qt-webengine). Рядом
    /// с каркасом, у которого есть собственный фронтенд, компаньон не
    /// скаффолдится (compose_recipe) — его UI уже встроен в каркас.
    pub is_ui_companion: bool,
    /// CLI требует ПУСТОЙ каталог назначения: в существующем каталоге с
    /// посторонними файлами CLI падает (create-next-app, create-expo-app).
    pub requires_empty_dir: bool,
    /// CLI умеет дописывать каркас в УЖЕ СУЩЕСТВУЮЩИЙ каталог (nest new .,
    /// tauri init, zig init, flutter create .).
    pub may_run_in_existing_dir: bool,
    /// CLI может выполняться во временной папке (temp+move): созданный
    /// каталог программно переносится в каталог назначения.
    pub supports_staging_dir: bool,
    /// Выход CLI можно программно слить с каталогом назначения.
    pub output_mergeable: bool,
    /// Гарантированные выходные пути (пост-условия валидации) относительно
    /// каталога назначения.
    pub expected_outputs: &'static [&'static str],
    /// CLI — обёртка над другим проектом: требует уже существующий каркас
    /// (tauri init поверх фронтенда). Шаги обёртки откладываются в конец
    /// фазы скаффолдинга.
    pub wraps_existing_project: bool,
}

impl ScaffoldOwnership {
    pub const fn none() -> Self {
        Self {
            creates_app_shell: false,
            creates_frontend: false,
            is_ui_companion: false,
            requires_empty_dir: false,
            may_run_in_existing_dir: false,
            supports_staging_dir: false,
            output_mergeable: false,
            expected_outputs: &[],
            wraps_existing_project: false,
        }
    }

    /// Каркас создаёт собственный фронтенд целиком (app shell + UI):
    /// electron/renderer, flutter/dart-ui, react/vite...
    pub fn creates_frontend_shell(&self) -> bool {
        self.creates_app_shell && self.creates_frontend
    }

    /// Единственная таблица владения: id фреймворка → метаданные его
    /// скаффолдера. Фреймворки без CLI-каркаса (inplace: express, fastapi,
    /// axum...) — ScaffoldOwnership::none().
    pub fn for_framework(fw: &str) -> Self {
        match fw {
            // Встраиваемые UI-компаньоны и веб-фреймворки: полный
            // фронтенд-каркас в каталоге назначения.
            "react" | "vue" | "svelte" | "nextjs" | "sveltekit" | "nuxt" | "expo" | "solidjs" => {
                Self {
                    creates_app_shell: true,
                    creates_frontend: true,
                    is_ui_companion: matches!(fw, "react" | "vue" | "svelte"),
                    requires_empty_dir: true,
                    may_run_in_existing_dir: false,
                    supports_staging_dir: true,
                    output_mergeable: true,
                    expected_outputs: &["package.json"],
                    wraps_existing_project: false,
                }
            }
            // Electron: create-electron-app собирает ПОЛНЫЙ каркас
            // (main + renderer) — собственный фронтенд, UI-компаньоны
            // подавляются (см. compose_recipe). Forge init в непустом
            // каталоге назначения падает (запрещено) — каркас собирается
            // во временной папке (temp+move) и переносится программно.
            "electron" => Self {
                creates_app_shell: true,
                creates_frontend: true,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: false,
                supports_staging_dir: true,
                output_mergeable: true,
                expected_outputs: &["package.json"],
                wraps_existing_project: false,
            },
            // Flutter: flutter create --project-name <имя> . работает
            // ВНУТРИ каталога назначения (в существующем каталоге — dart-ui).
            "flutter" => Self {
                creates_app_shell: true,
                creates_frontend: true,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: true,
                expected_outputs: &["pubspec.yaml", "lib"],
                wraps_existing_project: false,
            },
            // Tauri: tauri init раскладывает src-tauri/ shell В КОРНЕ
            // существующего проекта — обёртка над фронтендом, который
            // обязан существовать ДО init (компаньон или vite-vanilla).
            "tauri" => Self {
                creates_app_shell: true,
                creates_frontend: false,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: false,
                expected_outputs: &["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                wraps_existing_project: true,
            },
            // Qt: C++-каркас с собственным UI-стеком (QML/Widgets/WebEngine/
            // Kirigami). Режим WebEngine встраивает веб-фронтенд
            // (react/vue/svelte): qt-шаги сборки веб-части выполняются
            // ПОСЛЕ фронтенд-скаффолда (явные зависимости в compose_recipe).
            "qt" => Self {
                creates_app_shell: true,
                creates_frontend: false,
                is_ui_companion: false,
                requires_empty_dir: false,
                may_run_in_existing_dir: true,
                supports_staging_dir: false,
                output_mergeable: false,
                expected_outputs: &["CMakeLists.txt", "src/main.cpp"],
                wraps_existing_project: false,
            },
            _ => Self::none(),
        }
    }
}

/// Каркас-владелец фронтенда (не-компаньон): electron, flutter, nextjs...
/// Его наличие подавляет UI-компаньонов (react/vue/svelte) в compose_recipe.
fn creates_frontend_shell(fw: &str) -> bool {
    let o = ScaffoldOwnership::for_framework(fw);
    o.creates_frontend_shell() && !o.is_ui_companion
}

fn steps_for_framework(
    fw: &str,
    project_path: &str,
    project_name: &str,
    context: &WizardContext,
    layout: &ProjectLayout,
) -> Vec<Step> {
    let seg = layout.framework_dir(fw);
    let mut steps =
        steps_for_framework_impl(fw, project_path, project_name, context, seg.as_deref());

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

    let mut steps = match seg.as_deref() {
        Some(dir) => into_segment(steps, dir),
        None => steps,
    };

    // Scaffold-генераторы: into_segment не трогает Generate-шаги, поэтому
    // целевой каталог выставляется здесь — по сегменту или output_subdir.
    // Проверка СТРУКТУРНАЯ (по фактическим шагам фреймворка), а не по
    // списку id: любой каркас, чей шаг — Step::Generate "scaffold",
    // попадает под правило. Шаги с явным каталогом (tauri_init,
    // tauri_web_scaffold, does_not_create_a_project) не трогаются —
    // см. scaffold_lands_in_target_dir (та же логика, что в into_segment).
    let has_scaffold_generate = steps.iter().any(|s| {
        matches!(s, Step::Generate { id, generator_id, generator_config, .. }
            if scaffold_lands_in_target_dir(id, generator_id, generator_config))
    });
    if has_scaffold_generate {
        let target_dir = scaffold_target_dir(fw, seg.as_deref());
        for step in &mut steps {
            if let Step::Generate {
                id,
                generator_id,
                generator_config,
                ..
            } = step
            {
                if scaffold_lands_in_target_dir(id, generator_id, generator_config) {
                    generator_config["target_dir"] = serde_json::Value::String(target_dir.clone());
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
        // package.json появляется у фреймворков со scaffold-каркасом
        // (root/subdir) и у integrated-оболочки tauri (frontend/).
        let scaffold_root = layout.owns_root(fw);
        if (scaffold_root || framework_def(fw).is_some_and(|d| d.scaffold.is_some()))
            && PACKAGE_JSON_SCAFFOLDS.contains(&fw)
        {
            let workdir: Option<String> =
                if ScaffoldOwnership::for_framework(fw).wraps_existing_project {
                    // Обёртка (tauri): package.json принадлежит встроенному
                    // фронтенду (frontend/), а не оболочке.
                    Some("frontend".to_string())
                } else if has_scaffold_generate {
                    // package.json лежит в каталоге, куда скаффолдер положил проект
                    Some(scaffold_target_dir(fw, seg.as_deref()))
                } else if scaffold_root {
                    // root-скаффолдеры (nest, django) создают package.json в корне
                    None
                } else {
                    // subdir-скаффолдеры — внутри созданной подпапки (сегмент
                    // frontend/ в моно-репозитории или <project_name> в монолите)
                    Some(seg.unwrap_or_else(|| project_name.to_string()))
                };
            steps.push(package_name_patch_step(
                &format!("{}_pkg_name", fw),
                &format!("Fix package.json name for {}", fw),
                workdir.as_deref(),
                project_name,
            ));
            // Пост-валидация package.json: каркас обязан реально содержать
            // зависимость фреймворка (npm install финальной фазы установит
            // её), а не только entry-файл. Зависимость `<fw>_pkg_check` ←
            // scaffold-шаг объявлена в compose_recipe.
            if let Some(dep_name) = framework_npm_dependency(fw) {
                steps.push(preflight::package_json_check_step(
                    &format!("{}_pkg_check", fw),
                    &format!("Validate {} package.json", fw),
                    &package_json_rel_path(workdir.as_deref()),
                    &[dep_name],
                ));
            }
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
    // Каталоги считаются ТОЧНО как в compose_recipe — через каноническую
    // раскладку (ProjectLayout), иначе легальные связки (tauri→корень,
    // react→frontend/) дали бы ложные срабатывания.
    let layout = ProjectLayout::compute(context);
    let mut by_path: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for fw in &context.frameworks {
        let steps = steps_for_framework(fw, ".", &project_name, context, &layout);
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
        policy: None,
        condition: None,
        on_error: ErrorMode::Abort,
    }
}

fn qt_steps_widgets(project_name: &str) -> Vec<Step> {
    vec![
        qt_step_write(
            "qt_main",
            "Create Qt main",
            "src/main.cpp",
            format!(
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
            ),
        ),
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
                r#"cmake_minimum_required(VERSION 3.16)
project({p})

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

add_executable({p} src/main.cpp)
target_link_libraries({p} Qt6::Widgets)
"#,
                p = project_name
            ),
        ),
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
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
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
            ),
        ),
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
        qt_step_write(
            "qt_kirigami_main",
            "Create Kirigami view",
            "src/main.qml",
            main_qml,
        ),
        qt_step_write(
            "qt_cmake",
            "Create CMakeLists.txt",
            "CMakeLists.txt",
            format!(
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
            ),
        ),
    ]
}

fn qt_steps_webengine(project_name: &str, context: &WizardContext, seg: Option<&str>) -> Vec<Step> {
    let web = qt_web_framework_label(context);
    // Расширенные шаблоны живут в TemplateEngine ({{ project_name }} и т.п.)
    // — см. engine/template.rs: qt_webengine_main_cpp / qt_webengine_cmake.
    let engine = template::TemplateEngine::new();
    let main_cpp = engine.qt_webengine_main_cpp(project_name);
    let cmake_lists = engine.qt_webengine_cmake(project_name);
    // Пост-условия с учётом сегментации qt (backend/ в mono-репозитории):
    // into_segment не переписывает condition-пути, поэтому префикс сегмента
    // добавляется ЗДЕСЬ; рабочие директории cmake-шагов (".") сегментация
    // переведёт в каталог qt (backend/) сама.
    let cmake_cond = match seg {
        Some(dir) => format!("{}/CMakeLists.txt", dir),
        None => "CMakeLists.txt".to_string(),
    };
    vec![
        qt_step_write("qt_main", "Create Qt main (WebEngine)", "src/main.cpp", main_cpp),
        qt_step_write("qt_cmake", "Create CMakeLists.txt", "CMakeLists.txt", cmake_lists),
        Step::Command {
            id: "qt_web_build".into(),
            label: "Build the web UI for Qt WebEngine".into(),
            description: format!(
                "Run npm run build in frontend/ (Web UI is {}). The built app must appear at frontend/dist/index.html — the Qt WebEngine widget loads this file at runtime; without it the window stays blank.",
                web
            ),
            command: "npm".into(),
            args: vec!["run".into(), "build".into()],
            working_dir: Some("frontend".into()),
            env: None,
            timeout_secs: Some(600),
            condition: Some(StepCondition::FileExists {
                path: "frontend/package.json".into(),
            }),
            on_error: ErrorMode::Skip,
            interactive: vec![],
        },
        Step::Command {
            id: "qt_cmake_configure".into(),
            label: "Configure Qt build with CMake".into(),
            description: format!(
                "Configure the Qt WebEngine application (cmake -S . -B build). Requires Qt6 with the WebEngine module (Qt6::WebEngineWidgets), CMake 3.16+ and a C++ compiler (MSVC, MinGW or g++/clang). The web UI must be built first (frontend/dist/index.html)."
            ),
            command: "cmake".into(),
            args: vec!["-S".into(), ".".into(), "-B".into(), "build".into()],
            working_dir: Some(".".into()),
            env: None,
            timeout_secs: Some(600),
            condition: Some(StepCondition::FileExists { path: cmake_cond }),
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
        Step::Command {
            id: "qt_cmake_build".into(),
            label: "Build Qt WebEngine application".into(),
            description: format!(
                "Compile the Qt WebEngine application (cmake --build build). Depends on the configured build/ and the built web UI (frontend/dist/index.html); failures usually mean missing Qt6 WebEngineWidgets dev files or a broken compiler toolchain."
            ),
            command: "cmake".into(),
            args: vec!["--build".into(), "build".into()],
            working_dir: Some(".".into()),
            env: None,
            timeout_secs: Some(1200),
            condition: Some(StepCondition::FileExists {
                path: "frontend/dist/index.html".into(),
            }),
            on_error: ErrorMode::Abort,
            interactive: vec![],
        },
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

/// Шаг «scaffold» через Composer: command/args резолвятся через
/// composer_launch() — глобальный `composer` или `php <абс. composer.phar>`,
/// поэтому плейсхолдер ищется по фактическому положению в args.
/// Composer — creates_named_directory с временной папкой по умолчанию
/// (temp+move): "." не принимается в непустом каталоге.
/// Пост-условие — composer.json (валидный манифест каркаса, а не
/// package.json). Abort: без composer каркас не инициализируется —
/// префлайт уже проверил доступность и напечатал пути/команду.
fn composer_scaffold_step(id: &str, label: &str, desc: &str, package: &str) -> Step {
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
    let mut step = scaffold_step(
        id,
        label,
        desc,
        &command,
        args.iter().map(String::as_str).collect(),
        ScaffoldCapability::CreatesNamedDirectory,
        ".",
        ScaffoldExtras::default()
            .expects(&["composer.json"])
            .policy(FilePolicy::SkipIfExists),
    );
    if let Step::Generate { on_error, .. } = &mut step {
        *on_error = ErrorMode::Abort;
    }
    step
}

/// Дополнительные параметры scaffold-шага (все опциональны; значения по
/// умолчанию — в ScaffoldGenerator: temp_dir_allowed по способности,
/// working_dir по способности, timeout 600).
#[derive(Default)]
struct ScaffoldExtras<'a> {
    working_dir: Option<&'a str>,
    temp_dir_allowed: Option<bool>,
    expected_outputs: Vec<&'a str>,
    interactive: Vec<serde_json::Value>,
    timeout_secs: Option<u64>,
    policy: Option<FilePolicy>,
}

impl<'a> ScaffoldExtras<'a> {
    /// Пост-условия: пути, которые обязаны появиться после завершения CLI
    /// (относительно каталога назначения). Провал любого — ошибка шага.
    fn expects(mut self, outputs: &[&'a str]) -> Self {
        self.expected_outputs.extend_from_slice(outputs);
        self
    }
    /// Интерактивные ответы: {"trigger": "...", "response_type": ...}.
    fn interact(mut self, entries: Vec<serde_json::Value>) -> Self {
        self.interactive = entries;
        self
    }
    fn in_dir(mut self, wd: &'a str) -> Self {
        self.working_dir = Some(wd);
        self
    }
    /// Явно разрешить/запретить временную папку (по умолчанию — по способности).
    fn temp_dir(mut self, allowed: bool) -> Self {
        self.temp_dir_allowed = Some(allowed);
        self
    }
    fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }
    /// Политика идемпотентности: SkipIfExists пропускает CLI, когда все
    /// expected_outputs уже на месте (повторный запуск рецепта).
    fn policy(mut self, policy: FilePolicy) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// Шаг «scaffold»: CLI-генератор, поведение которого задаёт ЯВНАЯ
/// способность (ScaffoldCapability). ScaffoldGenerator разбирается с
/// каталогом и пост-условиями сам (см. generators/mod.rs):
///   - creates_named_directory / creates_project_and_may_prompt: CLI
///     выполняется во временной папке temp_<target> (если разрешено),
///     содержимое (включая скрытые файлы) программно переносится в target;
///   - creates_in_current_directory: CLI работает ВНУТРИ target с ".".
/// Матрёшек testapp/testapp и пустых каркасов без node_modules нет.
#[allow(clippy::too_many_arguments)]
fn scaffold_step(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    capability: ScaffoldCapability,
    target_dir: &str,
    extras: ScaffoldExtras<'_>,
) -> Step {
    let mut config = serde_json::json!({
        "command": command,
        "args": args,
        "capability": capability.as_str(),
        "target_dir": target_dir,
    });
    if let Some(wd) = extras.working_dir {
        config["working_dir"] = serde_json::Value::String(wd.to_string());
    }
    if let Some(allowed) = extras.temp_dir_allowed {
        config["temp_dir_allowed"] = serde_json::Value::Bool(allowed);
    }
    if !extras.expected_outputs.is_empty() {
        config["expected_outputs"] = serde_json::Value::Array(
            extras
                .expected_outputs
                .iter()
                .map(|o| serde_json::Value::String(o.to_string()))
                .collect(),
        );
    }
    if !extras.interactive.is_empty() {
        config["interactive"] = serde_json::Value::Array(extras.interactive);
    }
    if let Some(secs) = extras.timeout_secs {
        config["timeout_secs"] = serde_json::Value::Number(secs.into());
    }
    Step::Generate {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        generator_id: "scaffold".into(),
        generator_config: config,
        policy: extras.policy,
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
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "async",
    "await",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "noalias",
    "noinline",
    "nosuspend",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "linksection",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "usingnamespace",
    "var",
    "volatile",
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
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
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
            policy: None,
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
            format!(
                r#"package com.example.app

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
"#
            ),
        )
    } else if java_lang {
        (
            "app/src/main/java/com/example/app/MainActivity.java".to_string(),
            format!(
                r#"package com.example.app;

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
"#
            ),
        )
    } else {
        (
            "app/src/main/kotlin/com/example/app/MainActivity.kt".to_string(),
            format!(
                r#"package com.example.app

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
"#
            ),
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
    // Единый список каталогов discovery (тот же, что печатает PHP-префлайт
    // в preflight.rs).
    for dir in preflight::COMPOSER_PHAR_DIRS {
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

fn steps_for_framework_impl(
    fw: &str,
    project_path: &str,
    project_name: &str,
    context: &WizardContext,
    seg: Option<&str>,
) -> Vec<Step> {
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
    let cmd_i = |id: &str,
                 label: &str,
                 desc: &str,
                 command: &str,
                 args: Vec<&str>,
                 interactive: Vec<InteractiveEntry>|
     -> Step {
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
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    match fw.to_lowercase().as_str() {
        // ==================== Rust ====================
        "axum" => vec![
            write_file(
                "axum_main",
                "Create Axum entry point",
                "src/main.rs",
                &format!(
                    r#"use axum::{{routing::get, Router}};

#[tokio::main]
async fn main() {{
    let app = Router::new().route("/", get(|| async {{ "Hello from {}!" }}));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}}
"#,
                    project_name
                ),
            ),
            // Добавление зависимостей в Cargo.toml — это отдельная тема
            // Пока просто команда, которую executor должен уметь парсить/выполнять
            cmd(
                "add_axum_deps",
                "Add Axum dependencies",
                "Add axum + tokio to Cargo.toml",
                "cargo",
                vec!["add", "axum", "tokio", "--features", "tokio/full"],
            ),
        ],

        "tauri" => {
            // Пайплайн tauri (порядок шагов гарантируется движком):
            //   1. Фронтенд-генератор ПЕРВЫМ: vite скаффолдит frontend/
            //      (компаньон react/vue/svelte или vanilla-ts без компаньона).
            //   2. npm install внутри frontend/ — до tauri init (явный шаг
            //      только когда фронтенд скаффолдит сам tauri; при компаньоне
            //      установку делает финальная фаза после его каркаса).
            //   3. cargo tauri init В КОРНЕ проекта (integrated-раскладка:
            //      tauri — владелец корня, см. ProjectLayout) — frontendDist
            //      указывает на frontend/dist.
            // create-tauri-app НЕ используется: он скаффолдил фронтенд по
            // шаблону в корне (структура была пустой без node_modules).
            //
            // Пути жёстко корневые: каноническая раскладка делает tauri
            // integrated ВСЕГДА (side="either" + scaffold="root"), поэтому
            // сегментного режима backend/ у tauri больше нет.
            let identifier = tauri_identifier(project_name);
            let has_companion = context.frameworks.iter().any(|f| {
                framework_def("tauri").is_some_and(|def| def.companions.iter().any(|c| c == f))
            });
            let frontend_dist = "../frontend/dist".to_string();
            let dev_cmd = "npm --prefix frontend run dev".to_string();
            let build_cmd = "npm --prefix frontend run build".to_string();
            let mut steps: Vec<Step> = Vec::new();
            if !has_companion {
                // Компаньон (react/vue/svelte) уже скаффолдит frontend/ —
                // без него фронтенд создаёт vite (vanilla).
                let template = if has_typescript {
                    "vanilla-ts"
                } else {
                    "vanilla"
                };
                steps.push(scaffold_step(
                    "tauri_web_scaffold",
                    "Create frontend for Tauri",
                    "Scaffold Vite frontend in frontend/",
                    "npx",
                    vec![
                        "create-vite@latest",
                        SCAFFOLD_TARGET,
                        "--template",
                        template,
                    ],
                    ScaffoldCapability::CreatesNamedDirectory,
                    "frontend",
                    ScaffoldExtras::default()
                        .expects(&["package.json"])
                        .policy(FilePolicy::SkipIfExists),
                ));
                // npm install внутри frontend/ — до tauri init. Выполняется
                // только если веб-скаффолд действительно создал package.json
                // (postcondition), иначе — провал скаффолда был бы замаскирован
                // вторичной ошибкой npm.
                steps.push(Step::Command {
                    id: "tauri_web_install".into(),
                    label: "Install Tauri frontend dependencies".into(),
                    description: "Run npm install inside frontend/".into(),
                    command: "npm".into(),
                    args: vec!["install".into()],
                    working_dir: Some("frontend".into()),
                    env: None,
                    timeout_secs: Some(600),
                    condition: Some(StepCondition::FileExists {
                        path: "frontend/package.json".into(),
                    }),
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
            }
            // cargo tauri init --ci: неинтерактивно, все пути — на frontend/dist.
            // Способность generates_root_shell: tauri init не создаёт каталог
            // проекта, а раскладывает shell в текущем каталоге; пост-условия
            // — src-tauri/tauri.conf.json И src-tauri/Cargo.toml (оба обязаны
            // появиться, иначе shell неполный). --yes: npx обязан согласиться
            // на скачивание @tauri-apps/cli без stdin. Условие frontend/
            // package.json: init выполняется только когда фронтенд-каркас
            // реально создан (компаньон react/vue/svelte или vite-vanilla).
            steps.push(scaffold_step(
                "tauri_init",
                "Initialize Tauri shell",
                "Run tauri init (non-interactive, --ci)",
                // Use the package-local/global npm CLI as a fallback instead
                // of requiring `cargo-tauri` to be preinstalled. `npx --yes`
                // downloads the official CLI when necessary and works on
                // Windows where `cargo tauri` otherwise reports "no such
                // command".
                "npx",
                vec![
                    "--yes",
                    "@tauri-apps/cli",
                    "init",
                    "--ci",
                    "--app-name",
                    project_name,
                    "--window-title",
                    project_name,
                    "--frontend-dist",
                    frontend_dist.as_str(),
                    "--dev-url",
                    "http://localhost:5173",
                    "--before-dev-command",
                    dev_cmd.as_str(),
                    "--before-build-command",
                    build_cmd.as_str(),
                ],
                ScaffoldCapability::GeneratesRootShell,
                ".",
                ScaffoldExtras::default()
                    .expects(&["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"])
                    .policy(FilePolicy::SkipIfExists),
            ));
            // Ворота: tauri init выполняется только когда фронтенд-каркас
            // реально создан (package.json в frontend/) — иначе init
            // сконфигурирует пустую frontend/dist, и провал фронтенд-
            // скаффолда был бы замаскирован вторичной ошибкой.
            if let Some(Step::Generate { condition, .. }) = steps.last_mut() {
                *condition = Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                });
            }
            // Rust-патч tauri.conf.json: пути на frontend/, identifier.
            // Выполняется только если tauri init действительно создал конфиг.
            steps.push(Step::Generate {
                id: "tauri_config_patch".into(),
                label: "Patch Tauri configuration".into(),
                description: "Adapt src-tauri/tauri.conf.json to the frontend/ layout".into(),
                generator_id: "tauri-config".into(),
                generator_config: serde_json::json!({
                    "frontend_dir": "frontend",
                    "tauri_dir": "",
                    "frontend_dist": frontend_dist,
                    "dev_url": "http://localhost:5173",
                    "before_dev_command": dev_cmd,
                    "before_build_command": build_cmd,
                    "identifier": identifier,
                }),
                policy: None,
                condition: Some(StepCondition::FileExists {
                    path: "src-tauri/tauri.conf.json".into(),
                }),
                on_error: ErrorMode::Skip,
            });
            steps
        }

        "clap" => {
            // Легальная связка axum + clap: веб-сервер владеет src/main.rs,
            // CLI становится отдельным бинарником Cargo (src/bin/cli.rs).
            // Поодиночке clap занимает src/main.rs.
            let cli_path = if context.frameworks.iter().any(|f| f == "axum") {
                "src/bin/cli.rs"
            } else {
                "src/main.rs"
            };
            let cli_id = if cli_path == "src/main.rs" {
                "clap_main"
            } else {
                "clap_cli"
            };
            let cli_label = if cli_path == "src/main.rs" {
                "Create CLI entry point"
            } else {
                "Create CLI binary (src/bin/cli.rs)"
            };
            vec![
                cmd(
                    "add_clap_deps",
                    "Add Clap dependency",
                    "Add clap with derive feature",
                    "cargo",
                    vec!["add", "clap", "--features", "derive"],
                ),
                write_file(
                    cli_id,
                    cli_label,
                    cli_path,
                    &format!(
                        r#"use clap::Parser;

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
"#,
                        project_name
                    ),
                ),
            ]
        }

        // ==================== Python ====================
        "fastapi" => vec![
            write_file(
                "fastapi_main",
                "Create FastAPI entry point",
                "src/main.py",
                &format!(
                    r#"from fastapi import FastAPI

app = FastAPI(title="{}", version="0.1.0")

@app.get("/")
async def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    import uvicorn
    uvicorn.run("main:app", host="0.0.0.0", port=3000, reload=True)
"#,
                    project_name, project_name
                ),
            ),
            // Зависимости живут в ЕДИНОМ requirements.txt python-скаффолда
            // (union: фреймворки + инструменты) — fastapi не перезаписывает
            // его, иначе aiogram/SQLAlchemy терялись.
        ],

        "django" => {
            // django-admin startproject требует валидный Python-идентификатор:
            // «my-project» (дефис) не подходит — заменяем на подчёркивание.
            let safe_name = project_name.replace('-', "_");
            // CLI запускается из КАНОНИЧЕСКОГО venv проекта (никакого
            // глобального django-admin и никакого отдельного venv-цикла):
            // каноническое окружение уже создано и манифест установлен
            // (django_start ← py_pip_install в декларациях зависимостей).
            let django_command = if context.languages.iter().any(|l| l == "python") {
                python_venv_bin(project_path, seg.unwrap_or("."), "django-admin")
            } else {
                "django-admin".to_string()
            };
            let mut start = cmd(
                "django_start",
                "Start Django project",
                "Create Django project structure (django-admin from the project venv)",
                &django_command,
                vec!["startproject", &safe_name, "."],
            );
            if let Step::Command { on_error, .. } = &mut start {
                // Обязательный CLI каркаса: провал останавливает пайплайн.
                *on_error = ErrorMode::Abort;
            }
            vec![start]
        }

        "flask" => vec![
            write_file(
                "flask_app",
                "Create Flask app",
                "src/app.py",
                &format!(
                    r#"from flask import Flask

app = Flask(__name__)

@app.route("/")
def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=3000, debug=True)
"#,
                    project_name
                ),
            ),
            // flask попадает в union requirements.txt python-скаффолда
        ],

        "aiogram" => vec![
            write_file(
                "aiogram_bot",
                "Create Telegram bot",
                "src/bot.py",
                &format!(
                    r#"import asyncio
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
"#,
                    project_name
                ),
            ),
            // aiogram попадает в union requirements.txt python-скаффолда
        ],

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
            vec![scaffold_step(
                "vite_create",
                &format!("Create {fw} app"),
                "Scaffold Vite project",
                "npx",
                vec![
                    "create-vite@latest",
                    SCAFFOLD_TARGET,
                    "--template",
                    template,
                ],
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        // ==================== JavaScript / TypeScript ====================
        "nextjs" => {
            // create-next-app с --yes работает без интерактива (--typescript/
            // --javascript фиксируют язык, остальное — флагами). Каталог
            // frontend/ выбирает ScaffoldGenerator (temp+move: скаффолд во
            // временной папке, программный перенос в frontend/).
            let ts_flag = if has_typescript {
                "--typescript"
            } else {
                "--javascript"
            };
            vec![scaffold_step(
                "nextjs_create",
                "Create Next.js app",
                "Scaffold Next.js project",
                "npx",
                vec![
                    "create-next-app@latest",
                    SCAFFOLD_TARGET,
                    ts_flag,
                    "--tailwind",
                    "--eslint",
                    "--app",
                    "--no-src-dir",
                    "--import-alias",
                    "@/*",
                    "--use-npm",
                    "--skip-install",
                    "--yes",
                ],
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        "sveltekit" => {
            // sv create полностью неинтерактивен с флагами: шаблон minimal,
            // типы фиксируются --types/--no-types, доп. инструменты не ставим.
            let mut sv_args = vec![
                "sv",
                "create",
                SCAFFOLD_TARGET,
                "--template",
                "minimal",
                "--no-add-ons",
                "--no-install",
            ];
            if has_typescript {
                sv_args.push("--types");
                sv_args.push("ts");
            } else {
                sv_args.push("--no-types");
            }
            vec![scaffold_step(
                "sveltekit_create",
                "Create SvelteKit app",
                "Scaffold SvelteKit project",
                "npx",
                sv_args,
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        "nuxt" => {
            // nuxi init неинтерактивен: в не-TTY сессии ОБЯЗАТЕЛЬНЫ dir,
            // template, packageManager и gitInit (nuxi выводит help и
            // завершается с ошибкой без любого из них). Шаблон задаётся
            // ВСЕГДА явно (--template minimal), булевы флаги — через "="
            // (citty не принимает пробельный вариант для booleans),
            // установка зависимостей откладывается в финальную фазу.
            vec![scaffold_step(
                "nuxt_create",
                "Create Nuxt app",
                "Scaffold Nuxt project",
                "npx",
                vec![
                    "--yes",
                    "nuxi@latest",
                    "init",
                    SCAFFOLD_TARGET,
                    "--template",
                    "minimal",
                    "--packageManager",
                    "npm",
                    "--gitInit=false",
                    "--no-install",
                ],
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        "express" => {
            // package.json в каталоге сегмента (into_segment префиксует
            // WriteFile-путь; condition/конфиг генератора — вручную).
            let pkg_path = match seg {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            vec![
                write_file(
                    "express_index",
                    "Create Express entry",
                    "src/index.js",
                    &format!(
                        r#"const express = require('express');
const app = express();
const PORT = process.env.PORT || 3000;

app.get('/', (req, res) => {{
    res.json({{ message: 'Hello from {}!' }});
}});

app.listen(PORT, () => {{
    console.log(`Server running on http://localhost:${{PORT}}`);
}});
"#,
                        project_name
                    ),
                ),
                write_file(
                    "express_package",
                    "Express dependencies",
                    "package.json",
                    &format!(
                        r#"{{
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
"#,
                        project_name
                    ),
                ),
                // Express обязан быть установлен как НАСТОЯЩАЯ зависимость
                // (финальный npm install), а не только упомянут entry-файлом:
                // пост-валидация манифеста это подтверждает.
                preflight::package_json_check_step(
                    "express_pkg_check",
                    "Validate Express package.json",
                    &pkg_path,
                    &["express"],
                ),
            ]
        }

        "electron" => {
            // create-electron-app собирает ПОЛНЫЙ каркас Electron
            // (main + renderer): собственный фронтенд, поэтому UI-компаньоны
            // (react/vue/svelte) рядом с electron не скаффолдятся отдельно
            // (см. compose_recipe — их UI уже встроен в каркас). Шаблон
            // renderer'а фиксируется флагом --template (typescript —
            // webpack+TS, vite — vanilla vite); CLI может задать вопрос про
            // git-init — способность creates_project_and_may_prompt с явным
            // ответом. Каркас собирается во ВРЕМЕННОЙ папке (temp+move):
            // Forge init в непустом каталоге назначения падает (запрещено),
            // стейджинг гарантирует пустой каталог. Пост-условие —
            // package.json (без него шаг считается неуспешным).
            let template = if has_typescript { "typescript" } else { "vite" };
            vec![scaffold_step(
                "electron_init",
                "Init Electron",
                "Create Electron app with electron-forge",
                "npx",
                vec![
                    "create-electron-app",
                    SCAFFOLD_TARGET,
                    "--template",
                    template,
                ],
                ScaffoldCapability::CreatesProjectAndMayPrompt,
                "frontend",
                ScaffoldExtras::default()
                    .expects(&["package.json"])
                    .policy(FilePolicy::SkipIfExists)
                    .interact(vec![serde_json::json!({
                        "trigger": "Initialize a git repository?",
                        "response_type": "n",
                    })]),
            )]
        }

        "telegraf" => {
            // Telegraf — побочный (kind="side") фреймворк: выполняется ПОСЛЕ
            // главного каркаса (nest) и обязан НЕ перезаписывать его
            // package.json. При связке с nest зависимости дописываются
            // dep-патчем (node -e), который читает существующий файл —
            // условие FileExists скипает шаг, если nest-каркас не создался
            // (нет вторичной ENOENT-ошибки). Без nest telegraf пишет
            // собственный package.json (перезапись: inplace-фреймворк).
            let mut steps = vec![write_file(
                "telegraf_bot",
                "Create Telegram bot",
                "src/bot.js",
                &format!(
                    r#"const {{ Telegraf }} = require('telegraf');
const bot = new Telegraf('YOUR_BOT_TOKEN');

bot.start((ctx) => ctx.reply('Hello from {}!'));

bot.launch();
process.once('SIGINT', () => bot.stop('SIGINT'));
process.once('SIGTERM', () => bot.stop('SIGTERM'));
"#,
                    project_name
                ),
            )];
            if context.frameworks.iter().any(|f| f == "nest") {
                // Путь package.json с учётом сегментации telegraf (backend/
                // в mono-репозитории): into_segment переведёт рабочую
                // директорию в сегмент, condition-путь задаётся здесь.
                let pkg_path = match seg {
                    Some(dir) => format!("{}/package.json", dir),
                    None => "package.json".to_string(),
                };
                steps.push(Step::Command {
                    id: "telegraf_pkg_patch".into(),
                    label: "Add Telegraf to NestJS dependencies".into(),
                    description: "Patch package.json created by the NestJS scaffold to add the telegraf dependency (NestJS owns package.json)".into(),
                    command: "node".into(),
                    args: vec![
                        "-e".into(),
                        "const fs=require('fs');const p='package.json';const j=JSON.parse(fs.readFileSync(p,'utf8'));j.dependencies=j.dependencies||{};j.dependencies['telegraf']='^4.16.3';fs.writeFileSync(p,JSON.stringify(j,null,2)+'\\n')".into(),
                    ],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(30),
                    condition: Some(StepCondition::FileExists { path: pkg_path.clone() }),
                    on_error: ErrorMode::Skip,
                    interactive: vec![],
                });
                // telegraf — НАСТОЯЩАЯ зависимость после патча: подтверждаем
                // манифестом (установку выполнит финальный npm install).
                steps.push(preflight::package_json_check_step(
                    "telegraf_pkg_check",
                    "Validate Telegraf dependency",
                    &pkg_path,
                    &["telegraf"],
                ));
            } else {
                steps.push(write_file(
                    "telegraf_package",
                    "Telegraf package.json",
                    "package.json",
                    &format!(
                        r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/bot.js",
  "dependencies": {{
    "telegraf": "^4.16.3"
  }}
}}
"#,
                        project_name
                    ),
                ));
                let pkg_path = match seg {
                    Some(dir) => format!("{}/package.json", dir),
                    None => "package.json".to_string(),
                };
                steps.push(preflight::package_json_check_step(
                    "telegraf_pkg_check",
                    "Validate Telegraf package.json",
                    &pkg_path,
                    &["telegraf"],
                ));
            }
            steps
        }

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
        }

        "expo" => {
            // create-expo-app НЕ принимает "." в качестве имени проекта —
            // ScaffoldGenerator (temp+move) скаффолдит во временную папку и
            // программно переносит содержимое в frontend/.
            let expo_template = if has_typescript {
                "blank-typescript"
            } else {
                "blank"
            };
            vec![scaffold_step(
                "expo_init",
                "Init Expo",
                "Create Expo project",
                "npx",
                vec![
                    "create-expo-app",
                    SCAFFOLD_TARGET,
                    "--yes",
                    "--no-install",
                    "--template",
                    expo_template,
                ],
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        "plasmo" => vec![cmd_i(
            "plasmo_init",
            "Init Plasmo",
            "Create browser extension with Plasmo",
            "npx",
            vec!["plasmo", "init", project_name],
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
            ],
        )],

        "nest" => vec![
            // --yes: npx обязан согласиться на скачивание @nestjs/cli без
            // интерактивного ввода («Ok to proceed? (y)»), иначе не-TTY
            // сессия повисает/падает.
            // --package-manager npm: фиксирует ответ на вопрос «Which package
            // manager would you love to use?» флагом, без ожидания stdin.
            // --skip-install: зависимости корня ставятся ОДИН раз в финальной
            // фазе пайплайна (steps_for_finalize), а не сразу в каркасе —
            // иначе node_modules плодятся на каждом шаге.
            // --skip-git: git инициализирует сам движок (steps_for_git_init).
            cmd_i(
                "nest_new",
                "Create NestJS project",
                "Scaffold NestJS application",
                "npx",
                vec![
                    "--yes",
                    "@nestjs/cli",
                    "new",
                    ".",
                    "--package-manager",
                    "npm",
                    "--skip-install",
                    "--skip-git",
                ],
                vec![InteractiveEntry {
                    trigger: "Which package manager would you love to use".into(),
                    response_type: ResponseType::Text("npm".to_string()),
                }],
            ),
        ],

        "fastify" => {
            let pkg_path = match seg {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            vec![
                write_file(
                    "fastify_index",
                    "Create Fastify entry",
                    "src/index.js",
                    &format!(
                        r#"const fastify = require('fastify')({{ logger: true }});

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
"#,
                        project_name
                    ),
                ),
                write_file(
                    "fastify_package",
                    "Fastify package.json",
                    "package.json",
                    &format!(
                        r#"{{
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
"#,
                        project_name
                    ),
                ),
                // fastify — НАСТОЯЩАЯ зависимость (финальный npm install),
                // пост-валидация подтверждает декларацию в package.json.
                preflight::package_json_check_step(
                    "fastify_pkg_check",
                    "Validate Fastify package.json",
                    &pkg_path,
                    &["fastify"],
                ),
            ]
        }

        "solidjs" => {
            // create-solid полностью неинтерактивен при ПОЛНОМ наборе
            // флагов: projectName и template — позиционные, тип — --solidstart
            // (SolidStart 1.x), версия — --v2 (без него CLI спрашивает
            // «Which version of SolidStart?» даже в не-TTY), язык — --ts.
            // Шаблон "basic" — валидный SolidStart-шаблон (в отличие от "ts",
            // который валиден только для vanilla-проектов и заставлял CLI
            // молча завершаться с exit 0). --no-install у create-solid нет —
            // CLI никогда не ставит зависимости сам.
            let template = "basic";
            vec![scaffold_step(
                "solid_init",
                "Create SolidStart app",
                "Scaffold SolidStart project in frontend/",
                "npx",
                vec![
                    "--yes",
                    "create-solid",
                    SCAFFOLD_TARGET,
                    template,
                    "--solidstart",
                    "--v2",
                    "--ts",
                ],
                ScaffoldCapability::CreatesProjectAndMayPrompt,
                "frontend",
                ScaffoldExtras::default().expects(&["package.json"]),
            )]
        }

        // ==================== Go ====================
        "gin" => {
            // go get с @latest — современная форма (без него go get может
            // вернуть кэшированную версию без обновления go.mod). Условие
            // FileExists go.mod: go mod init (language-фаза) выполняется
            // строго до framework-шагов, но без go.mod команда go get
            // создала бы его в неверном каталоге — скипаем с причиной.
            let go_mod_path = match seg {
                Some(dir) => format!("{}/go.mod", dir),
                None => "go.mod".to_string(),
            };
            vec![
                write_file(
                    "gin_main",
                    "Create Gin entry",
                    "cmd/main.go",
                    &format!(
                        r#"package main

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
"#,
                        project_name
                    ),
                ),
                Step::Command {
                    id: "get_gin".into(),
                    label: "Install Gin".into(),
                    description: "Add Gin dependency (modern go get pkg@latest)".into(),
                    command: "go".into(),
                    args: vec!["get".into(), "github.com/gin-gonic/gin@latest".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(300),
                    condition: Some(StepCondition::FileExists { path: go_mod_path }),
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                },
            ]
        }

        "cobra" => {
            // Легальная связка gin + cobra: веб-сервер владеет cmd/main.go,
            // CLI получает собственный пакет cmd/cli/main.go. Поодиночке
            // cobra занимает cmd/main.go.
            let (cli_path, cli_id, cli_label) = if context.frameworks.iter().any(|f| f == "gin") {
                (
                    "cmd/cli/main.go",
                    "cobra_cli",
                    "Create CLI entry (cmd/cli/main.go)",
                )
            } else {
                ("cmd/main.go", "cobra_main", "Create CLI entry")
            };
            // go.mod живёт в каталоге сегмента (backend/ в mono-репозитории);
            // cobra-cli init обязателен строго после go mod init (language-фаза),
            // иначе генератор падает «go.mod file not found». Условия FileExists
            // скипают шаги, если модуль не создался — без каскада ошибок.
            let go_mod_path = match seg {
                Some(dir) => format!("{}/go.mod", dir),
                None => "go.mod".to_string(),
            };
            vec![
                Step::WriteFile {
                    id: "go_mod".into(),
                    label: "Create Go module".into(),
                    description: "Initialize go.mod for the project".into(),
                    path: "go.mod".into(),
                    content: format!(
                        "module {}\n\ngo 1.22\n\nrequire github.com/spf13/cobra v1.0.0\n",
                        project_name
                    ),
                    overwrite: false,
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Abort,
                },
                // Устаревший `go get github.com/spf13/cobra/cobra` больше не
                // работает (пакет разделён): генератор ставится как отдельный
                // бинарь cobra-cli, а проект инициализируется его командой.
                Step::Command {
                    id: "cobra_install".into(),
                    label: "Install Cobra CLI".into(),
                    description: "Install cobra-cli generator".into(),
                    command: "go".into(),
                    args: vec!["install".into(), "github.com/spf13/cobra-cli@latest".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(300),
                    condition: Some(StepCondition::FileExists {
                        path: go_mod_path.clone(),
                    }),
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                },
                Step::Command {
                    id: "cobra_init".into(),
                    label: "Init Cobra CLI".into(),
                    description: "Initialize Cobra CLI project".into(),
                    command: "cobra-cli".into(),
                    args: vec!["init".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(60),
                    condition: Some(StepCondition::FileExists { path: go_mod_path }),
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                },
                write_file(
                    cli_id,
                    cli_label,
                    cli_path,
                    &format!(
                        r#"package main

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
"#,
                        project_name, project_name
                    ),
                ),
            ]
        }

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
                description:
                    "Download Spring Boot starter from Initializr (validates HTTP response)".into(),
                generator_id: "spring-boot".into(),
                generator_config: serde_json::json!({
                    "project_name": project_name,
                    "dependencies": deps_str,
                }),
                policy: None,
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
        }

        // ==================== C# ====================
        // -o .: проект создаётся НЕ в вложенной папке <project_name>/,
        // а прямо в рабочей директории (backend/ или frontend/ при
        // сегментации) — иначе aspnetcore + maui давали test16/test16.
        "aspnetcore" => vec![cmd(
            "aspnet_new",
            "Create ASP.NET Core Web API",
            "Scaffold Web API project",
            "dotnet",
            vec!["new", "webapi", "-n", project_name, "-o", ".", "--force"],
        )],

        "maui" => vec![cmd(
            "maui_new",
            "Create MAUI app",
            "Scaffold .NET MAUI project",
            "dotnet",
            vec!["new", "maui", "-n", project_name, "-o", ".", "--force"],
        )],

        // ==================== C++ / Qt ====================
        // Qt — фреймворк с собственным UI-стеком: режим (QML/Widgets/
        // WebEngine/Kirigami) выбирается в мастере (answers["qt_ui"]) и
        // определяет, какие модули Qt подключить и какой main.cpp написать.
        "qt" => {
            let mode = qt_ui_mode(context);
            match mode {
                "qml" => qt_steps_qml(project_name),
                "kirigami" => qt_steps_kirigami(project_name),
                "webengine" => qt_steps_webengine(project_name, context, seg),
                _ => qt_steps_widgets(project_name),
            }
        }

        // Варианты UI Qt — генерируются внутри блока "qt" (см. qt_ui_mode);
        // отдельные шаги не нужны, чтобы не дублировать файлы.
        "qt-qml" | "qt-widgets" | "qt-webengine" | "qt-kirigami" => vec![],

        // ==================== Dart ====================
        "flutter" => {
            // flutter create требует имя без дефиса (валидный Dart-пакет) —
            // project_name передаётся флагом --project-name, а сам CLI
            // работает ВНУТРИ каталога назначения с "." (способность
            // creates_in_current_directory): раньше `flutter create <name>`
            // создавал вложенную папку <name>/ (матрёшка frontend/<name>/),
            // и пост-условие pubspec.yaml в каталоге назначения не
            // выполнялось. Результат валидируется пост-условиями
            // pubspec.yaml + lib/.
            // Префлайт flutter --version ОБЯЗАТЕЛЕН (Abort): отсутствующий
            // flutter останавливает пайплайн с понятной причиной, а не
            // маскируется скипом каркаса («Скип не равно успех»).
            let safe_name = project_name.replace('-', "_");
            vec![
                Step::Command {
                    id: "flutter_preflight".into(),
                    label: "Check Flutter SDK".into(),
                    description: "Verify the Flutter SDK is installed (flutter --version) before scaffolding the app".into(),
                    command: "flutter".into(),
                    args: vec!["--version".into()],
                    working_dir: Some(project_path.to_string()),
                    env: None,
                    timeout_secs: Some(60),
                    condition: None,
                    on_error: ErrorMode::Abort,
                    interactive: vec![],
                },
                scaffold_step(
                    "flutter_create",
                    "Create Flutter project",
                    "Scaffold Flutter app",
                    "flutter",
                    vec!["create", "--project-name", &safe_name, SCAFFOLD_TARGET],
                    ScaffoldCapability::CreatesInCurrentDirectory,
                    "frontend",
                    ScaffoldExtras::default()
                        .expects(&["pubspec.yaml", "lib"])
                        .policy(FilePolicy::SkipIfExists),
                ),
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
        }

        "ktor" => vec![
            // Пишем В Application.kt, а в Main.kt: kotlin language-скаффолд
            // создаёт src/main/kotlin/Main.kt с main() — второй main()
            // в Application.kt не дал бы проекту собраться. Inplace-фреймворк
            // перезаписывает заглушку (overwrite выставляется в steps_for_framework).
            write_file(
                "ktor_main",
                "Create Ktor entry",
                "src/main/kotlin/Main.kt",
                &format!(
                    r#"import io.ktor.application.*
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
"#,
                    project_name
                ),
            ),
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
            // перенос в backend/ или корень. Общий PHP/Composer-префлайт
            // (preflight.rs) идёт ДО скаффолда и Abort-ит без Composer.
            let composer_path = match seg {
                Some(dir) => format!("{}/composer.json", dir),
                None => "composer.json".to_string(),
            };
            vec![
                preflight::php_preflight_step(
                    "laravel_php_check",
                    "Check PHP and Composer for Laravel",
                    "Verify PHP (version, php.ini, extension_dir, fileinfo) and Composer availability before composer create-project",
                    "laravel/laravel",
                ),
                composer_scaffold_step("laravel_new", "Create Laravel project",
                "Scaffold Laravel application via PHP Composer",
                "laravel/laravel"),
                // Каркас обязан содержать валидный composer.json с
                // laravel/framework — не generic-PHP fallback.
                preflight::manifest_check_step(
                    "laravel_composer_check",
                    "Validate Laravel composer.json",
                    &composer_path,
                    "composer_json",
                    &["laravel/framework"],
                ),
            ]
        }

        "symfony" => {
            // CLI Override (PHP Composer): локальный бинарь symfony не
            // требуется, npm не используется — только composer с
            // абсолютным путём (см. composer_launch). Общий PHP/Composer-
            // префлайт — preflight.rs.
            let composer_path = match seg {
                Some(dir) => format!("{}/composer.json", dir),
                None => "composer.json".to_string(),
            };
            vec![
                preflight::php_preflight_step(
                    "symfony_php_check",
                    "Check PHP and Composer for Symfony",
                    "Verify PHP (version, php.ini, extension_dir, fileinfo) and Composer availability before composer create-project",
                    "symfony/skeleton",
                ),
                composer_scaffold_step("symfony_new", "Create Symfony project",
                "Scaffold Symfony application via PHP Composer",
                "symfony/skeleton"),
                // symfony/skeleton обязан содержать symfony/framework-bundle
                // в composer.json — не generic-PHP fallback.
                preflight::manifest_check_step(
                    "symfony_composer_check",
                    "Validate Symfony composer.json",
                    &composer_path,
                    "composer_json",
                    &["symfony/framework-bundle"],
                ),
            ]
        }

        // ==================== Swift ====================
        "swiftui" => vec![cmd(
            "swiftui_hint",
            "SwiftUI project hint",
            "SwiftUI projects are created through Xcode",
            "echo",
            vec!["Create this project through Xcode with SwiftUI template"],
        )],

        "vapor" => vec![cmd_i(
            "vapor_new",
            "Create Vapor project",
            "Scaffold Vapor application",
            "vapor",
            vec!["new", project_name],
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
            ],
        )],

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
                    &format!(
                        r#"const std = @import("std");

pub fn run() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}} CLI!\n", .{{"{}"}});
}}
"#,
                        project_name
                    ),
                )]
            } else {
                vec![write_file(
                    "zig_main",
                    "Create Zig CLI entry",
                    "src/main.zig",
                    &format!(
                        r#"const std = @import("std");

pub fn main() !void {{
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Hello from {{s}}!\n", .{{"{}"}});
}}
"#,
                        project_name
                    ),
                )]
            }
        }

        "zap" => {
            // Zig-веб: zap через zig fetch (зависимость в build.zig.zon).
            // В связке с zig-cli main.zig получает диспетчер «<app> cli».
            let has_cli = context.frameworks.iter().any(|f| f == "zig-cli");
            let main_zig = if has_cli {
                format!(
                    r#"const std = @import("std");
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
"#,
                    project_name
                )
            } else {
                format!(
                    r#"const std = @import("std");
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
"#,
                    project_name
                )
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
                write_file(
                    "zap_zon",
                    "Create build.zig.zon",
                    "build.zig.zon",
                    &format!(
                        r#".{{
    .name = .{pkg_name},
    .version = "0.1.0",
    .minimum_zig_version = "0.14.0",
    .paths = .{{""}},
    .fingerprint = 0x{fingerprint:016x},
    .dependencies = .{{}},
}}
"#
                    ),
                ),
                // `zig init` оставляет build.zig без модуля zap — проект с
                // @import("zap") в main.zig не собрался бы. Переписываем
                // build.zig (inplace-фреймворк → overwrite=true) с
                // подключением зависимости.
                write_file(
                    "zap_build",
                    "Create build.zig",
                    "build.zig",
                    &format!(
                        r#"const std = @import("std");

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
"#,
                        project_name
                    ),
                ),
                cmd(
                    "zap_fetch",
                    "Add Zap dependency",
                    "Fetch zap and save to build.zig.zon",
                    "zig",
                    vec![
                        "fetch",
                        "--save",
                        "https://github.com/zigzap/zap/archive/refs/tags/v0.10.1.tar.gz",
                    ],
                ),
                write_file(
                    "zap_main",
                    "Create Zap server entry",
                    "src/main.zig",
                    &main_zig,
                ),
            ]
        }

        // ==================== Elixir ====================
        "phoenix" => vec![cmd_i(
            "phoenix_new",
            "Create Phoenix project",
            "Scaffold Phoenix application",
            "mix",
            vec!["phx.new", project_name],
            vec![
                InteractiveEntry {
                    trigger: "Fetch and install dependencies?".into(),
                    response_type: ResponseType::Confirm(true),
                },
                InteractiveEntry {
                    trigger: "Would you like to build assets?".into(),
                    response_type: ResponseType::Confirm(true),
                },
            ],
        )],

        _ => vec![cmd(
            "fw_unknown",
            "Unknown framework",
            &format!("Framework '{}' has no specific setup steps", fw),
            "echo",
            vec![&format!(
                "No automated setup available for framework: {}",
                fw
            )],
        )],
    }
}

/// Каталог сегмента, где живёт python-код проекта: "backend" в
/// моно-репозитории (backend + frontend), "." — корень проекта.
/// Именно рядом с ним лежит requirements.txt и создаётся venv.
fn python_segment_dir(context: &WizardContext) -> String {
    if context.languages.iter().any(|l| l == "python") {
        if let Some(seg) = ProjectLayout::compute(context).language_dir("python") {
            return seg;
        }
    }
    ".".to_string()
}

/// Каталог, где лежит package.json JS-части проекта (для dep-патчей и
/// пост-валидации prisma/drizzle): сначала каталог JS-фреймворка, затем
/// каталог JS-языка (None — JS-части в проекте нет).
fn js_manifest_dir(context: &WizardContext) -> Option<String> {
    let layout = ProjectLayout::compute(context);
    for fw in &context.frameworks {
        if is_js_framework(fw) {
            return Some(layout.framework_dir(fw).unwrap_or_else(|| ".".to_string()));
        }
    }
    for lang in &context.languages {
        if matches!(lang.as_str(), "typescript" | "javascript") {
            return Some(layout.language_dir(lang).unwrap_or_else(|| ".".to_string()));
        }
    }
    None
}

/// Интерпретатор Python: `python` на Windows (в PATH у установщиков и
/// StackPilot Toolchain), `python3` на unix (дистрибутивный). ВСЕ
/// python-шаги пайплайна используют ровно этот выбор — никаких жёстко
/// зашитых "python" там, где возможен unix.
fn python_command() -> &'static str {
    if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    }
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
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    };

    // Каталог python-кода (backend/ в моно-репозитории, иначе корень) —
    // нужен шагам alembic ниже. Канонический venv + установка манифеста
    // выполняются в compose_recipe ДО всех framework-шагов (preflight.rs) —
    // здесь окружение не создаётся и не конкурирует с ним.
    let python_dir = python_segment_dir(context);

    for tool_id in tools {
        match tool_id.as_str() {
            // Database tools
            "sqlalchemy" => {
                steps.push(write_file(
                    "sqlalchemy_config",
                    "SQLAlchemy config",
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
"#,
                ));
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
                    // Рабочая директория — КАТАЛОГ python-сегмента (backend/
                    // в моно-репозитории, корень в монолите): alembic init
                    // создаёт migrations/ рядом с venv и requirements.txt,
                    // а не в корне проекта.
                    working_dir: Some(if python_dir == "." {
                        project_path.to_string()
                    } else {
                        format!(
                            "{}/{}",
                            project_path.trim_end_matches(['/', '\\']),
                            python_dir
                        )
                    }),
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
                // Prisma-зависимости ДО init: init читает package.json и
                // добавляет prisma-скрипты; патч гарантирует, что prisma и
                // @prisma/client попадут в npm install финальной фазы (а не
                // только транзитно через npx). Патч применяется к package.json
                // JS-сегмента, когда он существует (FileExists-условие).
                let prisma_js_dir = js_manifest_dir(context);
                if let Some(dir) = &prisma_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_patch_step(
                        "prisma_deps",
                        "Add Prisma dependencies",
                        &pkg,
                        serde_json::json!({
                            "dependencies": { "@prisma/client": "^6.1.0" },
                            "devDependencies": { "prisma": "^6.1.0" }
                        })
                        .to_string(),
                    ));
                }
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
                    // Abort: выбранный инструмент обязан инициализироваться;
                    // невозможность запустить npx (нет npm) не маскируется
                    // молчаливым пропуском.
                    on_error: ErrorMode::Abort,
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
                    policy: None,
                    condition: None,
                    on_error: ErrorMode::Skip,
                });
                // Пост-валидация: prisma обязана быть задекларирована в
                // package.json JS-сегмента после init (Abort).
                if let Some(dir) = &prisma_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_check_step(
                        "prisma_deps_check",
                        "Validate Prisma dependencies",
                        &pkg,
                        "package_json",
                        &["@prisma/client", "prisma"],
                    ));
                }
            }
            "drizzle" => {
                steps.push(write_file(
                    "drizzle_config",
                    "Drizzle config",
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
"#,
                ));
                // drizzle-kit init требует установленные drizzle-kit и
                // drizzle-orm: патч декларирует их в package.json
                // JS-сегмента (MergeJson, только когда файл существует) и
                // пост-валидация подтверждает наличие после npm install.
                let drizzle_js_dir = js_manifest_dir(context);
                if let Some(dir) = &drizzle_js_dir {
                    let pkg = package_json_rel_path(Some(dir.as_str()));
                    steps.push(preflight::manifest_patch_step(
                        "drizzle_deps",
                        "Add Drizzle dependencies",
                        &pkg,
                        serde_json::json!({
                            "dependencies": { "drizzle-orm": "^0.36.0" },
                            "devDependencies": { "drizzle-kit": "^0.28.0" }
                        })
                        .to_string(),
                    ));
                    steps.push(preflight::manifest_check_step(
                        "drizzle_deps_check",
                        "Validate Drizzle dependencies",
                        &pkg,
                        "package_json",
                        &["drizzle-orm", "drizzle-kit"],
                    ));
                }
            }
            // Testing tools
            "pytest" => {
                steps.push(write_file(
                    "pytest_config",
                    "Pytest config",
                    "pytest.ini",
                    r#"[pytest]
testpaths = tests
python_files = test_*.py
python_classes = Test*
python_functions = test_*
addopts = -v --tb=short
"#,
                ));
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
                steps.push(write_file(
                    "ruff_config",
                    "Ruff config",
                    "ruff.toml",
                    r#"[lint]
select = ["E", "F", "I", "N", "W"]
ignore = []

[format]
quote-style = "double"
indent-style = "space"
"#,
                ));
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
                steps.push(write_file(
                    "airflow_example_dag",
                    "Example Airflow DAG",
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
"#,
                ));
            }
            // Infra tools — сервисы docker-compose; переменные окружения
            // собираем в один .env.example в конце (иначе каждый следующий
            // инструмент видел бы существующий файл и шаг скипался).
            // Инструменты из local_infra_tools поставлены локально:
            // для них в .env.example — локальные адреса (localhost),
            // а из docker-compose.yaml они исключаются (steps_for_docker).
            "postgresql" | "redis" | "mongodb" | "mysql" | "kafka" | "clickhouse" | "rabbitmq"
            | "minio" | "mailpit" => {
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
                steps.push(write_file(
                    &format!("config_{}", tool_id),
                    &format!("Config hint for {}", tool_id),
                    &format!("config/{}.md", tool_id),
                    &format!(
                        "# {} configuration\n\nSee documentation for setup details.\n",
                        tool_id
                    ),
                ));
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

/// Фаза 5 (часть): Docker-шаблоны пишутся ПОСЛЕ всех CLI-фреймворков и
/// перезаписывают их версии (overwrite=true). Каталоги берутся из
/// канонической раскладки: в split-проекте Dockerfile и .dockerignore живут
/// ВНУТРИ backend/ (там серверное приложение, и docker-compose собирает
/// контекст ./backend), в integrated/одно-сторонних — в корне.
fn steps_for_docker(
    layout: &ProjectLayout,
    context: &WizardContext,
    _project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    if context.docker == false {
        return Vec::new();
    }

    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    // Split: сервер живёт в backend/ — Dockerfile собирается оттуда.
    let app_dir = layout
        .eager_dirs()
        .iter()
        .find(|d| *d == "backend")
        .map(|d| format!("{}/", d))
        .unwrap_or_default();

    let mut result = Vec::new();

    if let Some(dockerfile_content) =
        content::generate_dockerfile_content(primary_lang, primary_fw, project_name)
    {
        result.push(Step::WriteFile {
            id: "dockerfile".into(),
            label: "Create Dockerfile".into(),
            description: format!("Create Dockerfile for {} + {:?}", primary_lang, primary_fw),
            path: format!("{}Dockerfile", app_dir),
            content: dockerfile_content,
            overwrite: true,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        })
    };
    result.push(Step::WriteFile {
        id: ("docker_ignore".into()),
        label: ("Create .dockerignore".into()),
        description: ("Generate .dockerignore file".into()),
        path: (format!("{}.dockerignore", app_dir)),
        content: (content::dockerignore_content(primary_lang)),
        overwrite: (true),
        policy: (None),
        condition: (None),
        on_error: (ErrorMode::Skip),
    });

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
        result.push(Step::WriteFile {
            id: "docker_compose".into(),
            label: "Create docker-compose".into(),
            description: "Generate docker-compose file".into(),
            path: "docker-compose.yaml".into(),
            content: content::generate_docker_compose(&services, project_name, app_port, &app_dir),
            overwrite: true,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
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

    // git init. Повторный запуск рецепта: .git/HEAD уже есть — шаг
    // пропускается (FileNotExists), git не переинициализируется.
    steps.push(Step::Command {
        id: "git_init".into(),
        label: "Initialize Git repository".into(),
        description: "Run git init".into(),
        command: "git".into(),
        args: vec!["init".into()],
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(10),
        condition: Some(StepCondition::FileNotExists {
            path: ".git/HEAD".into(),
        }),
        on_error: ErrorMode::Skip,
        interactive: vec![],
    });

    steps
}

/// Команда git commit, идемпотентная при повторном запуске рецепта:
/// коммитит только когда в индексе есть изменения (`git diff --cached`),
/// и всегда завершается успешно (exit 0) — «нечего коммитить» не ошибка.
/// Git-сообщение экранируется под оболочку платформы.
fn git_commit_command(project_name: &str) -> String {
    let message = format!("Initial commit: {} project", project_name);
    if cfg!(windows) {
        let msg = message.replace('\'', "''");
        format!("git diff --cached --quiet; if (-not $?) {{ git commit -m '{msg}' }}; exit 0")
    } else {
        let msg = message.replace('\'', "'\"'\"'");
        format!("git diff --cached --quiet || git commit -m \"{msg}\"; exit 0")
    }
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
        policy: None,
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
fn steps_for_finalize(
    context: &WizardContext,
    js_dirs: &[String],
    project_path: &str,
    project_name: &str,
) -> Vec<Step> {
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
            // Повторный запуск рецепта: node_modules уже на месте —
            // npm install пропускается (FileNotExists, путь от корня).
            condition: Some(StepCondition::FileNotExists {
                path: format!("{}/node_modules", if dir == "." { "." } else { dir }),
            }),
            // Abort: установка выбранных зависимостей (express, fastify,
            // telegraf, nest...) обязательна — провал не маскируется скипом.
            on_error: ErrorMode::Abort,
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
        description: "Commit staged files (no-op when nothing is staged)".into(),
        command: git_commit_command(project_name),
        args: vec![],
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

    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");
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
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    });

    steps
}

/// Текстовая карта структуры проекта для README — из канонической
/// раскладки (ProjectLayout): каталоги сегментов и файлы каждого фреймворка.
/// Текстовая карта структуры проекта для README — из канонической
/// раскладки (ProjectLayout): каталоги сегментов и файлы каждого фреймворка.
fn layout_structure_text(layout: &ProjectLayout, context: &WizardContext) -> String {
    // каталог → фреймворки, чьи файлы там лежат
    let mut by_dir: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for placement in layout.to_summary(context).framework_placement {
        let dir = if placement.directory == "." {
            ".".to_string()
        } else {
            placement.directory.clone()
        };
        by_dir.entry(dir).or_default().push(placement.framework);
    }
    if let Some(owner) = &layout.root_owner {
        by_dir
            .entry("src-tauri".to_string())
            .or_default()
            .push(owner.clone());
    }
    if by_dir.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for (dir, fws) in by_dir {
        lines.push(format!("├── {:<12} # {}", dir, fws.join(" + ")));
    }
    lines.join("\n")
}

fn steps_for_readme(
    layout: &ProjectLayout,
    context: &WizardContext,
    _project_path: &str,
    project_name: &str,
) -> Vec<Step> {
    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");
    let primary_fw = context.frameworks.first().map(|s| s.as_str());
    let project_type = context.project_type.as_deref().unwrap_or("project");

    // Структура проекта в README — из канонической раскладки: каталоги
    // сегментов и куда ложится каждый фреймворк.
    let structure = layout_structure_text(layout, context);

    let readme = content::generate_readme(
        project_name,
        primary_lang,
        primary_fw,
        project_type,
        &context.tools,
        &structure,
    );

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
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    }]
}

fn steps_for_vscode(layout: &ProjectLayout, context: &WizardContext) -> Vec<Step> {
    if !context.vscode_config {
        return Vec::new();
    }

    let primary_lang = context
        .languages
        .first()
        .map(|s| s.as_str())
        .unwrap_or("python");

    // Каталоги для слияния: корень всегда + сегменты канонической раскладки
    // + каталоги scaffold-генераторов (frontend/ в монолите). В не-корневых
    // каталогах генератор "vscode-merge" примешивает конфиг только если
    // .vscode/settings.json уже создал сам CLI (create-next-app и т.п.) —
    // лишние папки не дублируются.
    let mut dirs: Vec<String> = vec![".".to_string()];
    for dir in layout.eager_dirs() {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    for dir in [
        layout.backend_dir.as_deref(),
        layout.frontend_dir.as_deref(),
    ] {
        if let Some(dir) = dir {
            if !dirs.contains(&dir.to_string()) {
                dirs.push(dir.to_string());
            }
        }
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
        policy: None,
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

    fn recipe_for(ctx: &WizardContext, folder_name: &str) -> Result<Recipe, String> {
        compose_recipe(&ProjectLayout::compute(ctx), ctx, folder_name)
    }

    fn cmd_args(step: &Step) -> Vec<String> {
        match step {
            Step::Command { args, .. } => args.clone(),
            other => panic!("ожидался Command, получили {:?}", other.id()),
        }
    }

    fn gen_args(config: &serde_json::Value) -> Vec<String> {
        config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn gen_strs(config: &serde_json::Value, key: &str) -> Vec<String> {
        config
            .get(key)
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
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
        let steps = steps_for_framework(
            "zap",
            "C:\\dev\\myapp",
            "my_app",
            &context(),
            &ProjectLayout::compute(&context()),
        );
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
        let steps = steps_for_framework(
            "zap",
            "C:\\dev\\myapp",
            "my_app",
            &context(),
            &ProjectLayout::compute(&context()),
        );
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

    /// Android-контекст: язык явно назначен фронтенд-стороне (мобильный
    /// клиент), поэтому каноническая раскладка — FrontendOnly, и android
    /// пишет файлы в корень (каталоги не сегментируются). Тесты проверяют
    /// СОДЕРЖИМОЕ gradle-файлов, а не маршрутизацию: маршрут android →
    /// frontend/ в split-стеках покрыт тестами ProjectLayout.
    fn android_context(frameworks: &[&str], languages: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frontend_languages: languages.iter().map(|s| s.to_string()).collect(),
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
                Step::WriteFile {
                    path: p, content, ..
                } if p == path => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("нет шага записи {path}"))
    }

    #[test]
    fn android_generates_full_gradle_project() {
        let ctx = android_context(&["android"], &["kotlin"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        for expected in [
            "settings.gradle.kts",
            "build.gradle.kts",
            "gradle.properties",
            "app/build.gradle.kts",
            "app/src/main/AndroidManifest.xml",
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        ] {
            assert!(
                write_paths(&steps).contains(&expected),
                "нет файла {expected}"
            );
        }
        // без jetpack-compose — никакого compose-плагина и compose-импортов
        assert!(!write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(!main.contains("androidx.compose"), "{main}");
        assert!(main.contains("android.app.Activity"), "{main}");
        // манифест: метка проекта экранирована, тема — платформенная (без res/)
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(manifest.contains("android:label=\"myapp\""), "{manifest}");
    }

    #[test]
    fn android_with_java_language_generates_java_activity() {
        let ctx = android_context(&["android"], &["java"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let main = write_content(
            &steps,
            "app/src/main/java/com/example/app/MainActivity.java",
        );
        assert!(
            main.contains("public class MainActivity extends Activity"),
            "{main}"
        );
        let app_build = write_content(&steps, "app/build.gradle.kts");
        assert!(!app_build.contains("org.jetbrains.kotlin"), "{app_build}");
    }

    #[test]
    fn android_with_jetpack_compose_generates_compose_project() {
        let ctx = android_context(&["android", "jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let root = write_content(&steps, "build.gradle.kts");
        assert!(
            root.contains("org.jetbrains.kotlin.plugin.compose"),
            "{root}"
        );
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("setContent"), "{main}");
        assert!(main.contains("androidx.compose.material3"), "{main}");
        // jetpack-compose рядом с android не пишет свои файлы (иначе —
        // дубликаты путей, см. duplicate_framework_write_paths)
        let compose_steps = steps_for_framework(
            "jetpack-compose",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(compose_steps.is_empty(), "{compose_steps:?}");
    }

    #[test]
    fn jetpack_compose_standalone_generates_full_project() {
        let ctx = android_context(&["jetpack-compose"], &["kotlin"]);
        let steps = steps_for_framework(
            "jetpack-compose",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(write_paths(&steps).contains(&"app/build.gradle.kts"));
        assert!(write_content(&steps, "build.gradle.kts").contains("plugin.compose"));
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("setContent"), "{main}");
    }

    #[test]
    fn android_files_escape_user_text() {
        let ctx = WizardContext {
            project_name: Some("My \"App\" $v1".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: vec!["kotlin".into()],
            frontend_languages: vec!["kotlin".into()],
            frameworks: vec!["android".into()],
            ..Default::default()
        };
        let steps = steps_for_framework(
            "android",
            "C:\\dev\\myapp",
            "My \"App\" $v1",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let manifest = write_content(&steps, "app/src/main/AndroidManifest.xml");
        assert!(
            manifest.contains("android:label=\"My &quot;App&quot; $v1\""),
            "{manifest}"
        );
        let main = write_content(
            &steps,
            "app/src/main/kotlin/com/example/app/MainActivity.kt",
        );
        assert!(main.contains("Hello from My \\\"App\\\" \\$v1!"), "{main}");
    }

    #[test]
    fn frontend_frameworks_use_project_subfolder() {
        // Фронтенды скаффолдятся в frontend/ (ScaffoldGenerator), а не в
        // корне: иначе они перезапишут package.json бэкенда (express+nextjs
        // и т.п.). solidjs остаётся обычным Command, создающим подпапку
        // <project_name>.
        for fw_id in ["nextjs", "nuxt", "sveltekit", "expo", "react"] {
            let steps = steps_for_framework(
                fw_id,
                "C:\\dev\\myapp",
                "myapp",
                &context(),
                &ProjectLayout::compute(&context()),
            );
            let scaffold = steps
                .iter()
                .find(|s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"))
                .unwrap_or_else(|| panic!("{fw_id}: scaffold-шаг должен быть в плане"));
            match scaffold {
                Step::Generate {
                    generator_config, ..
                } => {
                    assert_eq!(
                        generator_config.get("target_dir").and_then(|v| v.as_str()),
                        Some("frontend"),
                        "{fw_id} должен скаффолдиться в frontend/: {generator_config}"
                    );
                }
                _ => panic!("{fw_id}: scaffold — Generate"),
            }
        }

        // solidjs — creates_project_and_may_prompt (create-solid может
        // задавать вопросы), каркас кладётся в frontend/ через плейсхолдер,
        // пост-условие — package.json.
        let steps = steps_for_framework(
            "solidjs",
            "C:\\dev\\myapp",
            "myapp",
            &context(),
            &ProjectLayout::compute(&context()),
        );
        let create = steps
            .iter()
            .find(|s| s.id() == "solid_init")
            .unwrap_or_else(|| panic!("solidjs: шаг solid_init должен быть в плане"));
        match create {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_project_and_may_prompt"),
                    "{generator_config}"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "solidjs скаффолдится в frontend/: {generator_config}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["package.json"],
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                assert!(
                    args.contains(&"__TARGET__".to_string()),
                    "create-solid получает имя проекта через плейсхолдер: {args:?}"
                );
            }
            _ => panic!("solid_init — Generate scaffold"),
        }
    }

    #[test]
    fn split_layout_puts_frameworks_into_segments() {
        // nextjs + fastapi: фронтенд — в frontend/, сервер — в backend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let next_steps = steps_for_framework(
            "nextjs",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        let scaffold = next_steps
            .iter()
            .find(
                |s| matches!(s, Step::Generate { generator_id, .. } if generator_id == "scaffold"),
            )
            .expect("nextjs: scaffold-шаг должен быть в плане");
        match scaffold {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
            }
            _ => panic!("nextjs — Generate"),
        }

        let api_steps = steps_for_framework(
            "fastapi",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(
            api_steps.iter().any(
                |s| matches!(s, Step::WriteFile { path, .. } if path == "backend/src/main.py")
            ),
            "fastapi должен писать в backend/src/main.py"
        );
    }

    #[test]
    fn solo_backend_framework_stays_in_root() {
        // Только fastapi (без фронтенда) — сегментации нет, файлы в корне
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];

        let api_steps = steps_for_framework(
            "fastapi",
            "C:\\dev\\myapp",
            "myapp",
            &ctx,
            &ProjectLayout::compute(&ctx),
        );
        assert!(
            api_steps
                .iter()
                .any(|s| matches!(s, Step::WriteFile { path, .. } if path == "src/main.py")),
            "fastapi без фронтенда пишет в корень"
        );
    }

    #[test]
    fn compose_recipe_creates_segment_dirs_for_split_stack() {
        // Полный план для nextjs + fastapi: создаются папки backend/ и frontend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "python".into()];
        ctx.frameworks = vec!["nextjs".into(), "fastapi".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "backend/ и frontend/ должны создаваться: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "backend/ и frontend/ должны создаваться: {mkdirs:?}"
        );
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );

        // express-файлы пишутся в backend/ (своя сторона), а не в frontend/
        for (step_id, expected_path) in [
            ("express_index", "backend/src/index.js"),
            ("express_package", "backend/package.json"),
        ] {
            let step = recipe
                .steps
                .iter()
                .find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::WriteFile {
                    path, overwrite, ..
                } => {
                    assert_eq!(
                        path, expected_path,
                        "{step_id} должен писать в {expected_path}"
                    );
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "нужны backend/ и frontend/: {mkdirs:?}"
        );

        // aspnetcore (dotnet new webapi) выполняется в backend/
        let asp = recipe
            .steps
            .iter()
            .find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command { working_dir, .. } => {
                let wd = working_dir
                    .as_deref()
                    .expect("aspnetcore должен работать в backend/");
                assert!(
                    wd.ends_with("backend"),
                    "aspnetcore должен работать в backend/, а не в корне: {wd}"
                );
            }
            _ => panic!("aspnet_new — Command"),
        }

        // nextjs (create-next-app) — scaffold-генератор: каталогом становится
        // сегмент frontend/ (ScaffoldGenerator выполнит CLI с "." внутри)
        let next = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Generate {
                generator_config, ..
            } => {
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Обе стороны: backend/ и frontend/ создаются движком
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "aspnetcore обязан получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "maui обязан получить frontend/: {mkdirs:?}"
        );

        // dotnet new webapi: работает в backend/ с -o . — без вложенной папки
        let asp = recipe
            .steps
            .iter()
            .find(|s| s.id() == "aspnet_new")
            .expect("aspnet_new должен быть в плане");
        match asp {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "webapi обязан работать внутри ./backend"
                );
                assert!(args.contains(&"-o".to_string()) && args.contains(&".".to_string()),
                    "webapi обязан идти с -o . (проект прямо в backend/, без test16/test16): {args:?}");
                assert!(args.contains(&"-n".to_string()), "{args:?}");
            }
            _ => panic!("aspnet_new — Command"),
        }

        // dotnet new maui: работает в frontend/ с -o .
        let maui = recipe
            .steps
            .iter()
            .find(|s| s.id() == "maui_new")
            .expect("maui_new должен быть в плане");
        match maui {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/frontend"),
                    "maui обязан работать внутри ./frontend"
                );
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

        let recipe = recipe_for(&ctx, "my-test-app").expect("recipe must build");

        let django = recipe
            .steps
            .iter()
            .find(|s| s.id() == "django_start")
            .expect("django_start должен быть в плане");
        match django {
            Step::Command {
                args, working_dir, ..
            } => {
                assert!(
                    args.contains(&"my_test_app".to_string()),
                    "имя проекта санитизируется (дефис → подчёркивание): {args:?}"
                );
                assert!(
                    args.contains(&".".to_string()),
                    "startproject создаёт проект в текущем каталоге: {args:?}"
                );
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)"
                );
            }
            _ => panic!("django_start — Command"),
        }
    }

    #[test]
    fn tauri_pipeline_scaffolds_frontend_init_and_patches_config() {
        // Integrated-раскладка: tauri (side="either" && scaffold="root") —
        // владелец корня. Rust остаётся в корне рядом с оболочкой, react
        // скаффолдится в frontend/; затем npx @tauri-apps/cli init в корне
        // (пути на ../frontend/dist) и Rust-патч src-tauri/tauri.conf.json.
        // create-tauri-app убран из пайплайна (его фронтенд-каркас в корне
        // был пустым без node_modules). Движок папки backend//frontend/ НЕ
        // предсоздаёт: frontend/ появляется из скаффолда react.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.backend_languages = vec!["rust".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

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
        // Мандат integrated: движок НЕ предсоздаёт backend//frontend/
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !mkdirs.iter().any(|d| d == "backend" || d == "frontend"),
            "integrated: движок не создаёт сегментные папки, их создают скаффолдеры: {mkdirs:?}"
        );
        // Компаньон react скаффолдится отдельно (не подавляется tauri)
        let react_scaffold = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match react_scaffold {
            Step::Generate {
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "react скаффолдится в frontend/: {generator_config}"
                );
                assert_eq!(on_error, &ErrorMode::Skip);
            }
            _ => panic!("vite_create — Generate"),
        }

        // Фронтенд-генератор выполняется ПЕРВЫМ — tauri init откладывается
        // в конец фазы скаффолдинга (движок откладывает tauri-шаги).
        let vite_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "vite_create")
            .unwrap();
        let init_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        assert!(vite_idx < init_idx, "фронтенд скаффолдится ДО tauri init");
        match &recipe.steps[init_idx] {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                // Способность generates_root_shell: tauri init раскладывает
                // shell в текущем каталоге, каталог проекта не создаёт.
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell"),
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                assert!(args.iter().any(|a| a == "init"), "{args:?}");
                assert!(
                    args.iter().any(|a| a == "--ci"),
                    "init должен быть неинтерактивным: {args:?}"
                );
                let dist_idx = args
                    .iter()
                    .position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "из корня путь на frontend/dist — ../frontend/dist: {args:?}"
                );
                assert_eq!(
                    args[args
                        .iter()
                        .position(|a| a == "--before-dev-command")
                        .unwrap()
                        + 1],
                    "npm --prefix frontend run dev",
                    "{args:?}"
                );
                assert_eq!(
                    args[args
                        .iter()
                        .position(|a| a == "--before-build-command")
                        .unwrap()
                        + 1],
                    "npm --prefix frontend run build",
                    "{args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне (integrated): {generator_config}"
                );
                // Пост-условие: tauri.conf.json И Cargo.toml обязаны появиться
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                    "{generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate scaffold"),
        }

        // Rust-патч tauri.conf.json — сразу после tauri init, в корне
        let patch_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "tauri_config_patch")
            .expect("tauri_config_patch должен быть в плане");
        assert!(init_idx < patch_idx, "патч конфига идёт после tauri init");
        match &recipe.steps[patch_idx] {
            Step::Generate {
                generator_id,
                generator_config,
                condition,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("identifier").and_then(|v| v.as_str()),
                    Some("com.myapp")
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dir")
                        .and_then(|v| v.as_str()),
                    Some("frontend")
                );
                assert_eq!(
                    generator_config.get("tauri_dir").and_then(|v| v.as_str()),
                    Some(""),
                    "конфиг живёт в src-tauri/ в корне: {generator_config}"
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dist")
                        .and_then(|v| v.as_str()),
                    Some("../frontend/dist"),
                    "frontendDist из корня — ../frontend/dist: {generator_config}"
                );
                // Патч выполняется только если tauri init создал конфиг
                // (пост-условие скаффолда) — вторичных ENOENT-ошибок нет.
                assert_eq!(
                    condition,
                    &Some(StepCondition::FileExists {
                        path: "src-tauri/tauri.conf.json".into()
                    }),
                    "патч конфига зависит от пост-условия tauri init"
                );
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
        let installs: Vec<_> = recipe
            .steps
            .iter()
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
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "package.json лежит в frontend/"
                );
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
        // npm install внутри frontend/, затем tauri init В КОРНЕ (integrated:
        // tauri — владелец корня, frontendDist — ../frontend/dist). Generic
        // js/ts-скаффолд в корне подавлен (его заглушки конфликтовали бы
        // с tauri-каркасом).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        assert!(
            !recipe.steps.iter().any(|s| s.id() == "package_json"),
            "generic js-скаффолд не нужен: фронтенд создаёт vite-vanilla"
        );
        let web = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_web_scaffold")
            .expect("tauri_web_scaffold должен быть в плане");
        match web {
            Step::Generate {
                generator_config, ..
            } => {
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("vanilla-ts"),
                    "typescript → vanilla-ts"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
            }
            _ => panic!("tauri_web_scaffold — Generate"),
        }

        // npm install — явный шаг внутри frontend/ (компаньона нет, финальная
        // фаза про tauri-фронтенд не знает)
        let install = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_web_install")
            .expect("tauri_web_install должен быть в плане");
        match install {
            Step::Command {
                working_dir,
                command,
                ..
            } => {
                assert_eq!(command, "npm");
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "install работает внутри frontend/ без join-сегмента: {working_dir:?}"
                );
            }
            _ => panic!("tauri_web_install — Command"),
        }

        // Фронтенд-генератор → npm install → tauri init → патч конфига
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("tauri_web_scaffold") < idx("tauri_web_install"),
            "скаффолд до install"
        );
        assert!(
            idx("tauri_web_install") < idx("tauri_init"),
            "install до tauri init"
        );

        match &recipe.steps[idx("tauri_init")] {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell"),
                    "{generator_config}"
                );
                let args = gen_args(generator_config);
                let dist_idx = args
                    .iter()
                    .position(|a| a == "--frontend-dist")
                    .expect("--frontend-dist обязан быть в args");
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "из корня путь на frontend/dist — ../frontend/dist: {args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне (integrated): {generator_config}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"],
                    "{generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate scaffold"),
        }

        match &recipe.steps[idx("tauri_config_patch")] {
            Step::Generate {
                generator_id,
                generator_config,
                condition,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "tauri-config");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("tauri_dir").and_then(|v| v.as_str()),
                    Some("")
                );
                assert_eq!(
                    generator_config
                        .get("frontend_dist")
                        .and_then(|v| v.as_str()),
                    Some("../frontend/dist")
                );
                assert_eq!(
                    condition,
                    &Some(StepCondition::FileExists {
                        path: "src-tauri/tauri.conf.json".into()
                    }),
                    "патч конфига зависит от пост-условия tauri init"
                );
            }
            _ => panic!("tauri_config_patch — Generate"),
        }

        // Финальная фаза про tauri-фронтенд не знает: install уже сделан
        // явным шагом, новых npm_install в финале нет
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            0,
            "фронтенд tauri ставится явным шагом, финальных npm_install быть не должно"
        );

        // Патч имени package.json для tauri — в frontend/
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_pkg_name")
            .expect("tauri_pkg_name должен быть в плане");
        match patch {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "package.json лежит в frontend/"
                );
            }
            _ => panic!("tauri_pkg_name — Command"),
        }
    }

    #[test]
    fn tauri_with_svelte_companion_keeps_shell_in_root() {
        // tauri + svelte: integrated — оболочка остаётся в корне, svelte
        // (vite-компаньон) скаффолдится в frontend/ отдельным шагом, движок
        // не предсоздаёт сегментные папки. tauri init — в корне.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !mkdirs.iter().any(|d| d == "backend" || d == "frontend"),
            "integrated: сегментные папки не предсоздаются: {mkdirs:?}"
        );

        let svelte = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("svelte скаффолдится через vite_create");
        match svelte {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "svelte в frontend/: {generator_config}"
                );
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("svelte-ts"),
                    "{args:?}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        match init {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                let dist_idx = args.iter().position(|a| a == "--frontend-dist").unwrap();
                assert_eq!(
                    args[dist_idx + 1],
                    "../frontend/dist",
                    "tauri init в корне смотрит на ../frontend/dist: {args:?}"
                );
                assert!(
                    generator_config
                        .get("working_dir")
                        .and_then(|v| v.as_str())
                        .is_none()
                        || generator_config.get("working_dir").and_then(|v| v.as_str())
                            == Some("."),
                    "tauri init работает в корне: {generator_config}"
                );
            }
            _ => panic!("tauri_init — Generate"),
        }
    }

    #[test]
    fn layout_class_and_framework_placement_are_canonical() {
        // Каноническая раскладка: класс и каталоги фреймворков/языков —
        // единственное решение ProjectLayout::compute, и оно видно в
        // предпросмотре (LayoutSummary).
        let assert_placement =
            |ctx: &WizardContext, expected_class: &str, expected: &[(&str, &str)]| {
                let layout = ProjectLayout::compute(ctx);
                let summary = layout.to_summary(ctx);
                assert_eq!(
                    summary.class, expected_class,
                    "фреймворки: {:?}",
                    ctx.frameworks
                );
                assert_eq!(summary.generated_directories, layout.eager_dirs());
                for (fw, dir) in expected {
                    assert_eq!(
                        layout.framework_dir(fw).unwrap_or_else(|| ".".to_string()),
                        *dir,
                        "фреймворк {fw} должен лежать в {dir}"
                    );
                }
            };

        // nest + nextjs: обе стороны даже при единственном typescript → split
        let mut ctx = context();
        ctx.frameworks = vec!["nest".into(), "nextjs".into()];
        assert_placement(
            &ctx,
            "split",
            &[("nest", "backend"), ("nextjs", "frontend")],
        );

        // laravel + react: php + typescript → split
        let mut ctx = context();
        ctx.languages = vec!["php".into(), "typescript".into()];
        ctx.frameworks = vec!["laravel".into(), "react".into()];
        assert_placement(
            &ctx,
            "split",
            &[("laravel", "backend"), ("react", "frontend")],
        );

        // django + vue: python + typescript → split, django работает в backend/
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.frameworks = vec!["django".into(), "vue".into()];
        assert_placement(&ctx, "split", &[("django", "backend"), ("vue", "frontend")]);

        // tauri + svelte: integrated — оболочка владеет корнем, svelte в frontend/
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let layout = ProjectLayout::compute(&ctx);
        let summary = layout.to_summary(&ctx);
        assert_eq!(summary.class, "integrated");
        assert_eq!(summary.root_owner.as_deref(), Some("tauri"));
        assert_eq!(
            layout
                .framework_dir("tauri")
                .unwrap_or_else(|| ".".to_string()),
            "."
        );
        assert_eq!(
            layout
                .framework_dir("svelte")
                .unwrap_or_else(|| ".".to_string()),
            "frontend"
        );
        assert_eq!(
            layout
                .language_dir("rust")
                .unwrap_or_else(|| ".".to_string()),
            ".",
            "rust — язык оболочки, остаётся в корне"
        );
        assert!(
            layout.eager_dirs().is_empty(),
            "integrated не предсоздаёт сегментные папки"
        );

        // zig-cli + flutter: zig → backend (по zig-cli), dart → frontend (по flutter)
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "flutter".into()];
        assert_placement(
            &ctx,
            "split",
            &[("zig-cli", "backend"), ("flutter", "frontend")],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(
            layout
                .language_dir("zig")
                .unwrap_or_else(|| ".".to_string()),
            "backend"
        );
        assert_eq!(
            layout
                .language_dir("dart")
                .unwrap_or_else(|| ".".to_string()),
            "frontend"
        );

        // gin + solidjs: go → backend, solidjs → frontend
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["gin".into(), "solidjs".into()];
        assert_placement(
            &ctx,
            "split",
            &[("gin", "backend"), ("solidjs", "frontend")],
        );

        // fastapi один: backend-only, всё в корне, папки не создаются
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        assert_placement(&ctx, "backend-only", &[("fastapi", ".")]);

        // nextjs один: frontend-only, scaffold-генератор уходит в frontend/
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        assert_placement(&ctx, "frontend-only", &[("nextjs", "frontend")]);
    }

    #[test]
    fn docker_and_readme_follow_the_canonical_layout() {
        // Split (laravel + react + postgresql): Dockerfile Рё .dockerignore
        // живут в backend/, docker-compose собирает app из backend/. README
        // показывает каноническую структуру.
        let mut ctx = context();
        ctx.languages = vec!["php".into(), "typescript".into()];
        ctx.frameworks = vec!["laravel".into(), "react".into()];
        ctx.tools = vec!["postgresql".into()];
        ctx.docker = true;

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let dockerfile = recipe.steps.iter().find(|s| s.id() == "dockerfile");
        if let Some(Step::WriteFile { path, .. }) = dockerfile {
            assert_eq!(
                path, "backend/Dockerfile",
                "Dockerfile собирается из backend/"
            );
        }
        let ignore = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_ignore")
            .unwrap();
        match ignore {
            Step::WriteFile { path, .. } => assert_eq!(path, "backend/.dockerignore"),
            _ => panic!("docker_ignore — WriteFile"),
        }
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .unwrap();
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("build: backend/"),
                    "compose собирает app из backend/: {content}"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
        }
        let readme = recipe.steps.iter().find(|s| s.id() == "readme").unwrap();
        match readme {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("backend"),
                    "README описывает сегменты: {content}"
                );
                assert!(
                    content.contains("frontend"),
                    "README описывает сегменты: {content}"
                );
            }
            _ => panic!("readme — WriteFile"),
        }

        // BackendOnly (fastapi + postgresql): Dockerfile и compose — в корне
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["postgresql".into()];
        ctx.docker = true;

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let ignore = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_ignore")
            .unwrap();
        match ignore {
            Step::WriteFile { path, .. } => {
                assert_eq!(path, ".dockerignore", "backend-only: всё в корне")
            }
            _ => panic!("docker_ignore — WriteFile"),
        }
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .unwrap();
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("build: ."),
                    "backend-only: сборка из корня: {content}"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "django (root→subdir) получает backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "react получает frontend/: {mkdirs:?}"
        );

        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("django_start") < idx("vite_create"),
            "root CLI идёт до subdir-скаффолда"
        );
        assert!(
            idx("vite_create") < idx("prisma_init"),
            "subdir-скаффолд идёт до инструментов"
        );
        assert!(
            idx("prisma_init") < idx("docker_compose"),
            "инструменты идут до конфигов"
        );

        // django-admin startproject работает в backend/, а не в корне
        match &recipe.steps[idx("django_start")] {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "django стартует в backend/ (Strict Subdir Mandate)"
                );
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
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "vite скаффолдится в frontend/: {generator_config}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install ровно один раз — ВНУТРИ frontend/, в финальной фазе
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
            }
            _ => panic!("npm_install — Command"),
        }
        assert!(
            idx("npm_install_0") > idx("readme"),
            "npm install — в финальной фазе, после шаблонизации"
        );
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // Мандат: backend/ И frontend/ создаются движком ДО запуска CLI
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "nest обязан получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "nextjs обязан получить frontend/: {mkdirs:?}"
        );
        let create_idx = |p: &str| {
            recipe
                .steps
                .iter()
                .position(|s| matches!(s, Step::CreateDirectory { path, .. } if path == p))
                .unwrap_or_else(|| panic!("{p}/ должен создаваться движком"))
        };
        let nest_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        assert!(
            create_idx("backend") < nest_idx,
            "backend/ создаётся до запуска nest"
        );
        assert!(
            create_idx("frontend")
                < recipe
                    .steps
                    .iter()
                    .position(|s| s.id() == "nextjs_create")
                    .unwrap(),
            "frontend/ создаётся до запуска nextjs"
        );

        // nest: "." + --yes + --skip-install + --skip-git, работает ВНУТРИ backend/
        let nest = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        match nest {
            Step::Command {
                args, working_dir, ..
            } => {
                assert_eq!(
                    args.get(0).map(String::as_str),
                    Some("--yes"),
                    "--yes сразу после npx (prompt «Ok to proceed?»): {args:?}"
                );
                assert_eq!(args.get(3).map(String::as_str), Some("."), "{args:?}");
                assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
                assert!(
                    args.contains(&"--skip-git".to_string()),
                    "git инициализирует движок: {args:?}"
                );
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "nest обязан работать в backend/, а не в корне (Decoupled Twin)"
                );
            }
            _ => panic!("nest_new — Command"),
        }

        // nextjs: ScaffoldGenerator выполняет create-next-app ВНУТРИ frontend/
        // (--skip-install — зависимости в финальной фазе)
        let next = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nextjs_create")
            .expect("nextjs_create должен быть в плане");
        match next {
            Step::Generate {
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "nextjs должен создаваться в frontend/: {generator_config}"
                );
                assert_eq!(on_error, &ErrorMode::Skip);
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert!(
                    args.iter().any(|a| a == "--skip-install"),
                    "create-next-app должен идти с --skip-install: {args:?}"
                );
            }
            _ => panic!("nextjs_create — Generate"),
        }

        // npm install ровно 2 раза: backend (nest) + frontend (nextjs) —
        // корневой install НЕ появляется (в корне нет package.json)
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            2,
            "install-шагов должно быть 2: {installs:?}"
        );
        match (&installs[0], &installs[1]) {
            (
                Step::Command {
                    working_dir: w0, ..
                },
                Step::Command {
                    working_dir: w1, ..
                },
            ) => {
                assert_eq!(
                    w0.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "nest-бэкенд ставится первым"
                );
                assert_eq!(
                    w1.as_deref(),
                    Some("C:\\dev\\myapp/frontend"),
                    "nextjs-фронтенд ставится вторым"
                );
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };

        // Шаблонизация ПОСЛЕ скаффолдинга...
        assert!(
            idx("nextjs_create") < idx("readme"),
            "README пишется после CLI"
        );
        assert!(
            idx("nextjs_create") < idx("gitignore"),
            ".gitignore пишется после CLI"
        );
        assert!(
            idx("nextjs_create") < idx("docker_compose"),
            "docker-compose пишется после CLI"
        );
        // ...но до git add/commit и npm install
        assert!(
            idx("git_init") < idx("readme"),
            "git init до шаблонизации — README в коммите"
        );
        assert!(
            idx("readme") < idx("git_add"),
            "README до стартового коммита"
        );
        assert!(
            idx("readme") < idx("npm_install_0"),
            "npm install — после шаблонизации"
        );

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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let gen = recipe
            .steps
            .iter()
            .find(|s| s.id() == "spring_init")
            .expect("spring_init должен быть в плане");
        match gen {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "spring-boot");
                assert_eq!(on_error, &ErrorMode::Abort);
                let deps = generator_config
                    .get("dependencies")
                    .and_then(|d| d.as_str());
                assert_eq!(
                    deps,
                    Some("web,data-jpa,postgresql,data-redis"),
                    "зависимости собираются из tools"
                );
                let name = generator_config
                    .get("project_name")
                    .and_then(|n| n.as_str());
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let step = recipe
            .steps
            .iter()
            .find(|s| s.id() == "prisma_init")
            .expect("prisma_init должен быть в плане");
        match step {
            Step::Command { command, args, .. } => {
                assert_eq!(command, "npx");
                assert!(args.contains(&"--yes".to_string()), "{args:?}");
                let provider = args
                    .iter()
                    .position(|a| a == "--datasource-provider")
                    .map(|i| args[i + 1].as_str());
                assert_eq!(provider, Some("postgresql"), "{args:?}");
                // Prisma 6.16+ разворачивает AI-навыки (.agents/.claude/...,
                // десятки тысяч файлов) — отключаем флагом
                assert!(
                    args.contains(&"--no-skills".to_string()),
                    "prisma init без --no-skills: {args:?}"
                );
            }
            _ => panic!("prisma_init — Command"),
        }

        // Подстраховка: Rust-генератор принудительно чистит агентные папки
        let cleanup = recipe
            .steps
            .iter()
            .find(|s| s.id() == "prisma_cleanup")
            .expect("prisma_cleanup должен быть в плане");
        match cleanup {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "fs-cleanup");
                assert_eq!(on_error, &ErrorMode::Skip);
                let paths = generator_config.get("paths").and_then(|p| p.as_array());
                assert!(paths.is_some_and(|p| p.iter().any(|v| v == ".agents")));
            }
            _ => panic!("prisma_cleanup — Generate"),
        }
        let cleanup_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "prisma_cleanup")
            .unwrap();
        let init_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "prisma_init")
            .unwrap();
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "react_pkg_name")
            .expect("патч имени package.json должен быть в плане");
        match patch {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "патч работает в папке скаффолда"
                );
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
    fn python_union_requirements_contains_all_deps() {
        // ЕДИНЫЙ requirements.txt python-скаффолда собирает зависимости ВСЕХ
        // python-фреймворков и инструментов (union), а не перезаписывается
        // последним пишущим: fastapi + flask + aiogram + alembic в одном файле,
        // пер-фреймворковые requirements-шаги удалены.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into(), "flask".into(), "aiogram".into()];
        ctx.tools = vec!["alembic".into(), "sqlalchemy".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(recipe.steps.iter().any(|s| s.id() == "pyproject_toml"));
        let reqs = recipe
            .steps
            .iter()
            .find(|s| s.id() == "requirements_txt")
            .unwrap();
        match reqs {
            Step::WriteFile {
                path,
                content,
                overwrite,
                ..
            } => {
                assert_eq!(path, "requirements.txt");
                assert!(
                    !*overwrite,
                    "union-файл создаётся один раз (overwrite=false)"
                );
                assert!(content.contains("fastapi[standard]"), "fastapi: {content}");
                assert!(content.contains("uvicorn"), "uvicorn: {content}");
                assert!(content.contains("flask"), "flask: {content}");
                assert!(content.contains("aiogram"), "aiogram: {content}");
                assert!(content.contains("alembic"), "alembic: {content}");
                assert!(content.contains("sqlalchemy"), "sqlalchemy: {content}");
                assert!(
                    !recipe.steps.iter().any(|s| s.id() == "fastapi_requirements"
                        || s.id() == "flask_requirements"
                        || s.id() == "aiogram_requirements"),
                    "пер-фреймворковые requirements-шаги удалены"
                );
            }
            _ => panic!("requirements_txt — WriteFile"),
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
            ("flask", vec!["flask_app"]),
        ] {
            let steps = steps_for_framework(
                fw,
                "C:\\dev\\myapp",
                "myapp",
                &context(),
                &ProjectLayout::compute(&context()),
            );
            for id in &entry_ids {
                let step = steps
                    .iter()
                    .find(|s| s.id() == id.to_string())
                    .unwrap_or_else(|| panic!("{fw}: шаг {id} должен существовать"));
                match step {
                    Step::WriteFile { overwrite, .. } => {
                        assert!(*overwrite, "{fw}: шаг {id} должен перезаписываться")
                    }
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        // react (frontend-фреймворк) даёт обе стороны → backend/ и frontend/
        let mkdirs: Vec<String> = recipe
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::CreateDirectory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(
            mkdirs.contains(&"backend".to_string()),
            "fastapi должен получить backend/: {mkdirs:?}"
        );
        assert!(
            mkdirs.contains(&"frontend".to_string()),
            "react должен получить frontend/: {mkdirs:?}"
        );

        // react: ScaffoldGenerator кладёт vite-проект в frontend/
        let vite = recipe
            .steps
            .iter()
            .find(|s| s.id() == "vite_create")
            .expect("vite_create должен быть в плане");
        match vite {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("npx")
                );
                let args = generator_config
                    .get("args")
                    .and_then(|a| a.as_array())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    args.get(0).and_then(|v| v.as_str()),
                    Some("create-vite@latest")
                );
                assert_eq!(
                    args.get(3).and_then(|v| v.as_str()),
                    Some("react-ts"),
                    "typescript → react-ts шаблон: {args:?}"
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "vite-проект живёт в frontend/: {generator_config}"
                );
            }
            _ => panic!("vite_create — Generate"),
        }

        // npm install выполняется РОВНО один раз в финальной фазе пайплайна —
        // ВНУТРИ frontend/
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(installs.len(), 1, "должен быть ровно один npm install");
        match installs[0] {
            Step::Command {
                working_dir, args, ..
            } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
                assert_eq!(args, &vec!["install".to_string()]);
            }
            _ => panic!("npm_install — Command"),
        }

        // fastapi (inplace): entry-файлы в backend/
        let main = recipe
            .steps
            .iter()
            .find(|s| s.id() == "fastapi_main")
            .expect("fastapi_main должен быть в плане");
        match main {
            Step::WriteFile { path, .. } => assert_eq!(path, "backend/src/main.py"),
            _ => panic!("fastapi_main — WriteFile"),
        }

        // airflow: dags/ директория + пример DAG — в корне (оркестрация)
        assert!(
            recipe
                .steps
                .iter()
                .any(|s| matches!(s, Step::CreateDirectory { path, .. } if path == "dags")),
            "airflow должен создать dags/"
        );
        assert!(
            recipe.steps.iter().any(|s| s.id() == "airflow_example_dag"),
            "airflow должен создать example_dag.py"
        );

        // docker-compose: airflow + postgres
        let compose = recipe
            .steps
            .iter()
            .find(|s| s.id() == "docker_compose")
            .expect("docker_compose должен быть в плане");
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("airflow"),
                    "compose должен включать airflow"
                );
                assert!(
                    content.contains("postgres"),
                    "compose должен включать postgres"
                );
            }
            _ => panic!("docker_compose — WriteFile"),
        }

        // .env.example: переменные airflow и postgres
        let env = recipe
            .steps
            .iter()
            .find(|s| s.id() == "env_example")
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };

        assert!(
            idx("py_venv_create") < idx("py_pip_install"),
            "venv создаётся до pip install"
        );
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

        // alembic ставится ЧЕРЕЗ единый манифест (requirements.txt), а не
        // отдельным pip-вызовом: манифест обязан содержать alembic, а
        // py_pip_install — единственный pip install рецепта.
        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        let pip_args = cmd_args(pip);
        assert!(
            pip_args.iter().any(|a| a == "-r"),
            "pip ставит РОВНО из манифеста (-r): {pip_args:?}"
        );
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic не ставится отдельно — он в манифесте: {pip_args:?}"
        );
        let requirements = recipe
            .steps
            .iter()
            .find(|s| s.id() == "requirements_txt")
            .expect("requirements_txt должен быть в плане");
        match requirements {
            Step::WriteFile { content, .. } => {
                assert!(
                    content.contains("alembic"),
                    "манифест обязан содержать alembic: {content}"
                );
            }
            _ => panic!("requirements_txt — WriteFile"),
        }
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

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let venv_create = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create должен быть в плане");
        let venv_args = cmd_args(venv_create);
        assert!(
            venv_args
                .iter()
                .any(|a| a.ends_with("backend/venv") || a.ends_with("backend\\venv")),
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
            Step::Command {
                command, on_error, ..
            } => {
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
        // Каждый Python-проект получает изолированное окружение (venv
        // создаётся даже без alembic — иначе fastapi-проекты ставили
        // зависимости в глобальный Python). Без Python venv-шагов нет.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["sqlalchemy".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(
            recipe.steps.iter().any(|s| s.id() == "py_venv_create"),
            "Python-проект без alembic всё равно получает venv"
        );
        // Без alembic pip НЕ ставит alembic явно (только -r requirements.txt)
        let pip = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_pip_install")
            .expect("py_pip_install должен быть в плане");
        let pip_args = cmd_args(pip);
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic не ставится, если инструмент не выбран: {pip_args:?}"
        );

        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nextjs".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
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
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

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

            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            assert!(
                !recipe
                    .steps
                    .iter()
                    .any(|s| s.id().contains("installer") || s.id().contains("@laravel")),
                "npm-путь @laravel/installer не должен использоваться"
            );

            let step = recipe
                .steps
                .iter()
                .find(|s| s.id() == step_id)
                .unwrap_or_else(|| panic!("{step_id} должен быть в плане"));
            match step {
                Step::Generate {
                    generator_id,
                    generator_config,
                    on_error,
                    ..
                } => {
                    assert_eq!(generator_id, "scaffold");
                    // Composer-скаффолд — обязательный шаг: провал
                    // инициализации фреймворка не маскируется скипом.
                    assert_eq!(on_error, &ErrorMode::Abort);
                    let command = generator_config
                        .get("command")
                        .and_then(|v| v.as_str())
                        .expect("command обязан быть");
                    assert!(
                        command == "composer" || command == "php",
                        "composer запускается как composer или php, а не npm-клиент: {generator_config}"
                    );
                    let args = generator_config
                        .get("args")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let name_arg = args
                        .iter()
                        .position(|a| a.as_str() == Some("__TARGET__"))
                        .expect("__TARGET__ обязан быть в args")
                        as usize;

                    // create-project идёт сразу после префикса (путь к phar
                    // в режиме php, ничего в режиме composer), за ним —
                    // пакет, а плейсхолдер target стоит на name_arg.
                    let cp_idx = args
                        .iter()
                        .position(|a| a.as_str() == Some("create-project"))
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
                        args.get(cp_idx + 1).and_then(|v| v.as_str()),
                        Some(package),
                        "пакет сразу после create-project: {generator_config}"
                    );
                    assert!(args.iter().any(|a| a == "--no-interaction"), "{args:?}");
                    assert!(args.iter().any(|a| a == "--prefer-source"), "{args:?}");
                    assert_eq!(
                        generator_config.get("target_dir").and_then(|v| v.as_str()),
                        Some("."),
                        "в монолите PHP-фреймворк живёт в корне"
                    );
                    // Способность: composer create-project создаёт именованную
                    // папку; temp+move по умолчанию (composer не принимает "."
                    // в непустом каталоге). Пост-условие — НАСТОЯЩИЙ composer.json
                    // (generic-фолбэк не считается успешным каркасом).
                    assert_eq!(
                        generator_config.get("capability").and_then(|v| v.as_str()),
                        Some("creates_named_directory"),
                        "{generator_config}"
                    );
                    let expected = generator_config
                        .get("expected_outputs")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    assert_eq!(
                        expected,
                        vec!["composer.json"],
                        "composer обязан создать composer.json (не package.json): {generator_config}"
                    );
                }
                _ => panic!("{step_id} — Generate"),
            }
        }
    }

    // ==================== runtime conditions (пост-условия скаффолда) ======

    #[test]
    fn runtime_condition_checks_actual_filesystem_state() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_cond_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), "{}").unwrap();

        let exists = |path: &str| Some(StepCondition::FileExists { path: path.into() });
        let not_exists = |path: &str| Some(StepCondition::FileNotExists { path: path.into() });

        assert!(runtime_condition(exists("package.json").as_ref(), &dir));
        assert!(!runtime_condition(exists("missing.txt").as_ref(), &dir));
        assert!(!runtime_condition(
            not_exists("package.json").as_ref(),
            &dir
        ));
        assert!(runtime_condition(not_exists("missing.txt").as_ref(), &dir));
        assert!(
            !runtime_condition(exists("frontend/package.json").as_ref(), &dir),
            "вложенные пути проверяются тоже"
        );

        // контекстные условия на рантайме не фильтруются
        assert!(runtime_condition(Some(&StepCondition::Always), &dir));
        assert!(runtime_condition(None, &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============ безопасные пути и политики идемпотентности ==============

    #[test]
    fn runtime_condition_backslashes_and_unsafe_paths() {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_cond2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::write(dir.join("backend").join("package.json"), "{}").unwrap();

        let exists = |path: &str| Some(StepCondition::FileExists { path: path.into() });
        let not_exists = |path: &str| Some(StepCondition::FileNotExists { path: path.into() });

        // Обратные слеши нормализуются; путь условия root-relative.
        assert!(runtime_condition(
            exists("backend\\package.json").as_ref(),
            &dir
        ));
        assert!(runtime_condition(
            exists("backend/package.json").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            not_exists("backend/package.json").as_ref(),
            &dir
        ));

        // Выход за корень / абсолютные пути: условие НЕ выполнено (шаг
        // пропускается, а не пишет мимо проекта).
        assert!(!runtime_condition(exists("../outside.txt").as_ref(), &dir));
        assert!(!runtime_condition(
            not_exists("../outside.txt").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            exists("C:\\Windows\\win.ini").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(
            not_exists("C:\\Windows\\win.ini").as_ref(),
            &dir
        ));
        assert!(!runtime_condition(exists("/etc/hosts").as_ref(), &dir));
        assert!(!runtime_condition(not_exists("/etc/hosts").as_ref(), &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plan_with_write_file(
        path: &str,
        content: &str,
        policy: Option<FilePolicy>,
        dir_name: &str,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_policy_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::WriteFile {
                id: "w".into(),
                label: "Write".into(),
                description: String::new(),
                path: path.into(),
                content: content.into(),
                overwrite: false,
                policy,
                condition: None,
                on_error: ErrorMode::Skip,
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn execute_skip_if_exists_keeps_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::SkipIfExists), "skip");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_overwrite_policy_replaces_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::Overwrite), "over");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(matches!(
            result.step_results[0].status,
            StepStatus::Success { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_create_only_skips_existing_file() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::CreateOnly),
            "create_only",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_merge_json_keeps_existing_keys_and_merges_deeply() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "conf.json",
            r#"{"b": 2, "nested": {"y": 2}}"#,
            Some(FilePolicy::MergeJson),
            "merge",
        );
        std::fs::write(dir.join("conf.json"), r#"{"a": 1, "nested": {"x": 1}}"#).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(matches!(
            result.step_results[0].status,
            StepStatus::Success { .. }
        ));
        let merged: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("conf.json")).unwrap()).unwrap();
        assert_eq!(merged["a"], 1, "существующий ключ сохраняется");
        assert_eq!(merged["b"], 2, "недостающий ключ добавляется");
        assert_eq!(
            merged["nested"]["x"], 1,
            "вложенный существующий ключ сохраняется"
        );
        assert_eq!(
            merged["nested"]["y"], 2,
            "вложенный недостающий ключ добавляется"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_merge_json_fails_on_non_json_existing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("conf.json", "{}", Some(FilePolicy::MergeJson), "merge_bad");
        std::fs::write(dir.join("conf.json"), "not json at all").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("not valid JSON")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("conf.json")).unwrap(),
            "not json at all"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_fail_on_mismatch_noop_when_identical() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "same",
            Some(FilePolicy::FailOnMismatch),
            "fom_same",
        );
        std::fs::write(dir.join("app.json"), "same").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Success { message } if message.contains("unchanged")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "same"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_fail_on_mismatch_fails_when_different() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::FailOnMismatch),
            "fom_diff",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("fail_on_mismatch")),
            "{:?}",
            result.step_results[0].status
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_write_escaping_path_fails_without_creating() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("../escape.txt", "x", Some(FilePolicy::Overwrite), "escape");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("escapes")),
            "{:?}",
            result.step_results[0].status
        );
        assert!(
            !dir.parent().unwrap().join("escape.txt").exists(),
            "файл не пишется мимо корня"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_write_absolute_path_fails() {
        let engine = DefaultRecipeEngine::new();
        let abs = std::env::temp_dir().join("stackpilot_abs_outside.txt");
        let _ = std::fs::remove_file(&abs);
        let (plan, dir) = plan_with_write_file(
            &abs.to_string_lossy(),
            "x",
            Some(FilePolicy::Overwrite),
            "abs",
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Failed { error } if error.contains("escapes")),
            "{:?}",
            result.step_results[0].status
        );
        assert!(!abs.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_rerun_second_run_skips_write_step() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) =
            plan_with_write_file("app.json", "new", Some(FilePolicy::SkipIfExists), "rerun");
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let (tx1, _rx1) = tokio::sync::mpsc::channel(16);
        let first = engine.execute(plan.clone(), tx1).await;
        assert!(matches!(
            first.step_results[0].status,
            StepStatus::Skipped { .. }
        ));
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        let second = engine.execute(plan.clone(), tx2).await;
        assert!(
            matches!(second.step_results[0].status, StepStatus::Skipped { .. }),
            "повторный запуск снова пропускает существующий файл"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app.json")).unwrap(),
            "old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_preview_marks_existing_target_as_not_executing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::SkipIfExists),
            "prev_existing",
        );
        std::fs::write(dir.join("app.json"), "old").unwrap();
        let preview = engine.preview(&plan);
        assert_eq!(preview.total_steps, 1);
        let p = &preview.step_previews[0];
        assert!(p.existing_file, "fs-статус: файл существует");
        assert_eq!(p.file_policy, Some(FilePolicy::SkipIfExists));
        assert!(
            !p.will_execute,
            "skip_if_exists над существующим файлом — шаг не выполнится"
        );
        assert!(
            p.skip_reason.is_some(),
            "причина пропуска показывается в превью"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_preview_writes_when_target_missing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_write_file(
            "app.json",
            "new",
            Some(FilePolicy::SkipIfExists),
            "prev_missing",
        );
        let preview = engine.preview(&plan);
        assert!(!preview.step_previews[0].existing_file);
        assert!(preview.step_previews[0].will_execute);
        assert_eq!(preview.will_execute_count, 1);
        assert_eq!(preview.will_skip_count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scaffold_generate_step(extra_config: serde_json::Value) -> Step {
        // CLI создаёт временную папку (плейсхолдер → temp_name) и падает
        // с exit 1 — сценарий «CLI умер после создания temp-каталога».
        let args = if cfg!(target_os = "windows") {
            vec![
                "/c".to_string(),
                "mkdir".to_string(),
                SCAFFOLD_TARGET.to_string(),
                "&&".to_string(),
                "exit".to_string(),
                "1".to_string(),
            ]
        } else {
            vec![
                "-c".to_string(),
                "mkdir".to_string(),
                "-p".to_string(),
                SCAFFOLD_TARGET.to_string(),
                "&&".to_string(),
                "exit".to_string(),
                "1".to_string(),
            ]
        };
        let mut config = serde_json::json!({
            "command": if cfg!(target_os = "windows") { "cmd" } else { "sh" },
            "args": args,
            "capability": "creates_named_directory",
            "target_dir": ".",
            "temp_dir_allowed": true,
            "expected_outputs": vec!["package.json"],
        });
        if let serde_json::Value::Object(map) = &mut config {
            if let serde_json::Value::Object(extra) = extra_config {
                for (k, v) in extra {
                    map.insert(k, v);
                }
            }
        }
        Step::Generate {
            id: "scaffold_test".into(),
            label: "Scaffold".into(),
            description: String::new(),
            generator_id: "scaffold".into(),
            generator_config: config,
            policy: None,
            condition: None,
            on_error: ErrorMode::Skip,
        }
    }

    fn plan_with_generate(step: Step, dir_name: &str) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_scaffold_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![step],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn execute_scaffold_skip_if_exists_when_outputs_present() {
        let engine = DefaultRecipeEngine::new();
        let mut step = scaffold_generate_step(serde_json::json!({}));
        match &mut step {
            Step::Generate { policy, .. } => *policy = Some(FilePolicy::SkipIfExists),
            _ => unreachable!(),
        }
        let (plan, dir) = plan_with_generate(step, "skip_done");
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        // CLI (exit 1) НЕ запускается: все expected_outputs на месте.
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Success { message } if message.contains("skipped by policy")),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_scaffold_runs_cli_when_outputs_missing() {
        let engine = DefaultRecipeEngine::new();
        let step = scaffold_generate_step(serde_json::json!({}));
        let (plan, dir) = plan_with_generate(step, "run_cli");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Failed { .. }),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_failed_scaffold_leaves_no_temp_dir() {
        let engine = DefaultRecipeEngine::new();
        let step = scaffold_generate_step(serde_json::json!({}));
        let (plan, dir) = plan_with_generate(step, "temp_cleanup");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Failed { .. }),
            "{:?}",
            result.step_results[0].status
        );
        // CLI создал временную папку и упал — temp+move обязан её удалить.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("temp_"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "после провала не должно оставаться temp_-папок: {leftovers:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_marks_scaffold_with_existing_outputs_as_not_executing() {
        let engine = DefaultRecipeEngine::new();
        let mut step = scaffold_generate_step(serde_json::json!({}));
        match &mut step {
            Step::Generate { policy, .. } => *policy = Some(FilePolicy::SkipIfExists),
            _ => unreachable!(),
        }
        let (plan, dir) = plan_with_generate(step, "prev_scaffold");
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let preview = engine.preview(&plan);
        let p = &preview.step_previews[0];
        assert!(
            p.existing_file,
            "post-условия на месте — existing_file true"
        );
        assert!(
            !p.will_execute,
            "scaffold с готовыми выходами не выполняется"
        );
        assert_eq!(preview.will_execute_count, 0);
        assert_eq!(preview.will_skip_count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ============ идемпотентность git-шагов и политики скаффолдов =========

    #[test]
    fn git_init_gated_on_head_file() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_git_init(&ctx, "C:\\dev\\myapp");
        let init = steps
            .iter()
            .find(|s| s.id() == "git_init")
            .expect("git_init в плане");
        assert_eq!(
            init.condition(),
            Some(&StepCondition::FileNotExists {
                path: ".git/HEAD".into()
            }),
            "повторный запуск не переинициализирует репозиторий"
        );
    }

    #[test]
    fn git_commit_is_idempotent_noop_when_nothing_staged() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_finalize(&ctx, &[], "C:\\dev\\myapp", "myapp");
        let commit = steps
            .iter()
            .find(|s| s.id() == "git_commit")
            .expect("git_commit в плане");
        match commit {
            Step::Command { command, .. } => {
                assert!(
                    command.contains("git diff --cached --quiet"),
                    "коммит только при изменениях: {command}"
                );
                assert!(
                    command.contains("exit 0"),
                    "нечего коммитить — не ошибка: {command}"
                );
            }
            other => panic!("ожидался Command: {:?}", other.id()),
        }
    }

    #[test]
    fn safe_scaffolds_carry_skip_if_exists_policy() {
        let ctx = context();
        let layout = ProjectLayout::compute(&ctx);
        let zig = steps_for_language("zig", "myapp", "C:\\dev\\myapp", &ctx);
        let zig_step = zig
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init в плане");
        assert_eq!(zig_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let flutter = steps_for_framework("flutter", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let flutter_step = flutter
            .iter()
            .find(|s| s.id() == "flutter_create")
            .expect("flutter_create в плане");
        assert_eq!(flutter_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let laravel = steps_for_framework("laravel", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let laravel_step = laravel
            .iter()
            .find(|s| s.id() == "laravel_new")
            .expect("laravel_new в плане");
        assert_eq!(laravel_step.file_policy(), Some(FilePolicy::SkipIfExists));

        let symfony = steps_for_framework("symfony", "C:\\dev\\myapp", "myapp", &ctx, &layout);
        let symfony_step = symfony
            .iter()
            .find(|s| s.id() == "symfony_new")
            .expect("symfony_new в плане");
        assert_eq!(symfony_step.file_policy(), Some(FilePolicy::SkipIfExists));
    }

    #[test]
    fn npm_install_gated_on_node_modules() {
        let mut ctx = context();
        ctx.git_init = true;
        let steps = steps_for_finalize(&ctx, &["frontend".to_string()], "C:\\dev\\myapp", "myapp");
        let install = steps
            .iter()
            .find(|s| s.id() == "npm_install_0")
            .expect("npm_install_0 в плане");
        assert_eq!(
            install.condition(),
            Some(&StepCondition::FileNotExists {
                path: "frontend/node_modules".into()
            }),
            "повторный запуск не переустанавливает зависимости"
        );
    }

    #[test]
    fn language_inits_carry_rerun_guards() {
        let ctx = context();
        let cases: &[(&str, &str)] = &[
            ("rust", "Cargo.toml"),
            ("go", "go.mod"),
            ("java", "pom.xml"),
            ("elixir", "mix.exs"),
            ("gleam", "gleam.toml"),
            ("typescript", "tsconfig.json"),
        ];
        for (lang, marker) in cases {
            let steps = steps_for_language(lang, "myapp", "C:\\dev\\myapp", &ctx);
            assert!(
                steps.iter().any(|s| s.condition()
                    == Some(&StepCondition::FileNotExists {
                        path: marker.to_string()
                    })),
                "{lang}: шаг обязан иметь FileNotExists {marker}"
            );
        }
    }

    #[test]
    fn dart_create_gated_on_subdir_pubspec() {
        let ctx = context();
        let steps = steps_for_language("dart", "my-app", "C:\\dev\\myapp", &ctx);
        assert!(
            steps.iter().any(|s| s.condition()
                == Some(&StepCondition::FileNotExists {
                    path: "my_app/pubspec.yaml".into()
                })),
            "dart create кладёт пакет в подпапку — гейт на {}/pubspec.yaml",
            "my_app"
        );
    }

    #[test]
    fn django_venv_gated_on_marker() {
        // Django (как и любой Python-фреймворк) использует ЕДИНСТВЕННЫЙ
        // канонический venv проекта: py_venv_create гейтится маркером
        // venv/pyvenv.cfg, повторный запуск не пересоздаёт окружение,
        // а отдельного django-venv не существует.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let create = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create в плане");
        assert_eq!(
            create.condition(),
            Some(&StepCondition::FileNotExists {
                path: "venv/pyvenv.cfg".into()
            }),
            "повторный запуск не пересоздаёт venv"
        );
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "django_venv_create"),
            "отдельного django-venv быть не должно — venv единственный"
        );
    }

    fn plan_with_single_command(
        condition: Option<StepCondition>,
        dir_name: &str,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_engine_{dir_name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (command, args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec!["/d".into(), "/c".into(), "exit 0".into()],
            )
        } else {
            ("true".to_string(), vec![])
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::Command {
                id: "dependent".into(),
                label: "Dependent step".into(),
                description: "Depends on scaffold output".into(),
                command,
                args,
                working_dir: None,
                env: None,
                timeout_secs: Some(10),
                condition,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn engine_skips_dependent_step_when_postcondition_missing() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_single_command(
            Some(StepCondition::FileExists {
                path: "package.json".into(),
            }),
            "skip",
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert_eq!(result.step_results.len(), 1);
        assert!(
            matches!(
                &result.step_results[0].status,
                StepStatus::Skipped { reason } if reason.contains("package.json")
            ),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn engine_runs_step_when_postcondition_satisfied() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_single_command(
            Some(StepCondition::FileExists {
                path: "package.json".into(),
            }),
            "run",
        );
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        drop(rx);
        assert_eq!(result.step_results.len(), 1);
        assert!(
            !matches!(result.step_results[0].status, StepStatus::Skipped { .. }),
            "{:?}",
            result.step_results[0].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ==================== Explicit step dependencies ====================

    /// План из двух шагов: [prereq] + [dependent]; prereq выполняет команду
    /// из `exit_code` (0 = успех, 1 = провал). Оба шага без условий
    /// (runtime-условия не вмешиваются в зависимостные сценарии).
    fn plan_with_dependency(
        dir_name: &str,
        prereq_exit: &str,
        prereq_condition: Option<StepCondition>,
        dependent_condition: Option<StepCondition>,
        dep: StepDependency,
    ) -> (ExecutionPlan, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("stackpilot_deps_{dir_name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (cmd, ok_args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec!["/d".into(), "/c".into(), "exit 0".into()],
            )
        } else {
            ("true".to_string(), vec![])
        };
        let prereq = Step::Command {
            id: "prereq".into(),
            label: "Prerequisite step".into(),
            description: String::new(),
            command: cmd.clone(),
            args: if prereq_exit == "0" {
                ok_args.clone()
            } else if cfg!(target_os = "windows") {
                vec!["/d".into(), "/c".into(), "exit 1".into()]
            } else {
                vec!["false".into()]
            },
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: prereq_condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dependent = Step::Command {
            id: "dependent".into(),
            label: "Dependent step".into(),
            description: String::new(),
            command: cmd,
            args: ok_args,
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: dependent_condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![prereq, dependent],
            dependencies: vec![dep],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        (plan, dir)
    }

    #[tokio::test]
    async fn dependency_skips_dependent_when_prereq_failed() {
        // Провал предшественника (например, nest_new / py_venv_create /
        // go_mod_init / vite_create) → зависимый шаг пропускается С ТОЧНОЙ
        // причиной, а не выполняет команду и не падает с вторичной ошибкой.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "fail",
            "1",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert_eq!(result.step_results.len(), 2);
        assert!(matches!(
            &result.step_results[0].status,
            StepStatus::Failed { .. }
        ));
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("prerequisite 'prereq'")
                    && reason.contains("failed")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_skips_dependent_when_prereq_skipped() {
        // Предшественник пропущен по runtime-условию и файлового
        // пост-условия нет → зависимый шаг пропускается.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "skip_no_file",
            "0",
            Some(StepCondition::FileExists {
                path: "never_created.txt".into(),
            }),
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(matches!(
            &result.step_results[0].status,
            StepStatus::Skipped { .. }
        ));
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("prerequisite 'prereq' was skipped")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_proceeds_when_skipped_prereq_left_postcondition() {
        // Django-ранний venv: py_venv_create пропущен (маркер
        // venv/pyvenv.cfg уже создан django_venv_create), но py_pip_upgrade
        // ОБЯЗАН выполниться — пост-условие предшественника на месте.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "skip_has_file",
            "0",
            Some(StepCondition::FileNotExists {
                path: "venv/pyvenv.cfg".into(),
            }),
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: "venv/pyvenv.cfg".into(),
            },
        );
        std::fs::create_dir_all(dir.join("venv")).unwrap();
        std::fs::write(dir.join("venv/pyvenv.cfg"), "").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(&result.step_results[0].status, StepStatus::Skipped { .. }),
            "prereq: {:?}",
            result.step_results[0].status
        );
        assert!(
            matches!(&result.step_results[1].status, StepStatus::Success { .. }),
            "dependent обязан выполниться: {:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_marks_prereq_failed_when_postcondition_missing() {
        // Предшественник «успешно» завершился, но обещанного файла нет —
        // он ретроактивно помечается Failed (путь + рабочая директория),
        // зависимый шаг пропускается.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "missing_post",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: "dist/index.html".into(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(
                &result.step_results[0].status,
                StepStatus::Failed { error } if error.contains("dist/index.html")
                    && error.contains(&dir.to_string_lossy().into_owned())
            ),
            "prereq обязан стать Failed с путём и cwd: {:?}",
            result.step_results[0].status
        );
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("expected output 'dist/index.html'")
                    && reason.contains("'prereq'")
            ),
            "{:?}",
            result.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn dependency_skips_step_when_required_file_does_not_exist() {
        // Чистое файловое предусловие (без предшественника): файл обязан
        // быть на момент запуска — отсутствует → пропуск с причиной.
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "pure_file",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: String::new(),
                expects_file: "frontend/package.json".into(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(
                &result.step_results[1].status,
                StepStatus::Skipped { reason } if reason.contains("required file 'frontend/package.json' does not exist")
            ),
            "{:?}",
            result.step_results[1].status
        );
        // файл появляется → шаг выполняется
        let (plan2, dir2) = plan_with_dependency(
            "pure_file_ok",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: String::new(),
                expects_file: "frontend/package.json".into(),
            },
        );
        std::fs::create_dir_all(dir2.join("frontend")).unwrap();
        std::fs::write(dir2.join("frontend/package.json"), "{}").unwrap();
        let (tx2, _rx2) = tokio::sync::mpsc::channel(16);
        let result2 = engine.execute(plan2, tx2).await;
        assert!(
            matches!(&result2.step_results[1].status, StepStatus::Success { .. }),
            "{:?}",
            result2.step_results[1].status
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn topo_order_sorts_reverse_declared_dependency_chain() {
        // Рецепт декларирует шаги в ОБРАТНОМ порядке: сборка (C) → установка
        // (B) → каркас (A). Топосортировка выстраивает цепочку правильно.
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("C"), cmd("B"), cmd("A")];
        let ordered = topo_order_steps(&steps, &[dep("C", "B"), dep("B", "A")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B", "C"]);
    }

    #[test]
    fn topo_order_keeps_declaration_order_for_independent_steps() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        // Без зависимостей — порядок декларации сохраняется полностью.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
        // Y зависит от X — независимый Z остаётся на своём месте.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[dep("Y", "X")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
        // Z зависит от X — Y (независимый) не переставляется.
        let steps = vec![cmd("X"), cmd("Y"), cmd("Z")];
        let ordered = topo_order_steps(&steps, &[dep("Z", "X")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["X", "Y", "Z"]);
    }

    #[test]
    fn topo_order_rejects_missing_prerequisite() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("B", "missing")]).unwrap_err();
        assert!(err.contains("'missing' of 'B' is not in the plan"), "{err}");
        // Чисто файловое предусловие: dependent обязан существовать,
        // предшественник не нужен — порядок не меняется.
        let ordered = topo_order_steps(&steps, &[dep("A", "")]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B"]);
        let err = topo_order_steps(&steps, &[dep("ghost", "")]).unwrap_err();
        assert!(err.contains("'ghost' is not in the plan"), "{err}");
    }

    #[test]
    fn topo_order_rejects_self_dependency() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("A", "A")]).unwrap_err();
        assert!(err.contains("'A' cannot depend on itself"), "{err}");
    }

    #[test]
    fn topo_order_rejects_duplicate_step_ids() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![cmd("A"), cmd("A")];
        let err = topo_order_steps(&steps, &[]).unwrap_err();
        assert!(err.contains("duplicate step id 'A'"), "{err}");
    }

    #[test]
    fn topo_order_reports_two_node_cycle_path() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B")];
        let err = topo_order_steps(&steps, &[dep("A", "B"), dep("B", "A")]).unwrap_err();
        assert!(err.contains("Dependency cycle detected"), "{err}");
        assert!(err.contains("A -> B -> A"), "{err}");
    }

    #[test]
    fn topo_order_reports_three_node_cycle_path() {
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let dep = |step: &str, prereq: &str| StepDependency {
            step_id: step.into(),
            prereq_id: prereq.into(),
            expects_file: String::new(),
        };
        let steps = vec![cmd("A"), cmd("B"), cmd("C")];
        let err =
            topo_order_steps(&steps, &[dep("A", "B"), dep("B", "C"), dep("C", "A")]).unwrap_err();
        assert!(err.contains("Dependency cycle detected"), "{err}");
        assert!(err.contains("A -> B -> C -> A"), "{err}");
    }

    #[test]
    fn topo_order_reorders_parallel_flattening_dependencies() {
        // Parallel раскрывается в порядке следования шагов [B, A]; зависимость
        // B→A нормализуется перестановкой в [A, B] — план НЕ отклоняется.
        let cmd = |id: &str| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![Step::Parallel {
            id: "par".into(),
            label: "Parallel".into(),
            description: String::new(),
            steps: vec![cmd("B"), cmd("A")],
            condition: None,
            on_error: ErrorMode::Skip,
        }];
        let flat = flatten_steps(&steps, &WizardContext::default());
        assert_eq!(
            flat.iter().map(|s| s.id()).collect::<Vec<_>>(),
            vec!["B", "A"]
        );
        let dep = StepDependency {
            step_id: "B".into(),
            prereq_id: "A".into(),
            expects_file: String::new(),
        };
        let ordered = topo_order_steps(&flat, &[dep]).unwrap();
        let ids: Vec<String> = ordered.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec!["A", "B"]);
    }

    #[tokio::test]
    async fn execute_aborts_on_cyclic_dependencies() {
        // План с настоящим циклом не выполняется вовсе: ранний Aborted
        // с полным путём цикла, без единого шага.
        let engine = DefaultRecipeEngine::new();
        let (mut plan, dir) = plan_with_dependency(
            "cycle",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        plan.dependencies.push(StepDependency {
            step_id: "prereq".into(),
            prereq_id: "dependent".into(),
            expects_file: String::new(),
        });
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(
                &result.overall,
                OverallStatus::Aborted { reason, .. }
                    if reason.to_lowercase().contains("dependency cycle detected")
            ),
            "{:?}",
            result.overall
        );
        assert!(result.step_results.is_empty(), "ни один шаг не выполняется");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_normalizes_out_of_order_declared_steps() {
        // Прямо построенный план с зависимым шагом ДО предшественника:
        // execute() нормализует порядок (стабильная топосортировка) и
        // выполняет оба шага — предшественник первым.
        let engine = DefaultRecipeEngine::new();
        let (mut plan, dir) = plan_with_dependency(
            "reorder",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        plan.steps = vec![plan.steps[1].clone(), plan.steps[0].clone()];
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.overall, OverallStatus::Success),
            "{:?}",
            result.overall
        );
        let ids: Vec<String> = result
            .step_results
            .iter()
            .map(|r| r.step_id.clone())
            .collect();
        assert_eq!(ids, vec!["prereq", "dependent"]);
        assert!(
            result
                .step_results
                .iter()
                .all(|r| matches!(r.status, StepStatus::Success { .. })),
            "{:?}",
            result.step_results
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_dependencies_filters_absent_steps_and_derives_expects_file() {
        let cmd = |id: &str, condition: Option<StepCondition>| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![
            cmd("scaffold", None),
            // ровно ОДНА выжившая декларация → expects_file выводится из
            // FileExists-условия зависимого шага
            cmd(
                "patch",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
            cmd("other", None),
            // явное пост-условие переопределяет вывод из условия
            cmd(
                "patch2",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
        ];
        let declared = vec![
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
            // висячий предшественник — отбрасывается
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "ghost".into(),
                expects_file: String::new(),
            },
            // висячий dependent — отбрасывается
            StepDependency {
                step_id: "ghost".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
            StepDependency {
                step_id: "patch2".into(),
                prereq_id: "other".into(),
                expects_file: "explicit.txt".into(),
            },
            // дубликат схлопывается
            StepDependency {
                step_id: "patch".into(),
                prereq_id: "scaffold".into(),
                expects_file: String::new(),
            },
        ];
        let built = build_dependencies(&steps, &declared);
        assert_eq!(built.len(), 2, "{built:?}");
        let derived = built
            .iter()
            .find(|d| d.prereq_id == "scaffold")
            .expect("выжившая декларация");
        assert_eq!(derived.expects_file, "frontend/package.json");
        let explicit = built
            .iter()
            .find(|d| d.prereq_id == "other")
            .expect("выжившая декларация");
        assert_eq!(explicit.expects_file, "explicit.txt");
    }

    #[test]
    fn build_dependencies_skips_derivation_for_multi_prereq_dependents() {
        // qt_cmake_build зависит и от qt_web_build, и от qt_cmake_configure:
        // его FileExists-условие (frontend/dist/index.html) — пост-условие
        // ТОЛЬКО первого, авто-вывод отключён, expects_file задан явно.
        let cmd = |id: &str, condition: Option<StepCondition>| Step::Command {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            command: "true".into(),
            args: vec![],
            working_dir: None,
            env: None,
            timeout_secs: Some(10),
            condition,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        };
        let steps = vec![
            cmd(
                "qt_web_build",
                Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
            ),
            cmd(
                "qt_cmake_configure",
                Some(StepCondition::FileExists {
                    path: "CMakeLists.txt".into(),
                }),
            ),
            cmd(
                "qt_cmake_build",
                Some(StepCondition::FileExists {
                    path: "frontend/dist/index.html".into(),
                }),
            ),
        ];
        let declared = vec![
            StepDependency {
                step_id: "qt_cmake_build".into(),
                prereq_id: "qt_web_build".into(),
                expects_file: "frontend/dist/index.html".into(),
            },
            StepDependency {
                step_id: "qt_cmake_build".into(),
                prereq_id: "qt_cmake_configure".into(),
                expects_file: String::new(),
            },
        ];
        let built = build_dependencies(&steps, &declared);
        assert_eq!(built.len(), 2);
        let web = built
            .iter()
            .find(|d| d.prereq_id == "qt_web_build")
            .unwrap();
        assert_eq!(web.expects_file, "frontend/dist/index.html");
        let cmake = built
            .iter()
            .find(|d| d.prereq_id == "qt_cmake_configure")
            .unwrap();
        assert_eq!(
            cmake.expects_file, "",
            "без авто-вывода для множественных dep"
        );
    }

    #[test]
    fn recipe_declares_dependency_pairs() {
        // Nest + telegraf: патчи package.json — после nest_new.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "telegraf".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            find("nest_pkg_name", "nest_new"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("telegraf_pkg_patch", "nest_new"),
            "{:?}",
            recipe.dependencies
        );

        // Gin + Cobra: go-команды — после go mod init.
        let mut ctx = context();
        ctx.languages = vec!["go".into()];
        ctx.frameworks = vec!["gin".into(), "cobra".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(recipe
            .dependencies
            .iter()
            .any(|d| d.step_id == "get_gin" && d.prereq_id == "go_mod_init"));
        assert!(recipe
            .dependencies
            .iter()
            .any(|d| d.step_id == "cobra_init" && d.prereq_id == "go_mod_init"));
    }

    #[test]
    fn recipe_declares_python_dependencies() {
        // python + alembic: pip/alembic — строго после venv и pip-установки.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(find("py_pip_upgrade", "py_venv_create"));
        assert!(find("py_pip_check", "py_venv_create"));
        assert!(find("py_pip_install", "py_pip_upgrade"));
        assert!(find("py_pip_install", "py_pip_check"));
        assert!(find("alembic_init", "py_pip_install"));
        assert!(find("py_requirements_check", "py_pip_install"));
        // маркер venv — пост-условие venv-шагов (единый канонический venv)
        let dep = recipe
            .dependencies
            .iter()
            .find(|d| d.step_id == "py_pip_upgrade" && d.prereq_id == "py_venv_create")
            .unwrap();
        assert_eq!(
            dep.expects_file, "venv/pyvenv.cfg",
            "{:?}",
            recipe.dependencies
        );
    }

    #[test]
    fn recipe_declares_django_start_dependency() {
        // django-admin (django_start) — строго ПОСЛЕ установки манифеста
        // единого venv: отдельного django-venv не существует, пакеты ставятся
        // ровно один раз через py_pip_install.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == "django_start" && d.prereq_id == "py_pip_install"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "django_venv_create")
                && !recipe.steps.iter().any(|s| s.id() == "django_pip_install"),
            "django использует единый venv, отдельного django-venv нет"
        );
    }

    #[test]
    fn recipe_declares_qt_webengine_dependencies() {
        // qt webengine: cmake-сборка — после веб-сборки (с явным
        // пост-условием frontend/dist/index.html) и после cmake-конфигурации.
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let recipe = recipe_for(&ctx, "myapp").unwrap();
        let find = |step: &str, prereq: &str| {
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            find("qt_web_build", "vite_create"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_configure", "qt_cmake"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_build", "qt_web_build"),
            "{:?}",
            recipe.dependencies
        );
        assert!(
            find("qt_cmake_build", "qt_cmake_configure"),
            "{:?}",
            recipe.dependencies
        );
        let dep = recipe
            .dependencies
            .iter()
            .find(|d| d.step_id == "qt_cmake_build" && d.prereq_id == "qt_web_build")
            .unwrap();
        assert_eq!(dep.expects_file, "frontend/dist/index.html");
    }

    #[test]
    fn plan_keeps_only_surviving_dependencies() {
        // tauri + react (компаньон): фронтенд скаффолдит vite_create, а
        // НЕ tauri_web_scaffold — декларация на отсутствующий шаг исчезает,
        // выжившая получает expects_file из условия зависимого шага.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "rust".into()];
        ctx.frameworks = vec!["tauri".into(), "react".into()];
        let engine = DefaultRecipeEngine::new();
        let plan = engine
            .plan(&ctx, std::path::Path::new("C:\\dev\\myapp"))
            .expect("plan должен собраться");
        let ids: Vec<String> = plan.steps.iter().map(|s| s.id()).collect();
        assert!(ids.iter().any(|i| i == "vite_create"), "{ids:?}");
        assert!(!ids.iter().any(|i| i == "tauri_web_scaffold"), "{ids:?}");
        let has_pair = |plan: &ExecutionPlan, step: &str, prereq: &str| {
            plan.dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq)
        };
        assert!(
            has_pair(&plan, "tauri_pkg_name", "vite_create"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            !has_pair(&plan, "tauri_pkg_name", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            has_pair(&plan, "tauri_config_patch", "tauri_init"),
            "{:?}",
            plan.dependencies
        );
        let dep = plan
            .dependencies
            .iter()
            .find(|d| d.step_id == "tauri_pkg_name" && d.prereq_id == "vite_create")
            .unwrap();
        assert_eq!(dep.expects_file, "frontend/package.json");
        // без компаньона выживает tauri_web_scaffold
        let mut ctx = context();
        ctx.languages = vec!["typescript".into(), "rust".into()];
        ctx.frameworks = vec!["tauri".into()];
        let plan = engine
            .plan(&ctx, std::path::Path::new("C:\\dev\\myapp"))
            .unwrap();
        assert!(
            has_pair(&plan, "tauri_pkg_name", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            !has_pair(&plan, "tauri_pkg_name", "vite_create"),
            "{:?}",
            plan.dependencies
        );
        assert!(
            has_pair(&plan, "tauri_web_install", "tauri_web_scaffold"),
            "{:?}",
            plan.dependencies
        );
    }

    #[test]
    fn preview_shows_dependencies_and_skip_reasons() {
        let engine = DefaultRecipeEngine::new();
        let (plan, dir) = plan_with_dependency(
            "preview",
            "0",
            None,
            None,
            StepDependency {
                step_id: "dependent".into(),
                prereq_id: "prereq".into(),
                expects_file: String::new(),
            },
        );
        let preview = engine.preview(&plan);
        let dependent = preview
            .step_previews
            .iter()
            .find(|p| p.id == "dependent")
            .expect("dependent в превью");
        assert_eq!(dependent.prerequisites, vec!["prereq"]);
        assert!(
            dependent
                .possible_skip_reasons
                .iter()
                .any(|r| r.contains("'prereq'")),
            "{:?}",
            dependent.possible_skip_reasons
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recipe_without_dependencies_field_parses() {
        // Обратная совместимость: старые сериализованные рецепты (без
        // поля dependencies) десериализуются в пустой список.
        let json = r#"{
            "id": "recipe_old",
            "name": "Old recipe",
            "description": "legacy",
            "tags": ["typescript"],
            "steps": []
        }"#;
        let recipe: Recipe = serde_json::from_str(json).expect("старый Recipe парсится");
        assert!(recipe.dependencies.is_empty());
        // ExecutionPlan/StepPreview — те же гарантии
        let preview_json = r#"{"id":"s1","label":"L","description":"D","action":"$ x","will_execute":true,"skip_reason":null}"#;
        let sp: StepPreview =
            serde_json::from_str(preview_json).expect("старый StepPreview парсится");
        assert!(sp.prerequisites.is_empty());
        assert!(sp.possible_skip_reasons.is_empty());
    }

    // ==================== Scenario A: Nest + Telegraf ====================

    #[test]
    fn nest_uses_yes_flag_to_skip_npx_prompt() {
        // Без --yes npx спрашивает «Ok to proceed?» и падает в не-TTY
        // сессии; --package-manager фиксирует ответ промпта флагом.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let step = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nest_new")
            .expect("nest_new должен быть в плане");
        let args = cmd_args(step);
        assert_eq!(
            args.get(0).map(String::as_str),
            Some("--yes"),
            "--yes сразу после npx (иначе prompt 'Ok to proceed?'): {args:?}"
        );
        assert!(args.iter().any(|a| a == "--package-manager"), "{args:?}");
        assert!(args.iter().any(|a| a == "--skip-install"), "{args:?}");
        assert!(args.iter().any(|a| a == "--skip-git"), "{args:?}");
    }

    #[test]
    fn telegraf_patches_nest_package_json_without_clobbering() {
        // nest + telegraf: telegraf — side-фреймворк (kind="side"), его шаги
        // выполняются ПОСЛЕ nest и НЕ перезаписывают package.json nest.
        // В split (react + nest + telegraf) оба живут в backend/: dep-патч
        // работает в backend/ с условием FileExists backend/package.json.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.backend_languages = vec!["typescript".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nest".into(), "react".into(), "telegraf".into()];

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert!(
            !recipe.steps.iter().any(|s| s.id() == "telegraf_package"),
            "при nest telegraf не пишет собственный package.json (затирал бы nest)"
        );
        let patch = recipe
            .steps
            .iter()
            .find(|s| s.id() == "telegraf_pkg_patch")
            .expect("dep-патч обязан быть при nest");
        match patch {
            Step::Command {
                working_dir,
                condition,
                args,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "патч работает в каталоге nest-каркаса"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "backend/package.json", "условие seg-префиксовано")
                    }
                    other => panic!("ожидали FileExists backend/package.json: {other:?}"),
                }
                let script = args
                    .iter()
                    .find(|a| a.starts_with("const fs="))
                    .expect("node -e скрипт");
                assert!(script.contains("telegraf"), "{script}");
                assert!(script.contains("4.16.3"), "{script}");
            }
            _ => panic!("telegraf_pkg_patch — Command"),
        }
        // Без nest telegraf пишет собственный package.json
        let mut solo = context();
        solo.languages = vec!["typescript".into()];
        solo.frameworks = vec!["telegraf".into()];
        let solo_recipe = recipe_for(&solo, "myapp").expect("recipe must build");
        assert!(solo_recipe
            .steps
            .iter()
            .any(|s| s.id() == "telegraf_package"));
        assert!(!solo_recipe
            .steps
            .iter()
            .any(|s| s.id() == "telegraf_pkg_patch"));
    }

    #[test]
    fn express_fastify_telegraf_declare_real_deps_and_validate_manifest() {
        // Express/Fastify/Telegraf: зависимости — НАСТОЯЩИЕ записи в
        // package.json (финальный npm install ставит их), а не упоминание в
        // entry-файле. Каждый каркас получает пост-валидацию manifest-check
        // (Abort), требующую свой пакет.
        for (fw, pkg, check_id, package_id) in [
            ("express", "express", "express_pkg_check", "express_package"),
            ("fastify", "fastify", "fastify_pkg_check", "fastify_package"),
            (
                "telegraf",
                "telegraf",
                "telegraf_pkg_check",
                "telegraf_package",
            ),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["javascript".into()];
            ctx.frameworks = vec![fw.into()];
            let layout = ProjectLayout::compute(&ctx);
            let pkg_path = match layout.framework_dir(fw) {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            match find_step(&recipe, check_id) {
                Step::Generate {
                    generator_id,
                    generator_config,
                    on_error,
                    ..
                } => {
                    assert_eq!(generator_id, "manifest-check", "{check_id}");
                    assert_eq!(
                        generator_config.get("path").and_then(|v| v.as_str()),
                        Some(pkg_path.as_str()),
                        "{check_id}"
                    );
                    assert!(
                        gen_strs(generator_config, "required_dependencies")
                            .contains(&pkg.to_string()),
                        "{check_id}"
                    );
                    assert_eq!(
                        on_error,
                        &ErrorMode::Abort,
                        "отсутствие зависимости фреймворка останавливает пайплайн: {check_id}"
                    );
                }
                _ => panic!("{check_id} — Generate"),
            }
            match find_step(&recipe, package_id) {
                Step::WriteFile { content, .. } => {
                    assert!(
                        content.contains(&format!("\"{pkg}\"")),
                        "{package_id} обязан декларировать {pkg}: {content}"
                    );
                }
                _ => panic!("{package_id} — WriteFile"),
            }
        }
    }

    #[test]
    fn side_frameworks_run_after_main_frameworks() {
        // kind="side" (telegraf, aiogram) выполняется ПОСЛЕ главных
        // фреймворков (nest, django) независимо от порядка карточек в
        // мастере: их шаги пишут поверх/патчат каркас главного.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["aiogram".into(), "django".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(
            idx("django_start") < idx("aiogram_bot"),
            "django (app) обязан скаффолдиться до aiogram (side)"
        );
    }

    // ==================== Scenario B: Laravel / Symfony / PHP ====================

    #[test]
    fn laravel_symfony_php_preflight_precedes_composer_without_platform_req() {
        // composer create-project: PHP-префлайт (ext-fileinfo) идёт ДО
        // composer-скаффолда, а --ignore-platform-req=ext-fileinfo удалён —
        // он маскировал отсутствие расширения и Laravel/Symfony падали
        // в рантайме с невнятными ошибками.
        for (fw_id, check_id, new_id) in [
            ("laravel", "laravel_php_check", "laravel_new"),
            ("symfony", "symfony_php_check", "symfony_new"),
        ] {
            let mut ctx = context();
            ctx.languages = vec!["php".into()];
            ctx.frameworks = vec![fw_id.into()];
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            let idx = |id: &str| {
                recipe
                    .steps
                    .iter()
                    .position(|s| s.id() == id)
                    .unwrap_or_else(|| panic!("{fw_id}: {id} должен быть в плане"))
            };
            assert!(
                idx(check_id) < idx(new_id),
                "{fw_id}: php-префлайт обязан идти ДО composer create-project"
            );
            let check = &recipe.steps[idx(check_id)];
            assert_eq!(cmd_args(check)[0], "-r", "префлайт — php -r скрипт");
            let scaffold = recipe.steps.iter().find(|s| s.id() == new_id).unwrap();
            if let Step::Generate {
                generator_config, ..
            } = scaffold
            {
                let args = gen_args(generator_config);
                assert!(
                    !args.iter().any(|a| a.contains("ignore-platform-req")),
                    "{fw_id}: --ignore-platform-req удалён (маскировал отсутствие ext-fileinfo): {args:?}"
                );
            } else {
                panic!("{new_id} — Generate");
            }
        }
    }

    // ==================== Scenario C: Django / FastAPI / Python ====================

    #[test]
    fn python_preflight_runs_before_venv_and_django_steps() {
        // python_preflight (версия + путь интерпретатора) выполняется
        // РАНЬШЕ любых venv/pip/django-admin шагов: ошибка интерпретатора
        // видна сразу, а не в середине пайплайна.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["django".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(idx("python_preflight") < idx("py_venv_create"));
        assert!(idx("python_preflight") < idx("py_pip_install"));
        // django_start декларируется в фазе 1 (root-скаффолд), но его
        // предусловие py_pip_install (топологически) ставит django-admin
        // ПОСЛЕ установки манифеста — venv/pip гарантированно готовы.
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == "django_start" && d.prereq_id == "py_pip_install"),
            "{:?}",
            recipe.dependencies
        );
        let preflight = &recipe.steps[idx("python_preflight")];
        match preflight {
            Step::Command {
                command, on_error, ..
            } => {
                assert!(
                    command == "python" || command == "python3",
                    "интерпретатор через python_command(): {command}"
                );
                assert_eq!(on_error, &ErrorMode::Abort);
            }
            _ => panic!("python_preflight — Command"),
        }
    }

    #[test]
    fn py_venv_create_skips_when_django_created_venv_early() {
        // Ранний django-venv (django_venv_create) создаёт окружение ДО
        // tools-фазы; штатный py_venv_create не должен дублировать работу —
        // условие FileNotExists venv/pyvenv.cfg (seg-префикс в split).
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let venv = recipe
            .steps
            .iter()
            .find(|s| s.id() == "py_venv_create")
            .expect("py_venv_create должен быть в плане");
        match venv {
            Step::Command {
                condition, command, ..
            } => {
                match condition {
                    Some(StepCondition::FileNotExists { path }) => {
                        assert_eq!(path, "backend/venv/pyvenv.cfg", "маркер в сегменте")
                    }
                    other => panic!("ожидали FileNotExists: {other:?}"),
                }
                assert!(command == "python" || command == "python3");
            }
            _ => panic!("py_venv_create — Command"),
        }
    }

    #[test]
    fn py_pip_upgrade_precedes_pip_install() {
        // Bootstrap pip (python -m pip install --upgrade pip) идёт ДО
        // py_pip_install: старые окружения несут устаревший pip, который
        // ломает установку requirements.
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let idx = |id: &str| {
            recipe
                .steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"))
        };
        assert!(idx("py_pip_upgrade") < idx("py_pip_install"));
        let upgrade = &recipe.steps[idx("py_pip_upgrade")];
        match upgrade {
            Step::Command { command, args, .. } => {
                assert!(command.contains("venv"), "pip из venv: {command}");
                let js = args.join(" ");
                assert!(js.contains("--upgrade") && js.ends_with("pip"), "{js}");
            }
            _ => panic!("py_pip_upgrade — Command"),
        }
    }

    #[test]
    fn alembic_init_runs_in_python_segment_dir() {
        // alembic init создаёт migrations/ В КАТАЛОГЕ python-сегмента
        // (backend/), рядом с venv и requirements.txt, а не в корне проекта.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["fastapi".into(), "react".into()];
        ctx.tools = vec!["alembic".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let alembic = recipe
            .steps
            .iter()
            .find(|s| s.id() == "alembic_init")
            .expect("alembic_init должен быть в плане");
        match alembic {
            Step::Command { working_dir, .. } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("C:\\dev\\myapp/backend"),
                    "alembic init работает в каталоге python-сегмента"
                );
            }
            _ => panic!("alembic_init — Command"),
        }
    }

    #[test]
    fn python_root_project_uses_canonical_venv_and_manifest_at_root() {
        // Корневой Python-проект (backend-only, fastapi): единый канонический
        // venv лежит в <root>/venv, манифест — <root>/requirements.txt,
        // пост-валидация требует fastapi И ASGI-сервер (uvicorn).
        let mut ctx = context();
        ctx.languages = vec!["python".into()];
        ctx.frameworks = vec!["fastapi".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "py_venv_create"), "venv/pyvenv.cfg");
        assert_eq!(
            write_path_of(find_step(&recipe, "requirements_txt")),
            "requirements.txt"
        );
        match find_step(&recipe, "requirements_txt") {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("fastapi[standard]"), "{content}");
                assert!(
                    content.contains("uvicorn"),
                    "ASGI-сервер обязателен: {content}"
                );
            }
            _ => unreachable!(),
        }
        match find_step(&recipe, "py_requirements_check") {
            Step::Generate {
                generator_id,
                generator_config,
                ..
            } => {
                assert_eq!(generator_id, "manifest-check");
                assert_eq!(
                    generator_config.get("path").and_then(|v| v.as_str()),
                    Some("requirements.txt")
                );
                let deps = gen_strs(generator_config, "required_dependencies");
                assert!(deps.contains(&"fastapi".to_string()), "{deps:?}");
                assert!(deps.contains(&"uvicorn".to_string()), "{deps:?}");
            }
            _ => panic!("py_requirements_check — Generate"),
        }
    }

    #[test]
    fn python_backend_segment_places_venv_and_manifest_in_backend() {
        // Split-проект (python backend + typescript frontend): venv и
        // requirements.txt живут ВНУТРИ backend/, маркер — root-relative
        // backend/venv/pyvenv.cfg, django задекларирован в манифесте.
        let mut ctx = context();
        ctx.languages = vec!["python".into(), "typescript".into()];
        ctx.backend_languages = vec!["python".into()];
        ctx.frontend_languages = vec!["typescript".into()];
        ctx.frameworks = vec!["django".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(
            find_step(&recipe, "py_venv_create"),
            "backend/venv/pyvenv.cfg",
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "requirements_txt")),
            "backend/requirements.txt"
        );
        match find_step(&recipe, "requirements_txt") {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("django"), "{content}");
            }
            _ => unreachable!(),
        }
        match find_step(&recipe, "py_requirements_check") {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("path").and_then(|v| v.as_str()),
                    Some("backend/requirements.txt")
                );
                let deps = gen_strs(generator_config, "required_dependencies");
                assert!(deps.contains(&"django".to_string()), "{deps:?}");
            }
            _ => panic!("py_requirements_check — Generate"),
        }
    }

    // ==================== Scenario E: Zig / Flutter ====================

    #[test]
    fn zig_init_uses_generates_root_shell_capability() {
        // `zig init` раскладывает shell в текущем каталоге (способность
        // generates_root_shell), пост-условия — build.zig + build.zig.zon.
        // В split (zig + flutter) каталогом становится backend/.
        let mut ctx = context();
        ctx.languages = vec!["zig".into()];
        ctx.frameworks = vec![];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let zig = recipe
            .steps
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init должен быть в плане");
        match zig {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("zig")
                );
                assert_eq!(gen_args(generator_config), vec!["init".to_string()]);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell")
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["build.zig".to_string(), "build.zig.zon".to_string()]
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some(".")
                );
                assert_eq!(on_error, &ErrorMode::Skip);
            }
            _ => panic!("zig_init — Generate"),
        }

        // Split: zig + flutter → zig init работает в backend/
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let zig = recipe
            .steps
            .iter()
            .find(|s| s.id() == "zig_init")
            .expect("zig_init должен быть в плане (zig-cli не подавляет zig-скаффолд)");
        match zig {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("backend"),
                    "в split zig init работает в backend/"
                );
            }
            _ => panic!("zig_init — Generate"),
        }
    }

    #[test]
    fn flutter_create_uses_project_name_flag_in_current_directory() {
        // `flutter create --project-name <safe> .` работает ВНУТРИ frontend/
        // (creates_in_current_directory) — без вложенной матрёшки
        // frontend/<name>/; пост-условия — pubspec.yaml + lib/.
        let mut ctx = context();
        ctx.project_name = Some("my-app".into());
        ctx.languages = vec!["dart".into()];
        ctx.frameworks = vec!["flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let flutter = recipe
            .steps
            .iter()
            .find(|s| s.id() == "flutter_create")
            .expect("flutter_create должен быть в плане");
        match flutter {
            Step::Generate {
                generator_config, ..
            } => {
                assert_eq!(
                    generator_config.get("command").and_then(|v| v.as_str()),
                    Some("flutter")
                );
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("create"));
                assert_eq!(args.get(1).map(String::as_str), Some("--project-name"));
                assert_eq!(
                    args.get(2).map(String::as_str),
                    Some("my_app"),
                    "дефис в имени → валидный Dart-пакет"
                );
                assert_eq!(
                    args.get(3).map(String::as_str),
                    Some("__TARGET__"),
                    "CLI работает в каталоге назначения, без вложенной папки: {args:?}"
                );
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_in_current_directory")
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["pubspec.yaml".to_string(), "lib".to_string()]
                );
            }
            _ => panic!("flutter_create — Generate"),
        }
    }

    #[test]
    fn flutter_preflight_aborts_when_sdk_missing() {
        // Отсутствующий flutter обязан ОСТАНОВИТЬ пайплайн (Abort) с
        // понятной причиной, а не маскироваться скипом каркаса: префлайт
        // проверяет SDK строго ДО flutter create.
        let mut ctx = context();
        ctx.languages = vec!["dart".into()];
        ctx.frameworks = vec!["flutter".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let pre = recipe
            .steps
            .iter()
            .find(|s| s.id() == "flutter_preflight")
            .expect("flutter_preflight должен быть в плане");
        match pre {
            Step::Command {
                command,
                args,
                on_error,
                ..
            } => {
                assert_eq!(command, "flutter");
                assert_eq!(args, &vec!["--version".to_string()]);
                assert_eq!(
                    on_error,
                    &ErrorMode::Abort,
                    "SDK отсутствует — пайплайн останавливается"
                );
            }
            _ => panic!("flutter_preflight — Command"),
        }

        let pre_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "flutter_preflight")
            .unwrap();
        let create_idx = recipe
            .steps
            .iter()
            .position(|s| s.id() == "flutter_create")
            .unwrap();
        assert!(pre_idx < create_idx, "префлайт ДО flutter create");
    }

    #[test]
    fn scaffold_ownership_table_describes_known_frameworks() {
        // Единственная таблица владения: поведение каждого скаффолдера
        // описано здесь, движок не делает выводов по id фреймворка.
        let tauri = ScaffoldOwnership::for_framework("tauri");
        assert!(tauri.creates_app_shell);
        assert!(!tauri.creates_frontend, "tauri — обёртка, фронтенд чужой");
        assert!(
            tauri.wraps_existing_project,
            "tauri init требует готовый фронтенд"
        );
        assert!(tauri.may_run_in_existing_dir);
        assert!(!tauri.supports_staging_dir);
        assert_eq!(
            tauri.expected_outputs,
            &["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"]
        );

        let electron = ScaffoldOwnership::for_framework("electron");
        assert!(
            electron.creates_frontend_shell(),
            "electron: main + renderer"
        );
        assert!(!electron.is_ui_companion);
        assert!(
            electron.supports_staging_dir,
            "Forge init — только в пустой папке (temp+move)"
        );
        assert!(!electron.may_run_in_existing_dir);
        assert_eq!(electron.expected_outputs, &["package.json"]);

        let flutter = ScaffoldOwnership::for_framework("flutter");
        assert!(flutter.creates_frontend_shell(), "flutter: dart-ui");
        assert!(!flutter.is_ui_companion);
        assert!(
            flutter.may_run_in_existing_dir,
            "flutter create работает ВНУТРИ каталога"
        );
        assert!(!flutter.supports_staging_dir);
        assert_eq!(flutter.expected_outputs, &["pubspec.yaml", "lib"]);

        let qt = ScaffoldOwnership::for_framework("qt");
        assert!(qt.creates_app_shell);
        assert!(
            !qt.creates_frontend,
            "qt: собственный UI-стек, веб-часть — компаньон"
        );
        assert!(
            !qt.wraps_existing_project,
            "qt собирается в своём каталоге, не поверх проекта"
        );

        // UI-компаньоны — встраиваемые библиотеки под оболочку.
        for companion in ["react", "vue", "svelte"] {
            let o = ScaffoldOwnership::for_framework(companion);
            assert!(o.is_ui_companion, "{companion} — UI-компаньон");
            assert!(
                o.creates_frontend_shell(),
                "{companion} — полный фронтенд-каркас"
            );
            assert!(o.requires_empty_dir);
        }
        // Веб-фреймворки и прочие владельцы компаньонами не являются.
        for owner in ["nextjs", "sveltekit", "nuxt", "expo", "solidjs"] {
            assert!(
                !ScaffoldOwnership::for_framework(owner).is_ui_companion,
                "{owner}"
            );
        }

        // Inplace-фреймворки (express, fastapi, axum...) каркас не создают.
        assert_eq!(
            ScaffoldOwnership::for_framework("express"),
            ScaffoldOwnership::none()
        );
        assert_eq!(
            ScaffoldOwnership::for_framework("fastapi"),
            ScaffoldOwnership::none()
        );
        assert_eq!(
            ScaffoldOwnership::for_framework("unknown-fw"),
            ScaffoldOwnership::none()
        );
    }

    #[test]
    fn electron_scaffolds_own_frontend_and_suppresses_companion() {
        // electron + react: у electron СОБСТВЕННЫЙ renderer (create-electron-app
        // собирает main + renderer), поэтому react — UI-компаньон — НЕ
        // скаффолдится отдельно: два несвязанных фронтенда в frontend/
        // запрещены. electron_init — scaffold-генератор (temp+move: Forge init
        // в непустом каталоге назначения падает), шаблон renderer'а
        // фиксируется флагом --template.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["electron".into(), "react".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        assert!(
            !recipe.steps.iter().any(|s| s.id() == "vite_create"),
            "react не скаффолдится: electron владеет фронтендом"
        );

        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "electron_init")
            .expect("electron_init должен быть в плане");
        match init {
            Step::Generate {
                generator_id,
                generator_config,
                on_error,
                ..
            } => {
                assert_eq!(generator_id, "scaffold");
                assert_eq!(on_error, &ErrorMode::Skip);
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("creates_project_and_may_prompt")
                );
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend"),
                    "electron живёт в frontend/ (собственный renderer)"
                );
                let args = gen_args(generator_config);
                assert!(args.contains(&"__TARGET__".to_string()), "{args:?}");
                assert!(
                    args.iter().any(|a| a == "--template"),
                    "шаблон renderer'а фиксируется флагом: {args:?}"
                );
                assert!(
                    args.iter().any(|a| a == "typescript"),
                    "TS-стек → typescript-шаблон: {args:?}"
                );
                assert_eq!(
                    gen_strs(generator_config, "expected_outputs"),
                    vec!["package.json".to_string()]
                );
                assert!(generator_config
                    .get("interactive")
                    .and_then(|v| v.as_array())
                    .is_some_and(|entries| entries.iter().any(|e| e
                        .get("trigger")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t.contains("git repository")))));
            }
            _ => panic!("electron_init — Generate scaffold"),
        }

        // npm install ровно один раз, в frontend/ (node_modules не плодятся).
        let installs: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| s.id().starts_with("npm_install"))
            .collect();
        assert_eq!(
            installs.len(),
            1,
            "один npm install на каталог с package.json"
        );
        match installs[0] {
            Step::Command { working_dir, .. } => {
                assert_eq!(working_dir.as_deref(), Some("C:\\dev\\myapp/frontend"));
            }
            _ => panic!("npm_install — Command"),
        }
    }

    #[test]
    fn four_target_combos_keep_legal_step_order() {
        // Четыре целевых стека: итоговый ПОРЯДОК шагов плана для каждого —
        // легальная последовательность (см. отчёт по сценариям).
        // Порядок проверяется на уровне plan() (topo_order_steps): связи,
        // объявленные в рецепте (qt_web_build после vite_create), в
        // compose_recipe гарантируются именно топологической сортировкой.
        let engine = DefaultRecipeEngine::new();
        let plan_for = |ctx: &WizardContext| -> ExecutionPlan {
            engine
                .plan(ctx, std::path::Path::new("C:\\dev\\myapp"))
                .expect("plan должен собраться")
        };
        let idx = |plan: &ExecutionPlan, id: &str| -> usize {
            plan.steps
                .iter()
                .position(|s| s.id() == id)
                .unwrap_or_else(|| panic!("шаг {id} отсутствует в плане"))
        };
        let installs = |plan: &ExecutionPlan| -> usize {
            plan.steps
                .iter()
                .filter(|s| s.id().starts_with("npm_install"))
                .count()
        };

        // (a) Tauri + Svelte + TypeScript: фронтенд FIRST → tauri init.
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let plan_a = plan_for(&ctx);
        assert!(
            idx(&plan_a, "vite_create") < idx(&plan_a, "tauri_init"),
            "(a) svelte-фронтенд ДО tauri init"
        );
        assert!(
            idx(&plan_a, "tauri_init") < idx(&plan_a, "tauri_config_patch"),
            "(a) config-патч после init"
        );
        assert!(
            !plan_a.steps.iter().any(|s| s.id() == "cargo_init"),
            "(a) rust-языковой скаффолд подавлен tauri"
        );
        assert_eq!(installs(&plan_a), 1, "(a) один npm install");

        // (b) Electron + React + TypeScript: react подавлен (собственный
        // renderer electron), один npm install в frontend/.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["electron".into(), "react".into()];
        let plan_b = plan_for(&ctx);
        assert!(
            !plan_b.steps.iter().any(|s| s.id() == "vite_create"),
            "(b) react подавлен electron"
        );
        assert_eq!(installs(&plan_b), 1, "(b) один npm install в frontend/");

        // (c) Qt WebEngine + Vue + TypeScript: веб-сборка после vite_create,
        // cmake — после qt_cmake; сборка ждёт собранный фронтенд.
        let mut ctx = context();
        ctx.languages = vec!["cpp".into(), "typescript".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "vue".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let plan_c = plan_for(&ctx);
        assert!(
            idx(&plan_c, "vite_create") < idx(&plan_c, "qt_web_build"),
            "(c) vue-фронтенд ДО веб-сборки qt"
        );
        assert!(
            idx(&plan_c, "qt_cmake") < idx(&plan_c, "qt_cmake_configure")
                && idx(&plan_c, "qt_cmake_configure") < idx(&plan_c, "qt_cmake_build"),
            "(c) cmake: configure после CMakeLists, build после configure"
        );
        assert!(
            idx(&plan_c, "qt_web_build") < idx(&plan_c, "qt_cmake_build"),
            "(c) сборка после собранного фронтенда"
        );

        // (d) Zig CLI + Zap + Flutter: zig-shell в backend/, flutter в
        // frontend/; префлайт SDK ДО каркаса.
        let mut ctx = context();
        ctx.languages = vec!["zig".into(), "dart".into()];
        ctx.frameworks = vec!["zig-cli".into(), "zap".into(), "flutter".into()];
        let plan_d = plan_for(&ctx);
        assert!(
            idx(&plan_d, "zig_init") < idx(&plan_d, "flutter_create"),
            "(d) zig-shell в backend/ до flutter-каркаса"
        );
        assert!(
            idx(&plan_d, "flutter_preflight") < idx(&plan_d, "flutter_create"),
            "(d) префлайт SDK ДО каркаса"
        );
        assert!(
            idx(&plan_d, "zap_zon") < idx(&plan_d, "zap_fetch")
                && idx(&plan_d, "zap_fetch") < idx(&plan_d, "zap_main"),
            "(d) zon → fetch → entry"
        );

        // Сводка порядков для отчёта (cargo test -- --nocapture).
        for (name, plan) in [
            ("(a) tauri+svelte+ts", &plan_a),
            ("(b) electron+react+ts", &plan_b),
            ("(c) qt-webengine+vue+ts", &plan_c),
            ("(d) zig-cli+zap+flutter", &plan_d),
        ] {
            println!(
                "[{name}] {}",
                plan.steps
                    .iter()
                    .map(|s| s.id())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
    }

    // ==================== Scenario G: Qt WebEngine ====================

    #[test]
    fn qt_webengine_build_and_cmake_steps_use_segment_dirs() {
        // qt-webengine + react: веб-сборка работает в frontend/ (npm run
        // build), cmake-шаги — в каталоге qt-сегмента (backend/); условия
        // FileExists seg-префиксованы (into_segment не трогает condition).
        let mut ctx = context();
        ctx.languages = vec!["cpp".into()];
        ctx.frameworks = vec!["qt".into(), "qt-webengine".into(), "react".into()];
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");

        let web_build = recipe
            .steps
            .iter()
            .find(|s| s.id() == "qt_web_build")
            .expect("qt_web_build должен быть в плане");
        match web_build {
            Step::Command {
                working_dir,
                condition,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("frontend"),
                    "веб-сборка в frontend/ (рядом с qt-сегментом)"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "frontend/package.json")
                    }
                    other => panic!("ожидали FileExists frontend/package.json: {other:?}"),
                }
            }
            _ => panic!("qt_web_build — Command"),
        }

        let cmake = recipe
            .steps
            .iter()
            .find(|s| s.id() == "qt_cmake_configure")
            .expect("qt_cmake_configure должен быть в плане");
        match cmake {
            Step::Command {
                working_dir,
                condition,
                ..
            } => {
                assert_eq!(
                    working_dir.as_deref(),
                    Some("backend"),
                    "cmake работает в каталоге qt-сегмента (относительно корня проекта)"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "backend/CMakeLists.txt")
                    }
                    other => panic!("ожидали FileExists backend/CMakeLists.txt: {other:?}"),
                }
            }
            _ => panic!("qt_cmake_configure — Command"),
        }
    }

    // ==================== Scenario H: Go / Gin / Cobra / SolidStart ====================

    #[test]
    fn gin_go_get_uses_at_latest_and_is_gated_on_gomod() {
        // go get pkg@latest — современная форма (обновляет go.mod); условие
        // FileExists go.mod (seg-префикс backend/) скипает шаг без go.mod
        // вместо создания модуля в неверном каталоге.
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["gin".into(), "solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let get = recipe
            .steps
            .iter()
            .find(|s| s.id() == "get_gin")
            .expect("get_gin должен быть в плане");
        match get {
            Step::Command {
                args, condition, ..
            } => {
                assert_eq!(
                    args,
                    &vec![
                        "get".to_string(),
                        "github.com/gin-gonic/gin@latest".to_string()
                    ],
                    "go get pkg@latest: {args:?}"
                );
                match condition {
                    Some(StepCondition::FileExists { path }) => assert_eq!(path, "backend/go.mod"),
                    other => panic!("ожидали FileExists backend/go.mod: {other:?}"),
                }
            }
            _ => panic!("get_gin — Command"),
        }
    }

    #[test]
    fn cobra_steps_gated_on_gomod_in_backend_segment() {
        // cobra-cli init падает «go.mod file not found» без модуля: оба шага
        // (install + init) имеют условие FileExists go.mod (seg-префикс).
        let mut ctx = context();
        ctx.languages = vec!["go".into(), "typescript".into()];
        ctx.frameworks = vec!["cobra".into(), "solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        for id in ["cobra_install", "cobra_init"] {
            let step = recipe
                .steps
                .iter()
                .find(|s| s.id() == id)
                .unwrap_or_else(|| panic!("{id} должен быть в плане"));
            match step {
                Step::Command { condition, .. } => match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "backend/go.mod", "{id}: условие seg-префиксовано")
                    }
                    other => panic!("{id}: ожидали FileExists backend/go.mod: {other:?}"),
                },
                _ => panic!("{id} — Command"),
            }
        }
    }

    #[test]
    fn solidstart_noninteractive_flags() {
        // create-solid: позиционные projectName+template, --solidstart --v2
        // (без --v2 CLI спрашивает версию SolidStart), --ts (язык) —
        // полный неинтерактивный набор; шаблон "basic" валиден для
        // SolidStart (в отличие от "ts").
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["solidjs".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let solid = recipe
            .steps
            .iter()
            .find(|s| s.id() == "solid_init")
            .expect("solid_init должен быть в плане");
        match solid {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("--yes"), "{args:?}");
                assert_eq!(
                    args.get(1).map(String::as_str),
                    Some("create-solid"),
                    "{args:?}"
                );
                assert_eq!(
                    args.get(3).map(String::as_str),
                    Some("basic"),
                    "шаблон basic (валиден для SolidStart): {args:?}"
                );
                assert!(args.contains(&"--solidstart".to_string()), "{args:?}");
                assert!(
                    args.contains(&"--v2".to_string()),
                    "без --v2 промпт версии: {args:?}"
                );
                assert!(args.contains(&"--ts".to_string()), "{args:?}");
                assert_eq!(
                    generator_config.get("target_dir").and_then(|v| v.as_str()),
                    Some("frontend")
                );
            }
            _ => panic!("solid_init — Generate"),
        }
    }

    #[test]
    fn nuxt_git_init_false_is_single_token() {
        // citty (nuxi) не принимает `--gitInit false` пробелом для boolean —
        // только один токен --gitInit=false, иначе nuxi игнорирует значение.
        let mut ctx = context();
        ctx.languages = vec!["typescript".into()];
        ctx.frameworks = vec!["nuxt".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let nuxt = recipe
            .steps
            .iter()
            .find(|s| s.id() == "nuxt_create")
            .expect("nuxt_create должен быть в плане");
        match nuxt {
            Step::Generate {
                generator_config, ..
            } => {
                let args = gen_args(generator_config);
                assert!(
                    args.contains(&"--gitInit=false".to_string()),
                    "gitInit=false одним токеном: {args:?}"
                );
                let git_idx = args.iter().position(|a| a == "--gitInit");
                assert!(git_idx.is_none(), "пробельный вариант недопустим: {args:?}");
                assert!(args.contains(&"--no-install".to_string()), "{args:?}");
                assert!(args.contains(&"--packageManager".to_string()), "{args:?}");
            }
            _ => panic!("nuxt_create — Generate"),
        }
    }

    // ==================== Scenario D: Tauri ====================

    #[test]
    fn tauri_init_gated_on_frontend_package_json_and_expects_cargo_toml() {
        // tauri init: --yes (npx prompt), пост-условия tauri.conf.json +
        // Cargo.toml, условие FileExists frontend/package.json (init только
        // когда фронтенд-каркас реально создан).
        let mut ctx = context();
        ctx.languages = vec!["rust".into(), "typescript".into()];
        ctx.frameworks = vec!["tauri".into(), "svelte".into()];
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        let init = recipe
            .steps
            .iter()
            .find(|s| s.id() == "tauri_init")
            .expect("tauri_init должен быть в плане");
        match init {
            Step::Generate {
                generator_config,
                condition,
                ..
            } => {
                let args = gen_args(generator_config);
                assert_eq!(args.get(0).map(String::as_str), Some("--yes"), "{args:?}");
                assert!(args.contains(&"--ci".to_string()), "{args:?}");
                match condition {
                    Some(StepCondition::FileExists { path }) => {
                        assert_eq!(path, "frontend/package.json")
                    }
                    other => panic!("ожидали FileExists frontend/package.json: {other:?}"),
                }
                let expected = gen_strs(generator_config, "expected_outputs");
                assert!(
                    expected.contains(&"src-tauri/tauri.conf.json".to_string()),
                    "{expected:?}"
                );
                assert!(
                    expected.contains(&"src-tauri/Cargo.toml".to_string()),
                    "Cargo.toml — вторая обязательная часть shell: {expected:?}"
                );
                assert_eq!(
                    generator_config.get("capability").and_then(|v| v.as_str()),
                    Some("generates_root_shell")
                );
            }
            _ => panic!("tauri_init — Generate"),
        }
    }

    // ==================== Validation pass: 12-stack scenario matrix ====================
    // Каждый тест проверяет КОНКРЕТНЫЙ стек: класс раскладки, порядок шагов,
    // команды/аргументы, рабочие директории, условия (root-relative), таймауты,
    // зависимости и размещение финального npm install.

    fn ctx_scenario(languages: &[&str], frameworks: &[&str], tools: &[&str]) -> WizardContext {
        WizardContext {
            project_name: Some("myapp".into()),
            project_path: Some("C:\\dev\\myapp".into()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            docker: false,
            ..Default::default()
        }
    }

    fn plan_ids(recipe: &Recipe) -> Vec<String> {
        recipe.steps.iter().map(|s| s.id()).collect()
    }

    fn find_step<'a>(recipe: &'a Recipe, id: &str) -> &'a Step {
        recipe
            .steps
            .iter()
            .find(|s| s.id() == id)
            .unwrap_or_else(|| panic!("step '{id}' is missing from the plan"))
    }

    /// Каждый следующий id обязан встретиться ПОСЛЕ предыдущего (подпоследовательность).
    fn assert_order(recipe: &Recipe, expected: &[&str]) {
        let ids = plan_ids(recipe);
        let mut pos = 0;
        for want in expected {
            let found = ids[pos..]
                .iter()
                .position(|id| id == want)
                .unwrap_or_else(|| panic!("'{want}' missing after {:?}", &ids[..pos]));
            pos += found + 1;
        }
    }

    fn assert_dep(recipe: &Recipe, step: &str, prereq: &str) {
        assert!(
            recipe
                .dependencies
                .iter()
                .any(|d| d.step_id == step && d.prereq_id == prereq),
            "dependency {step} <- {prereq} is missing"
        );
    }

    fn command_of(step: &Step) -> (String, Vec<String>) {
        match step {
            Step::Command { command, args, .. } => (command.clone(), args.clone()),
            other => panic!("expected Command, got {:?}", other.id()),
        }
    }

    fn wd_of(step: &Step) -> String {
        match step {
            Step::Command { working_dir, .. } => working_dir
                .clone()
                .unwrap_or_else(|| panic!("working_dir unset")),
            other => panic!("expected Command, got {:?}", other.id()),
        }
    }

    fn write_path_of(step: &Step) -> String {
        match step {
            Step::WriteFile { path, .. } => path.clone(),
            other => panic!("expected WriteFile, got {:?}", other.id()),
        }
    }

    fn assert_file_exists(step: &Step, path: &str) {
        match step.condition() {
            Some(StepCondition::FileExists { path: p }) => assert_eq!(p, path, "FileExists path"),
            other => panic!("expected FileExists({path}), got {other:?}"),
        }
    }

    fn assert_file_not_exists(step: &Step, path: &str) {
        match step.condition() {
            Some(StepCondition::FileNotExists { path: p }) => {
                assert_eq!(p, path, "FileNotExists path")
            }
            other => panic!("expected FileNotExists({path}), got {other:?}"),
        }
    }

    fn gen_policy(step: &Step) -> Option<FilePolicy> {
        match step {
            Step::Generate { policy, .. } => *policy,
            other => panic!("expected Generate, got {:?}", other.id()),
        }
    }

    fn assert_no_npm_install(recipe: &Recipe) {
        assert!(
            !plan_ids(recipe)
                .iter()
                .any(|id| id.starts_with("npm_install")),
            "no JS framework — npm install must not be scheduled"
        );
    }

    #[test]
    fn validation_s1_nest_telegraf_nextjs_split_with_docker_services() {
        let mut ctx = ctx_scenario(
            &["typescript"],
            &["nest", "telegraf", "nextjs"],
            &["postgresql", "redis"],
        );
        ctx.docker = true;
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");
        assert!(layout.eager_dirs().contains(&"backend".to_string()));
        assert!(layout.eager_dirs().contains(&"frontend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "nest_new",
                "nest_pkg_name",
                "nextjs_create",
                "nextjs_pkg_name",
                "telegraf_bot",
                "telegraf_pkg_patch",
                "env_example",
                "git_cleanup_nested",
                "git_init",
                "dockerfile",
                "docker_ignore",
                "docker_compose",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "npm_install_1",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "nest_pkg_name", "nest_new");
        assert_dep(&recipe, "telegraf_pkg_patch", "nest_new");

        // nest: npx @nestjs/cli new . внутри backend/, с interactive-ответом на пакетный менеджер.
        let (cmd, args) = command_of(find_step(&recipe, "nest_new"));
        assert_eq!(cmd, "npx");
        assert_eq!(
            args,
            vec![
                "--yes",
                "@nestjs/cli",
                "new",
                ".",
                "--package-manager",
                "npm",
                "--skip-install",
                "--skip-git",
            ]
        );
        assert!(wd_of(find_step(&recipe, "nest_new")).ends_with("/backend"));
        let nest_pkg = find_step(&recipe, "nest_pkg_name");
        assert_file_exists(nest_pkg, "backend/package.json");
        assert_eq!(
            wd_of(nest_pkg),
            "backend",
            "package_name_patch работает в относительной backend/"
        );

        // telegraf: бот в backend/src/bot.js, патч package.json строго после nest.
        assert_eq!(
            write_path_of(find_step(&recipe, "telegraf_bot")),
            "backend/src/bot.js"
        );
        let patch = find_step(&recipe, "telegraf_pkg_patch");
        assert_file_exists(patch, "backend/package.json");
        assert!(wd_of(patch).ends_with("/backend"));
        assert_eq!(command_of(patch).0, "node");
        assert!(command_of(patch).1[0] == "-e");

        // nextjs: scaffold-генератор в frontend/ с --skip-install.
        let nextjs = find_step(&recipe, "nextjs_create");
        let args = gen_args(gen_config_of_step(nextjs));
        assert!(
            args.contains(&"create-next-app@latest".to_string()),
            "{args:?}"
        );
        assert!(args.contains(&"--skip-install".to_string()), "{args:?}");
        assert!(args.contains(&"--typescript".to_string()), "{args:?}");
        assert_eq!(
            gen_config_of_step(nextjs)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(gen_strs(gen_config_of_step(nextjs), "expected_outputs")
            .contains(&"package.json".to_string()));
        let nextjs_pkg = find_step(&recipe, "nextjs_pkg_name");
        assert_file_exists(nextjs_pkg, "frontend/package.json");
        assert_eq!(wd_of(nextjs_pkg), "frontend");

        // Инфра: .env.example + docker-compose с postgres и redis.
        let env = find_step(&recipe, "env_example");
        assert_eq!(write_path_of(env), ".env.example");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("POSTGRES_DB"), "{content}");
                assert!(content.contains("REDIS_URL"), "{content}");
            }
            _ => unreachable!(),
        }
        assert_eq!(
            write_path_of(find_step(&recipe, "dockerfile")),
            "backend/Dockerfile"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "docker_ignore")),
            "backend/.dockerignore"
        );
        let compose = find_step(&recipe, "docker_compose");
        assert_eq!(write_path_of(compose), "docker-compose.yaml");
        match compose {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("postgres"), "{content}");
                assert!(content.contains("redis"), "{content}");
            }
            _ => unreachable!(),
        }

        // Один npm install на backend (nest) и один на frontend (nextjs).
        let (cmd0, args0) = command_of(find_step(&recipe, "npm_install_0"));
        assert_eq!(cmd0, "npm");
        assert_eq!(args0, vec!["install"]);
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/backend"));
        assert_file_not_exists(find_step(&recipe, "npm_install_0"), "backend/node_modules");
        assert!(wd_of(find_step(&recipe, "npm_install_1")).ends_with("/frontend"));
        assert_file_not_exists(find_step(&recipe, "npm_install_1"), "frontend/node_modules");
    }

    #[test]
    fn validation_s2_laravel_react_composer_scaffold() {
        let ctx = ctx_scenario(&["php", "typescript"], &["laravel", "react"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "laravel_php_check",
                "laravel_new",
                "vite_create",
                "react_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        // Языковые scaffold'ы подавлены: laravel сам создаёт PHP-каркас, react — TS.
        assert!(
            !plan_ids(&recipe).contains(&"composer_json".to_string()),
            "php scaffold suppressed by laravel"
        );
        assert!(!plan_ids(&recipe).contains(&"tsc_init".to_string()));

        // PHP-префлайт: fileinfo обязателен до composer create-project.
        let preflight = find_step(&recipe, "laravel_php_check");
        let (cmd, args) = command_of(preflight);
        assert_eq!(cmd, "php");
        assert_eq!(args[0], "-r");
        assert!(args[1].contains("fileinfo"));
        assert!(matches!(error_mode_of(preflight), ErrorMode::Abort));

        // Composer: create-project laravel/laravel в backend/ (temp+move, SkipIfExists).
        let laravel = find_step(&recipe, "laravel_new");
        let gen_args_v = gen_args(gen_config_of_step(laravel));
        assert!(
            gen_args_v.contains(&"create-project".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"laravel/laravel".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"--no-interaction".to_string()),
            "{gen_args_v:?}"
        );
        assert!(
            gen_args_v.contains(&"--prefer-source".to_string()),
            "{gen_args_v:?}"
        );
        assert_eq!(
            gen_config_of_step(laravel)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        // Composer-скаффолд обязан оставить НАСТОЯЩИЙ composer.json
        // (не generic-фолбэк), иначе каркас не считается успешным.
        assert!(
            gen_strs(gen_config_of_step(laravel), "expected_outputs")
                .contains(&"composer.json".to_string()),
            "laravel_new обязан ожидать composer.json, а не package.json"
        );
        assert!(
            !gen_strs(gen_config_of_step(laravel), "expected_outputs")
                .contains(&"package.json".to_string()),
            "laravel — PHP-каркас, package.json ему не нужен"
        );
        assert!(matches!(
            gen_policy(laravel),
            Some(FilePolicy::SkipIfExists)
        ));

        // React в frontend/ через create-vite (react-ts).
        let vite = find_step(&recipe, "vite_create");
        let vargs = gen_args(gen_config_of_step(vite));
        assert!(vargs.contains(&"react-ts".to_string()), "{vargs:?}");
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert_file_exists(
            find_step(&recipe, "react_pkg_name"),
            "frontend/package.json",
        );
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s3_django_vue_python_venv_chain() {
        let ctx = ctx_scenario(
            &["python", "typescript"],
            &["django", "vue"],
            &["alembic", "sqlalchemy"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "node_preflight",
                "npm_preflight",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "django_start",
                "vite_create",
                "vue_pkg_name",
                "alembic_init",
                "sqlalchemy_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "py_venv_verify", "py_venv_create");
        assert_dep(&recipe, "py_pip_upgrade", "py_venv_create");
        assert_dep(&recipe, "py_pip_install", "py_pip_upgrade");
        assert_dep(&recipe, "py_pip_install", "py_pip_check");
        assert_dep(&recipe, "py_requirements_check", "py_pip_install");
        assert_dep(&recipe, "alembic_init", "py_pip_install");
        assert_dep(&recipe, "django_start", "py_pip_install");

        // Python-каркас сегментирован в backend/.
        match find_step(&recipe, "create_src") {
            Step::CreateDirectory { path, .. } => assert_eq!(path, "backend/src"),
            _ => panic!("create_src — CreateDirectory"),
        }
        let reqs = find_step(&recipe, "requirements_txt");
        assert_eq!(write_path_of(reqs), "backend/requirements.txt");
        match reqs {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("django"), "{content}");
                assert!(content.contains("sqlalchemy"), "{content}");
                assert!(content.contains("alembic"), "{content}");
            }
            _ => unreachable!(),
        }

        // Ранний venv: python -m venv backend/venv, маркер root-relative.
        // Единый канонический venv: python -m venv backend/venv, гейт —
        // root-relative маркер. Никакого отдельного django-venv.
        let venv_create = find_step(&recipe, "py_venv_create");
        let (cmd, args) = command_of(venv_create);
        assert_eq!(cmd, python_command());
        assert_eq!(&args[..2], &["-m".to_string(), "venv".to_string()]);
        assert!(args[2].contains("venv"), "{args:?}");
        assert_file_not_exists(venv_create, "backend/venv/pyvenv.cfg");
        assert!(matches!(error_mode_of(venv_create), ErrorMode::Abort));

        // django-admin строго из venv, в рабочей директории backend/.
        let start = find_step(&recipe, "django_start");
        assert!(
            command_of(start).0.contains("django-admin"),
            "{}",
            command_of(start).0
        );
        assert_eq!(command_of(start).1, vec!["startproject", "myapp", "."]);
        assert!(wd_of(start).ends_with("/backend"));

        // tools-фаза: единый venv, pip ставит РОВНО из манифеста (-r),
        // alembic задекларирован в манифесте, а не отдельным pip-вызовом.
        let pip = find_step(&recipe, "py_pip_install");
        let (_, pip_args) = command_of(pip);
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        assert!(pip_args.iter().any(|a| a == "-r"), "{pip_args:?}");
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic ставится из манифеста: {pip_args:?}"
        );
        let requirements = find_step(&recipe, "requirements_txt");
        match requirements {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("alembic"), "{content}");
            }
            _ => unreachable!(),
        }
        let alembic = find_step(&recipe, "alembic_init");
        assert!(command_of(alembic).0.contains("alembic"));
        assert_eq!(command_of(alembic).1, vec!["init", "migrations"]);
        assert!(wd_of(alembic).ends_with("/backend"));

        // ИЗВЕСТНОЕ ОГРАНИЧЕНИЕ (фиксируем текущее поведение): sqlalchemy_config пишется
        // в корень, а НЕ в backend/ (steps_for_tools не сегментируется).
        assert_eq!(
            write_path_of(find_step(&recipe, "sqlalchemy_config")),
            "src/database.py"
        );

        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s4_tauri_svelte_integrated() {
        let ctx = ctx_scenario(&["rust", "typescript"], &["tauri", "svelte"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "integrated");
        assert!(
            layout.eager_dirs().is_empty(),
            "integrated — никаких eager-директорий"
        );

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "vite_create",
                "svelte_pkg_name",
                "tauri_init",
                "tauri_config_patch",
                "tauri_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "tauri_config_patch", "tauri_init");
        assert_dep(&recipe, "tauri_pkg_name", "vite_create");

        // Компаньон svelte: НЕ tauri_web_scaffold/install (только vite).
        assert!(
            !plan_ids(&recipe).contains(&"tauri_web_scaffold".to_string()),
            "companion suppresses tauri_web_scaffold"
        );
        assert!(!plan_ids(&recipe).contains(&"tauri_web_install".to_string()));

        // svelte-ts через create-vite в frontend/.
        let vite = find_step(&recipe, "vite_create");
        assert!(gen_args(gen_config_of_step(vite)).contains(&"svelte-ts".to_string()));
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );

        // tauri init в корне, гейт на frontend/package.json, пост-условия shell.
        let init = find_step(&recipe, "tauri_init");
        assert_file_exists(init, "frontend/package.json");
        assert!(matches!(gen_policy(init), Some(FilePolicy::SkipIfExists)));
        let iargs = gen_args(gen_config_of_step(init));
        assert!(iargs.contains(&"--ci".to_string()), "{iargs:?}");
        assert!(iargs.contains(&"--app-name".to_string()), "{iargs:?}");
        assert_eq!(
            gen_config_of_step(init)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some(".")
        );
        let expected = gen_strs(gen_config_of_step(init), "expected_outputs");
        assert!(
            expected.contains(&"src-tauri/tauri.conf.json".to_string()),
            "{expected:?}"
        );
        assert!(
            expected.contains(&"src-tauri/Cargo.toml".to_string()),
            "{expected:?}"
        );

        assert_file_exists(
            find_step(&recipe, "tauri_config_patch"),
            "src-tauri/tauri.conf.json",
        );
        assert_file_exists(
            find_step(&recipe, "tauri_pkg_name"),
            "frontend/package.json",
        );
        assert_eq!(wd_of(find_step(&recipe, "tauri_pkg_name")), "frontend");
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s5_fastapi_backend_only_with_tools() {
        let ctx = ctx_scenario(
            &["python"],
            &["fastapi"],
            &["alembic", "ruff", "sqlalchemy"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "backend-only");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "fastapi_main",
                "alembic_init",
                "ruff_config",
                "sqlalchemy_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);

        // Всё в корне: venv = ./venv, требования = ./requirements.txt.
        match find_step(&recipe, "create_src") {
            Step::CreateDirectory { path, .. } => assert_eq!(path, "src"),
            _ => panic!("create_src — CreateDirectory"),
        }
        assert_eq!(
            write_path_of(find_step(&recipe, "fastapi_main")),
            "src/main.py"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "sqlalchemy_config")),
            "src/database.py"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "ruff_config")),
            "ruff.toml"
        );
        assert_file_not_exists(find_step(&recipe, "py_venv_create"), "venv/pyvenv.cfg");
        let (_, pip_args) = command_of(find_step(&recipe, "py_pip_install"));
        assert!(pip_args.iter().any(|a| a == "-r"), "{pip_args:?}");
        assert!(
            !pip_args.iter().any(|a| a == "alembic"),
            "alembic ставится из манифеста: {pip_args:?}"
        );
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        let alembic_wd = wd_of(find_step(&recipe, "alembic_init"));
        assert!(
            alembic_wd.ends_with("/myapp") || alembic_wd.ends_with("\\myapp"),
            "alembic работает в корне проекта (backend-only): {alembic_wd}"
        );
        let reqs = find_step(&recipe, "requirements_txt");
        assert_eq!(write_path_of(reqs), "requirements.txt");
        match reqs {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("fastapi[standard]"), "{content}");
                assert!(content.contains("ruff"), "{content}");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn validation_s6_zig_zap_flutter_prisma_drizzle() {
        let ctx = ctx_scenario(
            &["zig", "dart"],
            &["zig-cli", "zap", "flutter"],
            &["prisma", "drizzle"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "zig_init",
                "zig_cli_module",
                "zap_zon",
                "zap_build",
                "zap_fetch",
                "zap_main",
                "flutter_create",
                "prisma_init",
                "prisma_cleanup",
                "drizzle_config",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);
        assert!(
            !plan_ids(&recipe).contains(&"dart_create".to_string()),
            "flutter suppresses the dart scaffold"
        );

        // zig init: shell в backend/ (сегмент), SkipIfExists, пост-условия build.zig + zon.
        let zig_init = find_step(&recipe, "zig_init");
        assert!(matches!(
            gen_policy(zig_init),
            Some(FilePolicy::SkipIfExists)
        ));
        assert_eq!(
            gen_config_of_step(zig_init)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        let zexpected = gen_strs(gen_config_of_step(zig_init), "expected_outputs");
        assert!(
            zexpected.contains(&"build.zig".to_string()),
            "{zexpected:?}"
        );
        assert!(
            zexpected.contains(&"build.zig.zon".to_string()),
            "{zexpected:?}"
        );

        // zap (inplace-фреймворк): перезаписывает build.zig/zon в backend/.
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_zon")),
            "backend/build.zig.zon"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_build")),
            "backend/build.zig"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zap_main")),
            "backend/src/main.zig"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "zig_cli_module")),
            "backend/src/cli.zig"
        );
        let (zcmd, zargs) = command_of(find_step(&recipe, "zap_fetch"));
        assert_eq!(zcmd, "zig");
        assert!(zargs.contains(&"fetch".to_string()), "{zargs:?}");
        assert!(zargs.contains(&"--save".to_string()), "{zargs:?}");
        assert!(wd_of(find_step(&recipe, "zap_fetch")).ends_with("/backend"));

        // flutter create в frontend/ (creates_in_current_directory, SkipIfExists).
        let fl = find_step(&recipe, "flutter_create");
        let fargs = gen_args(gen_config_of_step(fl));
        assert!(fargs.contains(&"create".to_string()), "{fargs:?}");
        assert!(fargs.contains(&"--project-name".to_string()), "{fargs:?}");
        assert!(fargs.contains(&"myapp".to_string()), "{fargs:?}");
        assert_eq!(
            gen_config_of_step(fl)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(matches!(gen_policy(fl), Some(FilePolicy::SkipIfExists)));
        let fexpected = gen_strs(gen_config_of_step(fl), "expected_outputs");
        assert!(
            fexpected.contains(&"pubspec.yaml".to_string()),
            "{fexpected:?}"
        );
        assert!(fexpected.contains(&"lib".to_string()), "{fexpected:?}");

        // prisma init в корне (ИЗВЕСТНОЕ ОГРАНИЧЕНИЕ: без JS/TS-каркаса и package.json
        // в корне шаг упадёт на рантайме и будет молча пропущен через ErrorMode::Skip).
        let prisma = find_step(&recipe, "prisma_init");
        let (pcmd, pargs) = command_of(prisma);
        assert_eq!(pcmd, "npx");
        assert!(pargs.contains(&"prisma".to_string()), "{pargs:?}");
        assert!(pargs.contains(&"init".to_string()), "{pargs:?}");
        assert!(
            pargs.contains(&"--datasource-provider".to_string()),
            "{pargs:?}"
        );
        assert!(pargs.contains(&"sqlite".to_string()), "{pargs:?}");
        assert!(pargs.contains(&"--no-skills".to_string()), "{pargs:?}");
        let prisma_wd = wd_of(prisma);
        assert!(
            prisma_wd.ends_with("/myapp") || prisma_wd.ends_with("\\myapp"),
            "prisma init работает в корне проекта: {prisma_wd}"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "drizzle_config")),
            "drizzle.config.ts"
        );
    }

    #[test]
    fn validation_s7_symfony_nuxt() {
        let ctx = ctx_scenario(&["php", "typescript"], &["symfony", "nuxt"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "symfony_php_check",
                "symfony_new",
                "nuxt_create",
                "nuxt_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert!(!plan_ids(&recipe).contains(&"composer_json".to_string()));

        let symfony = find_step(&recipe, "symfony_new");
        let sargs = gen_args(gen_config_of_step(symfony));
        assert!(sargs.contains(&"create-project".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"symfony/skeleton".to_string()), "{sargs:?}");
        assert_eq!(
            gen_config_of_step(symfony)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("backend")
        );
        assert!(matches!(
            gen_policy(symfony),
            Some(FilePolicy::SkipIfExists)
        ));

        // nuxi init: неинтерактивен только с полным набором флагов.
        let nuxt = find_step(&recipe, "nuxt_create");
        let nargs = gen_args(gen_config_of_step(nuxt));
        assert!(nargs.contains(&"nuxi@latest".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"init".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--template".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--packageManager".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--gitInit=false".to_string()), "{nargs:?}");
        assert!(nargs.contains(&"--no-install".to_string()), "{nargs:?}");
        assert_eq!(
            gen_config_of_step(nuxt)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert_file_exists(find_step(&recipe, "nuxt_pkg_name"), "frontend/package.json");
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s8_qt_webengine_vue() {
        let mut ctx = ctx_scenario(&["cpp", "typescript"], &["qt", "vue"], &[]);
        ctx.answers
            .insert("qt_ui".into(), vec!["qt-webengine".into()]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");
        assert!(layout.eager_dirs().contains(&"backend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "qt_main",
                "qt_cmake",
                "qt_web_build",
                "qt_cmake_configure",
                "qt_cmake_build",
                "vite_create",
                "vue_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "qt_web_build", "vite_create");
        assert_dep(&recipe, "qt_cmake_configure", "qt_cmake");
        assert_dep(&recipe, "qt_cmake_build", "qt_web_build");
        assert_dep(&recipe, "qt_cmake_build", "qt_cmake_configure");
        // qt_cmake_build дополнительно требует собранный frontend.
        assert!(
            recipe.dependencies.iter().any(|d| {
                d.step_id == "qt_cmake_build"
                    && d.prereq_id == "qt_web_build"
                    && d.expects_file == "frontend/dist/index.html"
            }),
            "qt_cmake_build <- qt_web_build должен нести expects_file"
        );

        // Qt (inplace): main.cpp + CMakeLists в backend/, перезапись поверх zig/c++ каркаса.
        assert_eq!(
            write_path_of(find_step(&recipe, "qt_main")),
            "backend/src/main.cpp"
        );
        assert_eq!(
            write_path_of(find_step(&recipe, "qt_cmake")),
            "backend/CMakeLists.txt"
        );

        // Цепочка сборки WebEngine: веб-часть в frontend/, cmake в backend/.
        let web_build = find_step(&recipe, "qt_web_build");
        let (wcmd, wargs) = command_of(web_build);
        assert_eq!(wcmd, "npm");
        assert_eq!(wargs, vec!["run", "build"]);
        assert_eq!(wd_of(web_build), "frontend");
        assert_file_exists(web_build, "frontend/package.json");
        assert!(matches!(error_mode_of(web_build), ErrorMode::Skip));

        let configure = find_step(&recipe, "qt_cmake_configure");
        assert_eq!(command_of(configure).0, "cmake");
        assert_eq!(command_of(configure).1, vec!["-S", ".", "-B", "build"]);
        assert_eq!(wd_of(configure), "backend", "cmake работает в qt-сегменте");
        assert_file_exists(configure, "backend/CMakeLists.txt");
        assert!(matches!(error_mode_of(configure), ErrorMode::Abort));

        let build = find_step(&recipe, "qt_cmake_build");
        assert_eq!(command_of(build).0, "cmake");
        assert_eq!(command_of(build).1, vec!["--build", "build"]);
        assert_eq!(wd_of(build), "backend");
        // Root-relative пост-условие: собранный фронтенд, а НЕ backend/frontend/...
        assert_file_exists(build, "frontend/dist/index.html");
        assert!(matches!(error_mode_of(build), ErrorMode::Abort));

        // vue в frontend/ (vue-ts), финальный npm install.
        let vite = find_step(&recipe, "vite_create");
        assert!(gen_args(gen_config_of_step(vite)).contains(&"vue-ts".to_string()));
        assert_eq!(
            gen_config_of_step(vite)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s9_go_gin_cobra_solidjs() {
        let ctx = ctx_scenario(
            &["go", "typescript"],
            &["gin", "cobra", "solidjs"],
            &["mongodb"],
        );
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_backend_dir",
                "create_frontend_dir",
                "go_mod_init",
                "create_cmd",
                "create_internal",
                "main_go",
                "gin_main",
                "get_gin",
                "go_mod",
                "cobra_install",
                "cobra_init",
                "cobra_cli",
                "solid_init",
                "solidjs_pkg_name",
                "env_example",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "get_gin", "go_mod_init");
        assert_dep(&recipe, "cobra_init", "go_mod_init");

        // go mod init живёт в backend/ вместе со своим маркером (rerun-guard сегментирован).
        let go_mod = find_step(&recipe, "go_mod_init");
        assert_eq!(command_of(go_mod).0, "go");
        assert_eq!(command_of(go_mod).1, vec!["mod", "init", "myapp"]);
        assert!(wd_of(go_mod).ends_with("/backend"));
        assert_file_not_exists(go_mod, "backend/go.mod");

        assert_eq!(
            write_path_of(find_step(&recipe, "main_go")),
            "backend/cmd/main.go"
        );
        let gin_main = find_step(&recipe, "gin_main");
        assert_eq!(write_path_of(gin_main), "backend/cmd/main.go");
        assert!(matches!(
            step_file_policy_of(gin_main),
            FilePolicy::Overwrite
        ));
        let get_gin = find_step(&recipe, "get_gin");
        assert_file_exists(get_gin, "backend/go.mod");
        assert!(matches!(error_mode_of(get_gin), ErrorMode::Abort));

        // cobra (inplace-фреймворк): go.mod пишется поверх go-каркаса; шаги cobra
        // проходят override overwrite=true (scaffold не задан), как и gin_main.
        let cobra_mod = find_step(&recipe, "go_mod");
        assert_eq!(write_path_of(cobra_mod), "backend/go.mod");
        assert!(
            matches!(step_file_policy_of(cobra_mod), FilePolicy::Overwrite),
            "go_mod policy: {:?}",
            step_file_policy_of(cobra_mod)
        );
        assert_file_exists(find_step(&recipe, "cobra_init"), "backend/go.mod");
        assert_eq!(
            write_path_of(find_step(&recipe, "cobra_cli")),
            "backend/cmd/cli/main.go"
        );

        // solidjs: create-solid в frontend/, полный набор флагов для неинтерактивности.
        let solid = find_step(&recipe, "solid_init");
        let sargs = gen_args(gen_config_of_step(solid));
        assert!(sargs.contains(&"--yes".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"create-solid".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"basic".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--solidstart".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--v2".to_string()), "{sargs:?}");
        assert!(sargs.contains(&"--ts".to_string()), "{sargs:?}");
        assert_eq!(
            gen_config_of_step(solid)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );

        let env = find_step(&recipe, "env_example");
        match env {
            Step::WriteFile { content, .. } => {
                assert!(content.contains("MONGODB_URI"), "{content}");
            }
            _ => panic!("env_example — WriteFile"),
        }
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s10_frontend_only_nextjs() {
        let ctx = ctx_scenario(&["typescript"], &["nextjs"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "frontend-only");
        assert!(
            layout.eager_dirs().is_empty(),
            "frontend-only не создаёт eager-директории"
        );

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "nextjs_create",
                "nextjs_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "npm_install_0",
                "git_add",
                "git_commit",
            ],
        );
        let nextjs = find_step(&recipe, "nextjs_create");
        assert_eq!(
            gen_config_of_step(nextjs)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend"),
            "special case: nextjs в frontend-only раскладке — frontend/"
        );
        assert_file_exists(
            find_step(&recipe, "nextjs_pkg_name"),
            "frontend/package.json",
        );
        assert_eq!(wd_of(find_step(&recipe, "nextjs_pkg_name")), "frontend");
        assert!(wd_of(find_step(&recipe, "npm_install_0")).ends_with("/frontend"));
    }

    #[test]
    fn validation_s11_backend_only_fastapi_plain() {
        let ctx = ctx_scenario(&["python"], &["fastapi"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "backend-only");

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "create_src",
                "pyproject_toml",
                "requirements_txt",
                "python_preflight",
                "py_venv_create",
                "py_venv_verify",
                "py_pip_upgrade",
                "py_pip_check",
                "py_pip_install",
                "py_requirements_check",
                "fastapi_main",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_no_npm_install(&recipe);
        // Без инструмента alembic в pip-установке только -r requirements.txt.
        let (_, pip_args) = command_of(find_step(&recipe, "py_pip_install"));
        assert!(!pip_args.contains(&"alembic".to_string()), "{pip_args:?}");
        assert!(
            pip_args.iter().any(|a| a.contains("requirements.txt")),
            "{pip_args:?}"
        );
        assert_file_not_exists(find_step(&recipe, "git_init"), ".git/HEAD");
        // git_commit идемпотентен при повторном запуске.
        let (commit_cmd, _) = command_of(find_step(&recipe, "git_commit"));
        assert!(
            commit_cmd.contains("git diff --cached --quiet"),
            "{commit_cmd}"
        );
    }

    #[test]
    fn validation_s12_tauri_only_rust_vite_vanilla() {
        let ctx = ctx_scenario(&["rust"], &["tauri"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "integrated");
        assert!(!layout.eager_dirs().contains(&"backend".to_string()));

        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_order(
            &recipe,
            &[
                "create_root",
                "tauri_web_scaffold",
                "tauri_web_install",
                "tauri_init",
                "tauri_config_patch",
                "tauri_pkg_name",
                "git_cleanup_nested",
                "git_init",
                "gitignore",
                "readme",
                "vscode_merge",
                "merge_inner_vscode",
                "git_add",
                "git_commit",
            ],
        );
        assert_dep(&recipe, "tauri_web_install", "tauri_web_scaffold");
        assert_dep(&recipe, "tauri_pkg_name", "tauri_web_scaffold");
        // tauri — не JS-фреймворк (langs=[rust]): отдельного npm install НЕТ.
        assert_no_npm_install(&recipe);

        // Без компаньона фронтенд создаёт vite (vanilla), SkipIfExists.
        let web = find_step(&recipe, "tauri_web_scaffold");
        let wargs = gen_args(gen_config_of_step(web));
        assert!(
            wargs.contains(&"create-vite@latest".to_string()),
            "{wargs:?}"
        );
        assert!(wargs.contains(&"--template".to_string()), "{wargs:?}");
        assert!(wargs.contains(&"vanilla".to_string()), "{wargs:?}");
        assert_eq!(
            gen_config_of_step(web)
                .get("target_dir")
                .and_then(|v| v.as_str()),
            Some("frontend")
        );
        assert!(matches!(gen_policy(web), Some(FilePolicy::SkipIfExists)));

        // npm install строго после появления frontend/package.json.
        let install = find_step(&recipe, "tauri_web_install");
        assert_eq!(command_of(install).0, "npm");
        assert_eq!(wd_of(install), "frontend");
        assert_file_exists(install, "frontend/package.json");

        assert_file_exists(find_step(&recipe, "tauri_init"), "frontend/package.json");
        assert_file_exists(
            find_step(&recipe, "tauri_config_patch"),
            "src-tauri/tauri.conf.json",
        );
        assert_file_exists(
            find_step(&recipe, "tauri_pkg_name"),
            "frontend/package.json",
        );
    }

    #[test]
    fn validation_segment_markers_follow_working_dir() {
        // Rerun-guard маркеры языковых init-шагов относительны рабочей директории:
        // в Split-раскладке они обязаны уехать в сегмент вместе с командой.
        let ctx = ctx_scenario(&["rust", "typescript"], &["axum"], &[]);
        let layout = ProjectLayout::compute(&ctx);
        assert_eq!(layout.to_summary(&ctx).class, "split");
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "cargo_init"), "backend/Cargo.toml");
        assert!(wd_of(find_step(&recipe, "cargo_init")).ends_with("/backend"));
        assert_file_not_exists(find_step(&recipe, "tsc_init"), "frontend/tsconfig.json");
        assert!(wd_of(find_step(&recipe, "tsc_init")).ends_with("/frontend"));

        let ctx = ctx_scenario(&["go", "typescript"], &["solidjs"], &[]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "go_mod_init"), "backend/go.mod");

        let ctx = ctx_scenario(&["dart", "typescript"], &["vue"], &[]);
        let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
        assert_file_not_exists(
            find_step(&recipe, "dart_create"),
            "backend/myapp/pubspec.yaml",
        );

        // Монолит: маркер остаётся в корне (поведение без изменений).
        let mono = ctx_scenario(&["go"], &[], &[]);
        let recipe = recipe_for(&mono, "myapp").expect("recipe must build");
        assert_file_not_exists(find_step(&recipe, "go_mod_init"), "go.mod");
        let mono_wd = wd_of(find_step(&recipe, "go_mod_init"));
        assert!(
            mono_wd.ends_with("/myapp") || mono_wd.ends_with("\\myapp"),
            "монолит: go mod init работает в корне проекта: {mono_wd}"
        );
    }

    #[test]
    fn validation_all_scenario_commands_have_timeouts() {
        // Ни один Command в рецептах не должен остаться без таймаута (защита от зависаний).
        let qt_ctx = ctx_scenario(&["cpp", "typescript"], &["qt", "vue"], &[]);
        let scenarios: Vec<WizardContext> = vec![
            ctx_scenario(
                &["typescript"],
                &["nest", "telegraf", "nextjs"],
                &["postgresql", "redis"],
            ),
            ctx_scenario(&["php", "typescript"], &["laravel", "react"], &[]),
            ctx_scenario(
                &["python", "typescript"],
                &["django", "vue"],
                &["alembic", "sqlalchemy"],
            ),
            ctx_scenario(&["rust", "typescript"], &["tauri", "svelte"], &[]),
            ctx_scenario(
                &["python"],
                &["fastapi"],
                &["alembic", "ruff", "sqlalchemy"],
            ),
            ctx_scenario(
                &["zig", "dart"],
                &["zig-cli", "zap", "flutter"],
                &["prisma", "drizzle"],
            ),
            ctx_scenario(&["php", "typescript"], &["symfony", "nuxt"], &[]),
            qt_ctx,
            ctx_scenario(
                &["go", "typescript"],
                &["gin", "cobra", "solidjs"],
                &["mongodb"],
            ),
            ctx_scenario(&["typescript"], &["nextjs"], &[]),
            ctx_scenario(&["python"], &["fastapi"], &[]),
            ctx_scenario(&["rust"], &["tauri"], &[]),
        ];
        for ctx in scenarios {
            let recipe = recipe_for(&ctx, "myapp").expect("recipe must build");
            for step in &recipe.steps {
                match step {
                    Step::Command {
                        id, timeout_secs, ..
                    } => {
                        assert!(
                            timeout_secs.is_some(),
                            "Command '{id}' has no timeout in a recipe"
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    fn gen_config_of_step(step: &Step) -> &serde_json::Value {
        match step {
            Step::Generate {
                generator_config, ..
            } => generator_config,
            other => panic!("expected Generate, got {:?}", other.id()),
        }
    }

    fn error_mode_of(step: &Step) -> ErrorMode {
        match step {
            Step::Command { on_error, .. } => on_error.clone(),
            other => panic!("expected Command, got {:?}", other.id()),
        }
    }

    fn step_file_policy_of(step: &Step) -> FilePolicy {
        step.file_policy().expect("file step must have a policy")
    }

    #[tokio::test]
    async fn engine_failed_step_reports_stdout_and_stderr_tails() {
        let engine = DefaultRecipeEngine::new();
        let dir =
            std::env::temp_dir().join(format!("stackpilot_engine_err_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (command, args) = if cfg!(target_os = "windows") {
            (
                "cmd".to_string(),
                vec![
                    "/d".into(),
                    "/c".into(),
                    "echo out-line && echo err-line 1>&2 && exit 1".into(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec![
                    "-c".into(),
                    "echo out-line; echo err-line 1>&2; exit 1".into(),
                ],
            )
        };
        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::Command {
                id: "failing".into(),
                label: "Failing step".into(),
                description: String::new(),
                command,
                args,
                working_dir: None,
                env: None,
                timeout_secs: Some(10),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        match &result.step_results[0].status {
            StepStatus::Failed { error } => {
                assert!(error.contains("out-line"), "stdout tail missing: {error}");
                assert!(error.contains("err-line"), "stderr tail missing: {error}");
                assert!(error.contains("command: "), "command line missing: {error}");
                assert!(error.contains("working directory"), "{error}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_create_root_step_with_dot_path_succeeds() {
        // Регрессия: первый шаг каждого рецепта — CreateDirectory path="."
        // («Create project root»). paths::resolve_in_root(".") раньше
        // возвращал None, и шаг падал с «escapes the project root» на
        // любой генерации — проект даже не начинал создаваться.
        let engine = DefaultRecipeEngine::new();
        let dir =
            std::env::temp_dir().join(format!("stackpilot_create_root_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let plan = ExecutionPlan {
            recipe: Recipe {
                id: "test".into(),
                name: "Test recipe".into(),
                description: String::new(),
                tags: vec![],
                steps: vec![],
                dependencies: vec![],
            },
            context: WizardContext::default(),
            project_path: dir.clone(),
            steps: vec![Step::CreateDirectory {
                id: "create_root".into(),
                label: "Create project root".into(),
                description: String::new(),
                path: ".".into(),
                condition: None,
                on_error: ErrorMode::Abort,
            }],
            dependencies: vec![],
            layout_summary: LayoutSummary {
                class: "frontend-only".to_string(),
                generated_directories: vec![],
                root_owner: None,
                framework_placement: vec![],
            },
        };
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let result = engine.execute(plan, tx).await;
        assert!(
            matches!(result.step_results[0].status, StepStatus::Success { .. }),
            "CreateDirectory path='.' обязан успешно создать корень: {:?}",
            result.step_results[0].status
        );
        assert!(dir.is_dir(), "корень проекта должен существовать");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
