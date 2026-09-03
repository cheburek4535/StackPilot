use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::modules::project_creator::engine::network;
use crate::modules::project_creator::engine::paths;
use crate::modules::project_creator::engine::process::{
    command_display, local_time, CommandRunner, ExecutionEventSink, InteractiveRules,
    ProcessRunner, ProcessSpec, StdinMode,
};
use crate::modules::project_creator::engine::ExecutionPlan;
use crate::modules::project_creator::generators::GeneratorRegistry;
use crate::modules::project_creator::models::*;

/// StepExecutor — выполняет отдельные шаги плана.
pub struct StepExecutor {
    /// Встроенные генераторы для шагов Step::Generate
    /// (spring-boot, fs-cleanup, cli).
    pub generators: Arc<GeneratorRegistry>,
    /// Шина запуска CLI-команд (реальный ProcessRunner или мок в тестах).
    pub command_runner: Arc<dyn CommandRunner>,
}

impl StepExecutor {
    pub fn new() -> Self {
        Self::with_command_runner(Arc::new(ProcessRunner))
    }

    pub fn with_generators(generators: Arc<GeneratorRegistry>) -> Self {
        Self {
            generators,
            command_runner: Arc::new(ProcessRunner),
        }
    }

    /// Экзекутор с переопределённой шиной процессов — единственная точка
    /// мока CLI в тестах исполнения (MockCommandRunner).
    pub fn with_command_runner(command_runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            generators: Arc::new(GeneratorRegistry::with_defaults()),
            command_runner,
        }
    }

    /// Событийный приёмник шага: маршрутизирует вывод процесса в канал
    /// ExecutionEvent (общий механизм для Step::Command и Step::Generate).
    fn sink_for_step(
        tx: &mpsc::Sender<ExecutionEvent>,
        step: &Step,
        index: usize,
        total_steps: usize,
    ) -> ExecutionEventSink {
        ExecutionEventSink::new(
            tx.clone(),
            step_id(step),
            index,
            total_steps,
            step_label(step),
            step_description(step),
        )
    }

    pub async fn run_command(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {
        let (command, raw_args, working_dir, env, timeout_secs, interactive) = match step {
            Step::Command {
                command,
                args,
                working_dir,
                env,
                timeout_secs,
                interactive,
                ..
            } => (command, args, working_dir, env, timeout_secs, interactive),
            _ => {
                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status: StepStatus::Failed {
                        error: "Expected Command step".into(),
                    },
                    duration_ms: 0,
                }
            }
        };

        // Аргументы рецепта передаются без скрытых модификаций. В частности,
        // рецепт сам отвечает за наличие `npx --yes`: глобальная инъекция меняет
        // позицию/семантику аргументов отдельных CLI.
        let args: Vec<String> = raw_args.clone();

        // Рабочая директория — строго внутри корня проекта: абсолютные пути
        // рецептов (project_path, project_path + "/segment") пропускаются,
        // выход за корень (../, чужие абсолютные пути) — ошибка шага.
        let full_working_dir = if let Some(dir) = working_dir {
            match paths::resolve_working_dir(&plan.project_path, dir) {
                Ok(dir) => dir,
                Err(e) => {
                    return failed_result(
                        step,
                        std::time::Instant::now(),
                        format!("Invalid working directory: {e}"),
                    )
                }
            }
        } else {
            plan.project_path.clone()
        };

        // Piped-режим сохраняет историческое поведение Step::Command:
        // детекция interactive-триггеров, fallback-ответы и idle-Enter при
        // молчании процесса. Сам запуск делегируется общему ProcessRunner.
        let spec = ProcessSpec {
            command: command.clone(),
            args,
            working_dir: Some(full_working_dir.clone()),
            env: env.clone(),
            timeout: timeout_secs.map(Duration::from_secs),
            stdin: StdinMode::Piped(InteractiveRules {
                entries: interactive
                    .iter()
                    .map(|e| (e.trigger.clone(), e.response_type.clone()))
                    .collect(),
            }),
            ci_mode: true,
        };

        let sink = Self::sink_for_step(tx, step, index, plan.step_count());
        let start = std::time::Instant::now();
        let command_text = command_display(command, &spec.args);

        // Сетевые действия (скачивание пакетов/провайдеров с реестра) могут
        // «внезапно» провалиться из-за сети, а не кода: таким командам даётся
        // несколько попыток с паузой. Повтор идёт ТОЛЬКО при сетевом маркере в
        // диагностике — битая конфигурация (ENOENT, синтаксис манифеста и т.п.)
        // не ждёт и падает сразу. Устойчивый сетевой сбой получает развёрнутый
        // совет и НЕ останавливает генерацию (см. step_should_abort в engine).
        let max_attempts = if network::is_network_command(command, &spec.args) {
            network::NETWORK_MAX_ATTEMPTS
        } else {
            1
        };
        let mut attempt = 0u32;
        let final_error: String = 'retry: loop {
            attempt += 1;
            match self.command_runner.run(spec.clone(), Some(&sink)).await {
                Ok(output) => {
                    return StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Success {
                            message: format!("Command '{}' completed successfully", command_text),
                        },
                        duration_ms: output.duration_ms,
                    }
                }
                Err(error) => {
                    let formatted = error.format_command_error();
                    let is_network_failure =
                        max_attempts > 1 && network::has_network_failure_markers(&formatted);
                    if attempt >= max_attempts || !is_network_failure {
                        if is_network_failure {
                            break 'retry format!(
                                "{formatted}\n\n{}\n",
                                network::network_failure_hint(command, &spec.args)
                            );
                        }
                        break 'retry formatted;
                    }
                    let _ = sink
                        .emit_stdout(&format!(
                            "\n[{}] '{}' hit a network error (attempt {}/{}), retrying after {} ms...\n",
                            step_id(step),
                            command_text,
                            attempt,
                            max_attempts,
                            network::NETWORK_RETRY_BACKOFF.as_millis()
                        ))
                        .await;
                    tokio::time::sleep(network::NETWORK_RETRY_BACKOFF).await;
                }
            }
        };
        failed_result(step, start, final_error)
    }

    /// Выполнить шаг Generate: диспетчеризация во встроенные генераторы
    /// движка (spring-boot, fs-cleanup, cli). Ошибка генератора (например,
    /// «Spring Initializr error: HTTP 400 ...») становится Failed-статусом
    /// шага и останавливает пайплайн при on_error=Abort.
    /// Политика SkipIfExists: CLI не запускается, когда все expected_outputs
    /// уже существуют в каталоге назначения (повторный запуск рецепта не
    /// перезатирает готовый каркас) — шаг успешен без действий.
    pub async fn run_generate(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {
        let (generator_id, generator_config, policy) = match step {
            Step::Generate {
                generator_id,
                generator_config,
                policy,
                ..
            } => (generator_id, generator_config, policy),
            _ => {
                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status: StepStatus::Failed {
                        error: "Expected Generate step".into(),
                    },
                    duration_ms: 0,
                }
            }
        };

        let start = std::time::Instant::now();

        if *policy == Some(FilePolicy::SkipIfExists) && generator_id == "scaffold" {
            if let Some(outputs) = generator_config
                .get("expected_outputs")
                .and_then(|v| v.as_array())
            {
                let outputs: Vec<String> = outputs
                    .iter()
                    .filter_map(|o| o.as_str().map(String::from))
                    .collect();
                if !outputs.is_empty()
                    && all_expected_outputs_exist(plan, generator_config, &outputs)
                {
                    let message = format!(
                        "Scaffold '{}' already exists — skipped by policy (skip_if_exists)",
                        outputs.join(", ")
                    );
                    tx.send(ExecutionEvent {
                        event_type: ExecutionEventType::StepProgress {
                            stdout: message.clone(),
                            stderr: String::new(),
                        },
                        step_id: step_id(step),
                        step_index: index,
                        total_steps: plan.step_count(),
                        step_name: step_label(step),
                        step_description: step_description(step),
                        timestamp: local_time(),
                    })
                    .await
                    .ok();
                    return StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Success { message },
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
            }
        }

        // Генераторам отдаётся событийный приёмник шага: их CLI-вывод
        // стримится в UI тем же механизмом, что и вывод Step::Command.
        let sink = Self::sink_for_step(tx, step, index, plan.step_count());

        // Scaffold/spring-boot качают каркас из сети — им, как и сетевым
        // Step::Command, даётся несколько попыток при сетевом маркере сбоя;
        // устойчивый сетевой сбой получает совет и не останавливает генерацию.
        let is_network_generator = network::is_network_generator(generator_id, generator_config);
        let max_attempts = if is_network_generator {
            network::NETWORK_MAX_ATTEMPTS
        } else {
            1
        };
        let (hint_command, hint_args) = network::generator_command_and_args(generator_config);
        let mut attempt = 0u32;
        let outcome = 'retry: loop {
            attempt += 1;
            let result = match self.generators.get(generator_id) {
                Some(generator) => {
                    generator
                        .generate_with_sink(
                            &plan.context,
                            &plan.project_path,
                            generator_config,
                            Some(&sink),
                        )
                        .await
                }
                None => Err(format!("Unknown generator '{}'", generator_id)),
            };
            match result {
                Ok(report) => break 'retry Ok(report),
                Err(error) => {
                    let is_network_failure =
                        is_network_generator && network::has_network_failure_markers(&error);
                    if attempt >= max_attempts || !is_network_failure {
                        if is_network_failure {
                            break 'retry Err(format!(
                                "{error}\n\n{}\n",
                                network::network_failure_hint(&hint_command, &hint_args)
                            ));
                        }
                        break 'retry Err(error);
                    }
                    let _ = sink
                        .emit_stdout(&format!(
                            "\n[{}] generator '{generator_id}' hit a network error (attempt {attempt}/{max_attempts}), retrying after {} ms...\n",
                            step_id(step),
                            network::NETWORK_RETRY_BACKOFF.as_millis()
                        ))
                        .await;
                    tokio::time::sleep(network::NETWORK_RETRY_BACKOFF).await;
                }
            }
        };

        let (status, progress_msg) = match outcome {
            Ok(report) => (
                StepStatus::Success {
                    message: report.message.clone(),
                },
                format!("{}: {}", generator_id, report.message),
            ),
            Err(error) => (
                StepStatus::Failed {
                    error: error.clone(),
                },
                format!("{}: {}", generator_id, error),
            ),
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        tx.send(ExecutionEvent {
            event_type: ExecutionEventType::StepProgress {
                stdout: progress_msg,
                stderr: String::new(),
            },
            step_id: step_id(step),
            step_index: index,
            total_steps: plan.step_count(),
            step_name: step_label(step),
            step_description: step_description(step),
            timestamp: local_time(),
        })
        .await
        .ok();

        StepResult {
            step_id: step_id(step),
            label: step_label(step),
            status,
            duration_ms,
        }
    }

    /// Записать файл. Путь строго внутри корня проекта; поведение при
    /// существующем файле — по политике идемпотентности (FilePolicy):
    ///   - Overwrite: всегда перезаписать;
    ///   - CreateOnly/SkipIfExists: существующий файл не трогается (skip);
    ///   - MergeJson: глубокое JSON-слияние с существующим содержимым
    ///     (существующие ключи сохраняются), не-JSON — ошибка шага;
    ///   - FailOnMismatch: идентичный файл — идемпотентный no-op (Success),
    ///     отличие — ошибка (молчаливый перезапрос невозможен).
    pub async fn write_file(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {
        let final_status: StepStatus;
        let duration: u64;

        if let Step::WriteFile {
            path,
            content,
            policy,
            overwrite,
            ..
        } = step
        {
            let start = std::time::Instant::now();

            // Безопасный путь строго внутри корня проекта: ../ и абсолютные
            // пути — ошибка шага, а не запись мимо проекта.
            let Some(project_path) = paths::resolve_in_root(&plan.project_path, path) else {
                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status: StepStatus::Failed {
                        error: format!("WriteFile path '{}' escapes the project root", path),
                    },
                    duration_ms: 0,
                };
            };

            if let Some(parent) = project_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Failed {
                            error: format!("Failed create all dirs to project root: {e}"),
                        },
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                }
            }

            let effective = policy.unwrap_or(if *overwrite {
                FilePolicy::Overwrite
            } else {
                FilePolicy::SkipIfExists
            });

            if project_path.exists() {
                final_status = match effective {
                    FilePolicy::CreateOnly | FilePolicy::SkipIfExists => StepStatus::Skipped {
                        reason: format!("file already exists and overwrite policy is {:?}", effective),
                    },
                    FilePolicy::Overwrite => {
                        Self::write_out(&project_path, content, path, tx, step, index, plan).await
                    }
                    FilePolicy::MergeJson => match merge_json_write(&project_path, content, path) {
                        Ok(status) => status,
                        Err(e) => StepStatus::Failed { error: e },
                    },
                    FilePolicy::FailOnMismatch => {
                        match std::fs::read_to_string(&project_path) {
                            Ok(existing) if existing == *content => StepStatus::Success {
                                message: format!("File {} is unchanged — keeping existing content", path),
                            },
                            Ok(_) => StepStatus::Failed {
                                error: format!("File {} already exists with different content and policy is fail_on_mismatch", path),
                            },
                            Err(e) => StepStatus::Failed {
                                error: format!("Failed read existing file {}: {}", path, e),
                            },
                        }
                    }
                };
            } else {
                final_status =
                    Self::write_out(&project_path, content, path, tx, step, index, plan).await;
            }
            duration = start.elapsed().as_millis() as u64;
        } else {
            final_status = StepStatus::Failed {
                error: ("Incorrect step type, expected WriteFile".into()),
            };
            duration = 0;
        }

        StepResult {
            step_id: step_id(step),
            label: step_label(step),
            status: final_status,
            duration_ms: duration,
        }
    }

    /// Записать файл и застримить StepProgress-событие.
    async fn write_out(
        path: &std::path::Path,
        content: &str,
        display_path: &str,
        tx: &mpsc::Sender<ExecutionEvent>,
        step: &Step,
        index: usize,
        plan: &ExecutionPlan,
    ) -> StepStatus {
        match std::fs::write(path, content) {
            Ok(_) => {
                tx.send(ExecutionEvent {
                    event_type: ExecutionEventType::StepProgress {
                        stdout: format!("Wrote {display_path}"),
                        stderr: String::new(),
                    },
                    step_id: step_id(step),
                    step_index: index,
                    total_steps: plan.step_count(),
                    step_name: step_label(step),
                    step_description: step_description(step),
                    timestamp: local_time(),
                })
                .await
                .ok();
                StepStatus::Success {
                    message: "Successfully written content".into(),
                }
            }
            Err(err) => StepStatus::Failed {
                error: format!(" Failed write file {err}"),
            },
        }
    }

    // Отрендерить шаблон и записать файл
    // pub async fn render_template(
    //     &self,
    //     step: &Step,
    //     plan: &ExecutionPlan,
    //     tx: &mpsc::Sender<ExecutionEvent>,
    //     index: usize,
    //     _engine: &TemplateEngine,
    // ) -> StepResult {
    //     // TZ Task 4: реализовать рендеринг шаблона
    //     // - взять template строку из шага
    //     // - подставить context переменные через TemplateEngine
    //     // - записать результат в path
    //     //
    //     // Пока заглушка — просто пишет template как есть
    //     if let Step::RenderTemplate { path, template, context, .. } = step {
    //         let project_path = &plan.project_path;
    //         let full_path = project_path.join(path);

    //         // Создать родительскую директорию
    //         if let Some(parent) = full_path.parent() {
    //             let _ = std::fs::create_dir_all(parent);
    //         }

    //         // Пока пишем сырой template (TZ: подставить context)
    //         let rendered = template.clone(); // TBD: engine.render(template, context)

    //         match std::fs::write(&full_path, &rendered) {
    //             Ok(_) => {
    //                 let _ = tx.send(ExecutionEvent {
    //                     event_type: ExecutionEventType::StepProgress {
    //                         stdout: format!("Wrote {}", full_path.display()),
    //                         stderr: String::new(),
    //                     },
    //                     step_id: step_id(step),
    //                     step_index: index,
    //                     total_steps: plan.step_count(),
    //                     step_name: step_label(step),
    //                     step_description: step_description(step),
    //                     timestamp: String::new(),
    //                 }).await;

    //                 StepResult {
    //                     step_id: step_id(step),
    //                     label: step_label(step),
    //                     status: StepStatus::Success {
    //                         message: format!("Wrote {}", full_path.display()),
    //                     },
    //                     duration_ms: 0,
    //                 }
    //             }
    //             Err(e) => StepResult {
    //                 step_id: step_id(step),
    //                 label: step_label(step),
    //                 status: StepStatus::Failed {
    //                     error: format!("Failed to write {}: {}", full_path.display(), e),
    //                 },
    //                 duration_ms: 0,
    //             },
    //         }
    //     } else {
    //         StepResult {
    //             step_id: step_id(step),
    //             label: step_label(step),
    //             status: StepStatus::Failed {
    //                 error: "Expected RenderTemplate step".into(),
    //             },
    //             duration_ms: 0,
    //         }
    //     }
    // }

    /// Создать директорию
    pub async fn create_directory(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {
        if let Step::CreateDirectory { path, .. } = step {
            let start = std::time::Instant::now();

            let Some(full_path) = paths::resolve_in_root(&plan.project_path, path) else {
                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status: StepStatus::Failed {
                        error: format!("CreateDirectory path '{}' escapes the project root", path),
                    },
                    duration_ms: 0,
                };
            };
            match std::fs::create_dir_all(&full_path) {
                Ok(_) => {
                    tx.send(ExecutionEvent {
                        event_type: ExecutionEventType::StepProgress {
                            stdout: format!("Created {}", full_path.display()),
                            stderr: String::new(),
                        },
                        step_id: step_id(step),
                        step_index: index,
                        total_steps: plan.step_count(),
                        step_name: step_label(step),
                        step_description: step_description(step),
                        timestamp: local_time(),
                    })
                    .await
                    .ok();

                    StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Success {
                            message: format!("Created {}", full_path.display()),
                        },
                        duration_ms: start.elapsed().as_millis() as u64,
                    }
                }
                Err(e) => StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status: StepStatus::Failed {
                        error: format!("Failed to create directory {}: {}", full_path.display(), e),
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            }
        } else {
            StepResult {
                step_id: step_id(step),
                label: step_label(step),
                status: StepStatus::Failed {
                    error: "Expected CreateDirectory step".into(),
                },
                duration_ms: 0,
            }
        }
    }
}

