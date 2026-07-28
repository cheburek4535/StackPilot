
use tokio::sync::mpsc;
use chrono::Local;
use crate::modules::project_creator::engine::ExecutionPlan;
use crate::modules::project_creator::models::*;

/// StepExecutor — выполняет отдельные шаги плана.
pub struct StepExecutor;

impl StepExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn run_command(
    &self,
    step: &Step,
    plan: &ExecutionPlan,
    tx: &mpsc::Sender<ExecutionEvent>,
    index: usize,
) -> StepResult {
    use std::env::consts::OS;
    use std::process::Stdio;

    let (command, args, working_dir, env, timeout_secs) = match step {
        Step::Command {
            command,
            args,
            working_dir,
            env,
            timeout_secs,
            ..
        } => (command, args, working_dir, env, timeout_secs),
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

    // Кроссплатформенный запуск через shell
    let mut cmd = match OS {
        "windows" => {
            // Проверяем, нужна ли PowerShell
            let is_powershell = command.starts_with("powershell")
                || command.starts_with("pwsh")
                || command.contains("Get-")
                || command.contains("Set-")
                || command.contains("Invoke-")
                || command.contains("New-");

            if is_powershell {
                let mut ps_cmd = tokio::process::Command::new("powershell");
                ps_cmd.arg("-Command");
                ps_cmd.arg(command);
                if !args.is_empty() {
                    ps_cmd.args(args);
                }
                ps_cmd
            } else {
                let mut win_cmd = tokio::process::Command::new("cmd");
                win_cmd.arg("/C");
                win_cmd.arg(command);
                if !args.is_empty() {
                    win_cmd.args(args);
                }
                win_cmd
            }
        }
        _ => {
            // Linux, macOS и остальные Unix-подобные
            let mut unix_cmd = tokio::process::Command::new("sh");
            unix_cmd.arg("-c");
            unix_cmd.arg(command);
            if !args.is_empty() {
                unix_cmd.args(args);
            }
            unix_cmd
        }
    };

    let full_working_dir = if let Some(dir) = working_dir {
        plan.project_path.join(dir)
    } else {
        plan.project_path.clone()
    };
    cmd.current_dir(&full_working_dir);

    if let Some(env_map) = env {
        cmd.envs(env_map);
    }

    // Используем tokio::process::Stdio для кроссплатформенной совместимости
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let start = std::time::Instant::now();
    let total_steps = plan.step_count();

    // Отправляем Started
    tx.send(ExecutionEvent {
        event_type: ExecutionEventType::StepStarted,
        step_id: step_id(step),
        step_index: index,
        total_steps,
        step_name: step_label(step),
        step_description: step_description(step),
        timestamp: local_time(),
    })
    .await
    .ok();

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let error_msg = format!("Failed to spawn command '{}': {}", command, e);
            let status = StepStatus::Failed { error: error_msg };
            let duration_ms = start.elapsed().as_millis() as u64;

            tx.send(ExecutionEvent {
                event_type: ExecutionEventType::StepCompleted {
                    status: status.clone(),
                    duration_ms,
                },
                step_id: step_id(step),
                step_index: index,
                total_steps,
                step_name: step_label(step),
                step_description: step_description(step),
                timestamp: local_time(),
            })
            .await
            .ok();

            return StepResult {
                step_id: step_id(step),
                label: step_label(step),
                status,
                duration_ms,
            };
        }
    };

    let stdout = child.stdout.take().expect("stdout should be piped");
    let stderr = child.stderr.take().expect("stderr should be piped");

    let tx_stdout = tx.clone();
    let tx_stderr = tx.clone();

    // Для stdout
    let step_id_out = step_id(step);
    let step_name_out = step_label(step);
    let step_desc_out = step_description(step);

    let stdout_handle = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            tx_stdout
                .send(ExecutionEvent {
                    event_type: ExecutionEventType::StepProgress {
                        stdout: line,
                        stderr: String::new(),
                    },
                    step_id: step_id_out.clone(),
                    step_index: index,
                    total_steps,
                    step_name: step_name_out.clone(),
                    step_description: step_desc_out.clone(),
                    timestamp: local_time(),
                })
                .await
                .ok();
        }
    });

    // Для stderr
    let step_id_err = step_id(step);
    let step_name_err = step_label(step);
    let step_desc_err = step_description(step);

    let stderr_handle = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            tx_stderr
                .send(ExecutionEvent {
                    event_type: ExecutionEventType::StepProgress {
                        stdout: String::new(),
                        stderr: line,
                    },
                    step_id: step_id_err.clone(),
                    step_index: index,
                    total_steps,
                    step_name: step_name_err.clone(),
                    step_description: step_desc_err.clone(),
                    timestamp: local_time(),
                })
                .await
                .ok();
        }
    });

    // Ожидаем завершения процесса
    let exit_status_result = if let Some(timeout_secs) = timeout_secs {
        let duration = std::time::Duration::from_secs(*timeout_secs);

        match tokio::time::timeout(duration, child.wait()).await {
            Ok(Ok(status)) => Some(status),
            Ok(Err(e)) => {
                let _ = child.kill().await;

                let error_msg = format!("Process wait error: {}", e);
                let status = StepStatus::Failed { error: error_msg };
                let duration_ms = start.elapsed().as_millis() as u64;

                tx.send(ExecutionEvent {
                    event_type: ExecutionEventType::StepCompleted {
                        status: status.clone(),
                        duration_ms,
                    },
                    step_id: step_id(step),
                    step_index: index,
                    total_steps,
                    step_name: step_label(step),
                    step_description: step_description(step),
                    timestamp: local_time(),
                })
                .await
                .ok();

                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status,
                    duration_ms,
                };
            }
            Err(_elapsed) => {
                let _ = child.kill().await;
                None
            }
        }
    } else {
        match child.wait().await {
            Ok(status) => Some(status),
            Err(e) => {
                let _ = child.kill().await;
                let _ = stdout_handle.await;
                let _ = stderr_handle.await;

                let error_msg = format!("Process wait error: {}", e);
                let status = StepStatus::Failed { error: error_msg };
                let duration_ms = start.elapsed().as_millis() as u64;

                tx.send(ExecutionEvent {
                    event_type: ExecutionEventType::StepCompleted {
                        status: status.clone(),
                        duration_ms,
                    },
                    step_id: step_id(step),
                    step_index: index,
                    total_steps,
                    step_name: step_label(step),
                    step_description: step_description(step),
                    timestamp: local_time(),
                })
                .await
                .ok();

                return StepResult {
                    step_id: step_id(step),
                    label: step_label(step),
                    status,
                    duration_ms,
                };
            }
        }
    };

    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    let final_status = if let Some(exit_status) = exit_status_result {
        if exit_status.success() {
            StepStatus::Success {
                message: format!("Command '{}' completed successfully", command),
            }
        } else {
            StepStatus::Failed {
                error: format!(
                    "Command '{}' failed with exit code: {}",
                    command,
                    exit_status.code().unwrap_or(-1)
                ),
            }
        }
    } else {
        StepStatus::Failed {
            error: format!(
                "Command '{}' timed out after {} seconds",
                command,
                timeout_secs.unwrap_or(0)
            ),
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    tx.send(ExecutionEvent {
        event_type: ExecutionEventType::StepCompleted {
            status: final_status.clone(),
            duration_ms,
        },
        step_id: step_id(step),
        step_index: index,
        total_steps,
        step_name: step_label(step),
        step_description: step_description(step),
        timestamp: local_time(),
    })
    .await
    .ok();

    StepResult {
        step_id: step_id(step),
        label: step_label(step),
        status: final_status,
        duration_ms,
    }
}

    /// Записать файл
    pub async fn write_file(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {

        let final_status: StepStatus;
        let duration: u64;

        if let Step::WriteFile {path, content, overwrite, ..} = step {
            
            tx.send(ExecutionEvent { 
                event_type: ExecutionEventType::StepStarted,
                step_id: step_id(step), 
                step_index: index, 
                total_steps: plan.step_count(), 
                step_name: step_label(step),
                step_description: step_description(step), 
                timestamp: local_time() 
            }).await.ok();

            let project_path = &plan.project_path.join(path);

            let start = std::time::Instant::now();

            if let Some(parent) = project_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    tx.send(ExecutionEvent { 
                    event_type: ExecutionEventType::StepCompleted { status: StepStatus::Failed { error: format!("Failed create all dirs to project root: {e}")}, duration_ms: start.elapsed().as_millis() as u64 },
                    step_id: step_id(step), 
                    step_index: index, 
                    total_steps: plan.step_count(), 
                    step_name: step_label(step),
                    step_description: step_description(step), 
                    timestamp: local_time() 
                }).await.ok();

                    return StepResult {
                        step_id: step_id(step), 
                        label: step_label(step), 
                        status: StepStatus::Failed { error: format!("Failed create all dirs to project root: {e}")}, duration_ms: start.elapsed().as_millis() as u64};
                }        
            }
            if !overwrite && project_path.exists() {
                final_status = StepStatus::Skipped { reason: "file already exists and overwrite == false".into() };
            } else {
                let success = std::fs::write(project_path, content);
                match success {
                    Ok(_) => {
                        final_status = StepStatus::Success { message: "Successfully written content".into() };

                        tx.send(ExecutionEvent { 
                            event_type: ExecutionEventType::StepProgress { stdout: format!("Wrote {path}"), stderr: String::new() },
                            step_id: step_id(step), 
                            step_index: index, 
                            total_steps: plan.step_count(), 
                            step_name: step_label(step),
                            step_description: step_description(step), 
                            timestamp: local_time() 
                        }).await.ok();

                    }
                    Err(err) => {
                        final_status = StepStatus::Failed {error: format!(" Failed write file {err}") };
                        }
                    }
            }      
            duration = start.elapsed().as_millis() as u64;    
        } else {
            final_status = StepStatus::Failed { error: ("Incorrect step type, expected WriteFile".into()) };
            duration = 0;
        }

        tx.send(ExecutionEvent { 
                event_type: ExecutionEventType::StepCompleted { status: final_status.clone(), duration_ms: duration },
                step_id: step_id(step), 
                step_index: index, 
                total_steps: plan.step_count(), 
                step_name: step_label(step),
                step_description: step_description(step), 
                timestamp: local_time() 
        }).await.ok();

        StepResult {
            step_id: step_id(step),
            label: step_label(step),
            status: final_status,
            duration_ms: duration
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
            tx.send(ExecutionEvent { 
                event_type: ExecutionEventType::StepStarted,
                step_id: step_id(step), 
                step_index: index, 
                total_steps: plan.step_count(), 
                step_name: step_label(step),
                step_description: step_description(step), 
                timestamp: local_time() 
            }).await.ok();

            let start = std::time::Instant::now();

            let full_path = plan.project_path.join(path);
            match std::fs::create_dir_all(&full_path) {
                Ok(_) => {
                    tx.send(ExecutionEvent { 
                            event_type: ExecutionEventType::StepProgress { stdout: format!("Created {}", full_path.display()), stderr: String::new() },
                            step_id: step_id(step), 
                            step_index: index, 
                            total_steps: plan.step_count(), 
                            step_name: step_label(step),
                            step_description: step_description(step), 
                            timestamp: local_time() 
                        }).await.ok();

                    tx.send(ExecutionEvent { 
                        event_type: ExecutionEventType::StepCompleted { status: StepStatus::Success {message: format!("Created {}", full_path.display())}, 
                        duration_ms: start.elapsed().as_millis() as u64 },
                        step_id: step_id(step), 
                        step_index: index, 
                        total_steps: plan.step_count(), 
                        step_name: step_label(step),
                        step_description: step_description(step), 
                        timestamp: local_time() 
                    }).await.ok();
                    StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Success {
                            message: format!("Created {}", full_path.display()),
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                    }
                },
                Err(e) => { 
                    tx.send(ExecutionEvent { 
                        event_type: ExecutionEventType::StepCompleted { status: StepStatus::Failed {
                            error: format!("Failed to create directory {}: {}", full_path.display(), e),
                        }, 
                        duration_ms: start.elapsed().as_millis() as u64 },
                        step_id: step_id(step), 
                        step_index: index, 
                        total_steps: plan.step_count(), 
                        step_name: step_label(step),
                        step_description: step_description(step), 
                        timestamp: local_time() 
                    }).await.ok();
                    
                    StepResult {
                        step_id: step_id(step),
                        label: step_label(step),
                        status: StepStatus::Failed {
                            error: format!("Failed to create directory {}: {}", full_path.display(), e),
                        },
                        duration_ms: start.elapsed().as_millis() as u64,
                    }    
                },
            }
        } else {
            tx.send(ExecutionEvent { 
                        event_type: ExecutionEventType::StepCompleted { status: StepStatus::Failed {
                            error: "Expected CreateDirectory step".into(),
                        }, 
                        duration_ms: 0 },
                        step_id: step_id(step), 
                        step_index: index, 
                        total_steps: plan.step_count(), 
                        step_name: step_label(step),
                        step_description: step_description(step), 
                        timestamp: local_time() 
                    }).await.ok();
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
fn step_id(step: &Step) -> String { step.id() }
fn step_label(step: &Step) -> String { step.label() }
fn step_description(step: &Step) -> String { step.description() }

fn local_time() -> String {
    // Получаем текущее локальное время
    let local_time = Local::now();
    
    // Форматируем в строку (в chrono для двоеточия не нужен экранирующий синтаксис)
    local_time.format("%H::%M:%S").to_string()
}