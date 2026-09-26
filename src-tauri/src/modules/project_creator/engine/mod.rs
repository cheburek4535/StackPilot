pub mod content;
pub mod executor;
mod network;
pub mod paths;
pub mod preflight;
pub mod process;
mod readme;
pub mod template;
pub mod providers;
pub mod preview;
pub mod composer;
pub mod layout;
pub mod helpers;

pub use composer::*;
pub use helpers::*;
pub use layout::*;
pub use preview::*;

#[cfg(test)]
mod tests;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::modules::project_creator::generators::GeneratorRegistry;
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
        cancel_flag: Arc<std::sync::atomic::AtomicBool>,
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

    /// Движок с переопределённым экзекутором (мок-шина процессов,
    /// мок-генераторы) — для тестов исполнения.
#[allow(dead_code)]
    pub fn with_executor(executor: Arc<executor::StepExecutor>) -> Self {
        Self { executor }
    }
}

/// Финализационные шаги, которые бессмысленны/вредны на частично
/// сломанном проекте: git add/commit зафиксируют нерабочий результат,
/// README перезапишет каркасные файлы в проекте, который не собрался.
/// При ЛЮБОМ Failed (даже Skip-режима) они пропускаются с точной
/// причиной — см. execute().
fn is_finalize_step(id: &str) -> bool {
    matches!(id, "git_add" | "git_commit" | "readme")
}

/// Должен ли Failed-результат шага прервать пайплайн (overall → Aborted).
///
/// Правило on_error (Abort) сохраняется для всех ошибок КРОМЕ сетевых:
/// устойчивый сбой именно сети/реестра (registry.terraform.io недоступен,
/// npm/pip не могут скачать пакеты и т.п.) — это проблема окружения, а не
/// проекта. Для него движок уже сделал повторы (executor) и добавил совет,
/// и генерация НЕ должна обрываться из-за недоступного реестра:
/// файлы созданы, команду пользователь повторит вручную. Об этом говорит
/// сетевой маркер в тексте ошибки + команда, классифицированная как сетевая.
fn step_should_abort(step: &Step, status: &StepStatus) -> bool {
    let network_error = match status {
        StepStatus::Failed { error } => network::has_network_failure_markers(error),
        _ => false,
    };
    match step {
        Step::Command {
            command,
            args,
            on_error: ErrorMode::Abort,
            ..
        } if network_error && network::is_network_command(command, args) => false,
        Step::Generate {
            generator_id,
            generator_config,
            on_error: ErrorMode::Abort,
            ..
        } if network_error && network::is_network_generator(generator_id, generator_config) => {
            false
        }
        Step::Command {
            on_error: ErrorMode::Abort,
            ..
        }
        | Step::WriteFile {
            on_error: ErrorMode::Abort,
            ..
        }
        | Step::CreateDirectory {
            on_error: ErrorMode::Abort,
            ..
        }
        | Step::Generate {
            on_error: ErrorMode::Abort,
            ..
        }
        | Step::Parallel {
            on_error: ErrorMode::Abort,
            ..
        } => true,
        _ => false,
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
        // Канонический контекст: нормализация + ПОЛНАЯ валидация ДО построения
        // плана. Первый шаг плана создаёт каталог проекта — невалидный стек
        // (неизвестные id, конфликты, дублирующиеся файлы) обязан отсекаться
        // здесь, а не на полпути выполнения. Контекст в каноническом виде
        // пронизывает compose_recipe и ExecutionPlan.
        let mut context = context.clone();
        if let Some(err) = super::validate::first_error(&super::validate::validate_context(
            wizard_tree(),
            &mut context,
            super::validate::current_os(),
        )) {
            return Err(err);
        }
        // Каноническая раскладка проекта вычисляется ОДИН раз и пронизывает
        // compose_recipe (каталоги фреймворков/языков), предпросмотр (LayoutSummary)
        // и шаблонизацию (README, Docker). Никаких повторных эвристик.
        let layout = ProjectLayout::compute(&context);
        let recipe = compose_recipe(&layout, &context, folder_name, project_path)?;
        // context.project_name (если задан) будет использован внутри compose_recipe
        let steps = flatten_and_filter(&recipe, &context, project_path);
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
            layout_summary: layout.to_summary(&context),
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
                            file_policy.expect(
                                "match arm only matches Some(SkipIfExists | CreateOnly)"
                            )
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
        cancel_flag: Arc<std::sync::atomic::AtomicBool>,
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
        // Любой Failed (включая Skip-режим): финализационные шаги
        // (git_add/git_commit/readme) после него не выполняются — коммитить
        // и перезаписывать README на сломанном проекте вредно.
        let mut saw_failure = false;

        for (i, step) in plan.steps.iter().enumerate() {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                aborted = true;
            }
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
                let reason = if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    "Operation cancelled by user".into()
                } else {
                    "Previous step failed, aborting".into()
                };
                results.push(StepResult {
                    step_id: step.id(),
                    label: step.label(),
                    status: StepStatus::Skipped { reason },
                    duration_ms: 0,
                });
                continue;
            }
            if saw_failure && is_finalize_step(&step.id()) {
                results.push(StepResult {
                    step_id: step.id(),
                    label: step.label(),
                    status: StepStatus::Skipped {
                        reason: "Previous step failed — finalize step (git add/commit/README) is skipped to avoid committing a broken project".into(),
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

            let abort = is_failure && step_should_abort(&step, &result.status);

            results.push(StepResult {
                duration_ms: duration,
                ..result
            });

            if is_failure {
                saw_failure = true;
            }
            if abort {
                aborted = true;
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

        let overall = if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            OverallStatus::Aborted {
                last_step: results.last().map(|r| r.step_id.clone()),
                reason: "Operation cancelled by user".into(),
            }
        } else if failed.is_empty() {
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
