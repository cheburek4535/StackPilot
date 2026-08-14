
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use chrono::Local;
use std::sync::Arc;
use std::time::Duration;
use crate::modules::project_creator::engine::ExecutionPlan;
use crate::modules::project_creator::generators::{GeneratorRegistry, sh_quote, win_command_line};
use crate::modules::project_creator::models::*;

/// Стандартные fallback-триггеры на все случаи, когда step‑специфичных нет.
static FALLBACK_TRIGGERS: &[(&str, &str)] = &[
    // Node.js / npx
    ("need to install the following packages", "y"),
    ("ok to proceed?", "y"),
    ("do you want to proceed?", "y"),
    // Универсальные булевы паттерны
    ("would you like to", "n"),
    ("do you want to", "n"),
    ("(y/n)", "y"),
    ("(y/n)", "y"),
    ("(y/n)", "y"),
    ("[y/n]", "y"),
    ("[y/n]", "y"),
    ("proceed?", "y"),
    ("accept?", "y"),
    // Зависимости
    ("install dependencies", "y"),
    ("download and install", "n"),
    ("install the ios and android", "n"),
    ("initialize a new git", "y"),
    // CocoaPods (macOS)
    ("install cocoapods", "n"),
    // Expo
    ("do you want to log in", "n"),
    // Лицензии
    ("accept the license", "y"),
    ("review licenses", "y"),
];

/// StepExecutor — выполняет отдельные шаги плана.
pub struct StepExecutor {
    /// Встроенные генераторы для шагов Step::Generate
    /// (spring-boot, fs-cleanup, cli).
    pub generators: Arc<GeneratorRegistry>,
}

impl StepExecutor {
    pub fn new() -> Self {
        Self {
            generators: Arc::new(GeneratorRegistry::with_defaults()),
        }
    }

    pub fn with_generators(generators: Arc<GeneratorRegistry>) -> Self {
        Self { generators }
    }

    /// Собрать карту триггеров из interactive-поля шага (владеющие данные).
    fn build_trigger_map(step: &Step) -> Vec<(String, ResponseType)> {
        let mut map: Vec<(String, ResponseType)> = Vec::new();
        if let Step::Command { interactive, .. } = step {
            for entry in interactive {
                map.push((entry.trigger.clone(), entry.response_type.clone()));
            }
        }
        map
    }

    /// Отправить ответ в stdin процесса согласно ResponseType.
    async fn send_response(
        stdin: &mut tokio::process::ChildStdin,
        rt: &ResponseType,
    ) {
        let bytes: Vec<u8> = match rt {
            ResponseType::Text(val) => {
                format!("{}\n", val).into_bytes()
            }
            ResponseType::Confirm(true) => b"y\n".to_vec(),
            ResponseType::Confirm(false) => b"n\n".to_vec(),
            ResponseType::Select(idx) => {
                // *idx* раз нажать стрелку вниз, затем Enter
                let mut seq = Vec::new();
                for _ in 0..*idx {
                    seq.extend_from_slice(b"\x1b[B"); // Down arrow
                }
                seq.push(b'\n');                     // Enter
                seq
            }
            ResponseType::Keys(raw) => raw.as_bytes().to_vec(),
        };
        let _ = stdin.write_all(&bytes).await;
        let _ = stdin.flush().await;
    }

    /// Проверить rolling‑буфер на совпадение с любым триггером.
    /// Возвращает true, если совпадение найдено и ответ отправлен.
    async fn check_triggers(
        rolling: &str,
        trigger_map: &[(String, ResponseType)],
        stdin: &mut tokio::process::ChildStdin,
    ) -> bool {
        let lower = rolling.to_lowercase();
        for (trigger, response_type) in trigger_map {
            if lower.contains(&trigger.to_lowercase()) {
                Self::send_response(stdin, response_type).await;
                return true;
            }
        }
        // Fallback — более широкая сеть
        for (trigger, response) in FALLBACK_TRIGGERS {
            if lower.contains(trigger) {
                let _ = stdin.write_all(response.as_bytes()).await;
                let _ = stdin.write_all(b"\n").await;
                let _ = stdin.flush().await;
                return true;
            }
        }
        false
    }