// Helpers
fn step_id(step: &Step) -> String {
    step.id()
}
fn step_label(step: &Step) -> String {
    step.label()
}
fn step_description(step: &Step) -> String {
    step.description()
}

/// Глубокое JSON-слияние при политике MergeJson: ключи существующего файла
/// сохраняются, недостающие берутся из записываемого содержимого; вложенные
/// объекты сливаются рекурсивно. Существующий не-JSON файл — ошибка шага
/// (молчаливый деструктивный перезапрос невозможен).
fn merge_json_write(
    path: &std::path::Path,
    content: &str,
    display_path: &str,
) -> Result<StepStatus, String> {
    let existing_text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed read existing file {}: {}", path.display(), e))?;
    let existing: serde_json::Value = serde_json::from_str(&existing_text).map_err(|e| {
        format!(
            "MergeJson: existing file {} is not valid JSON: {}",
            display_path, e
        )
    })?;
    let incoming: serde_json::Value = serde_json::from_str(content).map_err(|e| {
        format!(
            "MergeJson: content for {} is not valid JSON: {}",
            display_path, e
        )
    })?;
    let merged = merge_json(&existing, &incoming);
    let text = serde_json::to_string_pretty(&merged).map_err(|e| {
        format!(
            "MergeJson: failed to serialize merged JSON for {}: {}",
            display_path, e
        )
    })?;
    std::fs::write(path, format!("{}\n", text))
        .map_err(|e| format!("Failed write file {}: {}", path.display(), e))?;
    Ok(StepStatus::Success {
        message: format!("Merged JSON into {}", display_path),
    })
}

/// Все expected_outputs scaffold-шага уже существуют в каталоге назначения
/// (пути относительно target_dir, как их валидирует ScaffoldGenerator).
fn all_expected_outputs_exist(
    plan: &ExecutionPlan,
    generator_config: &serde_json::Value,
    outputs: &[String],
) -> bool {
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

fn failed_result(step: &Step, start: std::time::Instant, error: String) -> StepResult {
    StepResult {
        step_id: step_id(step),
        label: step_label(step),
        status: StepStatus::Failed { error },
        duration_ms: start.elapsed().as_millis() as u64,
    }
}