    /// Прочитать stdout с побайтовым накоплением, детекцией триггеров и отправкой ответов.
    async fn read_stdout_loop(
        mut stdout: tokio::process::ChildStdout,
        mut stdin: tokio::process::ChildStdin,
        trigger_map: Vec<(String, ResponseType)>, // владеющие данные
        tx: mpsc::Sender<ExecutionEvent>,
        step_id: String,
        step_name: String,
        step_desc: String,
        index: usize,
        total_steps: usize,
    ) {
        // 256‑байтовый буфер — не ждём \n, читаем как только данные появляются
        let mut buf = [0u8; 256];
        // Скользящее окно 2048 символов
        let mut rolling = String::with_capacity(2048);
        // Таймаут бездействия перед fallback-опросом
        let idle_timeout = Duration::from_secs(4);

        loop {
            let read_fut = stdout.read(&mut buf);
            let result = tokio::time::timeout(idle_timeout, read_fut).await;

            match result {
                // Данные пришли
                Ok(Ok(0)) => break, // EOF — процесс закрыл stdout
                Ok(Ok(n)) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]);

                    // Печатаем в консоль для отладки
                    print!("{}", chunk);
                    let _ = std::io::Write::flush(&mut std::io::stdout());

                    // Добавляем в скользящее окно
                    rolling.push_str(&chunk);
                    if rolling.len() > 2048 {
                        rolling.drain(..rolling.len() - 2048);
                    }

                    // Ищем триггеры
                    let matched = Self::check_triggers(&rolling, &trigger_map, &mut stdin).await;
                    if matched {
                        rolling.clear(); // Очищаем окно после ответа
                    }

                    // Шлём событие на фронтенд
                    let _ = tx
                        .send(ExecutionEvent {
                            event_type: ExecutionEventType::StepProgress {
                                stdout: chunk.into(),
                                stderr: String::new(),
                            },
                            step_id: step_id.clone(),
                            step_index: index,
                            total_steps,
                            step_name: step_name.clone(),
                            step_description: step_desc.clone(),
                            timestamp: local_time(),
                        })
                        .await;
                }
                // Ошибка чтения
                Ok(Err(_)) => break,
                // Таймаут — процесс молчит, возможно ждёт ввода без триггера
                Err(_elapsed) => {
                    // Пробуем последний раз проверить буфер и отправить fallback
                    if !rolling.is_empty() {
                        let matched = Self::check_triggers(&rolling, &trigger_map, &mut stdin).await;
                        if matched {
                            rolling.clear();
                            continue;
                        }
                    }
                    // Если процесс ещё жив — отправляем "y\n" как последнее средство
                    // (иначе выходим — процесс сам завершится)
                    let _ = stdin.write_all(b"\n").await;
                    let _ = stdin.flush().await;
                }
            }
        }
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

    let (command, raw_args, working_dir, env, timeout_secs) = match step {
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

    // Подавление интерактивности: npx без --yes спрашивает «Ok to proceed?
    // (y)» и ждёт ввода, убивая автоматическую генерацию. Флаг добавляется
    // первым аргументом, если его ещё нет (npx --yes nuxt ... и т.п.).
    let mut args: Vec<String> = raw_args.clone();
    if command == "npx" && !args.iter().any(|a| a.starts_with("--yes")) {
        args.insert(0, "--yes".into());
    }

    // Кроссплатформенный запуск через shell
    let mut cmd = match OS {
        "windows" => {
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
                // cmd /S /C с правильно кавычкуемой строкой: команда может
                // быть АБСОЛЮТНЫМ путём с пробелами (venv\Scripts\pip.exe),
                // аргументы — путями и URL. Внешняя пара кавычек обязательна:
                // без неё cmd снимает первую кавычку и режет команду по
                // пробелам.
                let mut win_cmd = tokio::process::Command::new("cmd");
                win_cmd.arg("/S");
                win_cmd.arg("/C");
                win_cmd.arg(win_command_line(command, &args));
                win_cmd
            }
        }
        _ => {
            let mut unix_cmd = tokio::process::Command::new("sh");
            unix_cmd.arg("-c");
            let mut shell_cmd = String::from(command);
            for arg in args.iter() {
                shell_cmd.push(' ');
                shell_cmd.push_str(&sh_quote(arg));
            }
            unix_cmd.arg(shell_cmd);
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

    // Принудительный неинтерактивный режим для ВСЕХ команд: CI=1 заставляет
    // npm/prisma/create-* CLI (Tauri, Vite, Next.js...) пропускать промпты,
    // NPM_CONFIG_YES отвечает «да» на подтверждение установки пакета у npx.
    // Без этого prisma init повисает на вопросе о БД, create-* ждут Enter.
    cmd.env("CI", "1");
    cmd.env("NPM_CONFIG_YES", "true");

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::piped());

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
    let stdin = child.stdin.take().expect("stdin should be piped");

    // Строим карту триггеров из interactive-поля шага
    let trigger_map = Self::build_trigger_map(step);

    // Запускаем stdout-читалку в отдельном таске
    let tx_stdout = tx.clone();
    let step_id_out = step_id(step);
    let step_name_out = step_label(step);
    let step_desc_out = step_description(step);

    let stdout_handle = tokio::spawn(async move {
        Self::read_stdout_loop(
            stdout,
            stdin,
            trigger_map,
            tx_stdout,
            step_id_out,
            step_name_out,
            step_desc_out,
            index,
            total_steps,
        )
        .await;
    });

    // Для stderr
    let tx_stderr = tx.clone();
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

    /// Выполнить шаг Generate: диспетчеризация во встроенные генераторы
    /// движка (spring-boot, fs-cleanup, cli). Ошибка генератора (например,
    /// «Spring Initializr error: HTTP 400 ...») становится Failed-статусом
    /// шага и останавливает пайплайн при on_error=Abort.
    pub async fn run_generate(
        &self,
        step: &Step,
        plan: &ExecutionPlan,
        tx: &mpsc::Sender<ExecutionEvent>,
        index: usize,
    ) -> StepResult {
        let (generator_id, generator_config) = match step {
            Step::Generate {
                generator_id,
                generator_config,
                ..
            } => (generator_id, generator_config),
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

        tx.send(ExecutionEvent {
            event_type: ExecutionEventType::StepStarted,
            step_id: step_id(step),
            step_index: index,
            total_steps: plan.step_count(),
            step_name: step_label(step),
            step_description: step_description(step),
            timestamp: local_time(),
        })
        .await
        .ok();

        let start = std::time::Instant::now();

        let outcome = match self.generators.get(generator_id) {
            Some(generator) => {
                generator
                    .generate(&plan.context, &plan.project_path, generator_config)
                    .await
            }
            None => Err(format!("Unknown generator '{}'", generator_id)),
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

        tx.send(ExecutionEvent {
            event_type: ExecutionEventType::StepCompleted {
                status: status.clone(),
                duration_ms,
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