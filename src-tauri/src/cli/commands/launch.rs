use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use colored::{Color, Colorize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::cli::args::LaunchArgs;
use crate::modules::devlauncher::analyzer::{AnalyzeOptions, FsProjectAnalyzer, ProjectAnalyzerV2};
use crate::modules::devlauncher::models::{LaunchProfileV2, LaunchStep, StepKind};
use crate::modules::devlauncher::profile_manager::{JsonProfileManager, ProfileManagerV2};
use crate::platform::paths::get_app_data_dir;

const PALETTE: [Color; 7] = [
    Color::Cyan,
    Color::Magenta,
    Color::Yellow,
    Color::Green,
    Color::Blue,
    Color::BrightCyan,
    Color::BrightYellow,
];

pub async fn execute(args: LaunchArgs) -> Result<(), String> {
    let project_path = args.path.canonicalize().unwrap_or(args.path.clone());
    let path_str = project_path.to_string_lossy().to_string();

    if !project_path.exists() {
        return Err(format!("Directory '{}' does not exist", path_str));
    }

    println!("\n{}", "🚀 StackPilot DevLauncher CLI".bold().cyan());
    println!("{}", "─".repeat(60).dimmed());
    println!("  Directory: {}", path_str.bold());

    // 1. Resolve Profile: Check saved profiles or run analyzer
    let app_data_dir = get_app_data_dir();
    let profiles_dir = app_data_dir.join("profiles");
    let profile_manager = JsonProfileManager::new(profiles_dir);

    let profile: LaunchProfileV2 = if let Some(ref name) = args.profile {
        // Load specific saved profile
        profile_manager
            .get_profile_v2(name)
            .map_err(|e| format!("Failed to load profile '{}': {}", name, e))?
    } else if let Some(existing) = profile_manager.find_by_project_path_v2(&path_str) {
        println!("  Profile:   {} (loaded from saved profiles)", existing.name.bold());
        existing
    } else {
        // Auto-detect with FsProjectAnalyzer
        println!("  Profile:   Auto-detecting project configuration...");
        let analyzer = FsProjectAnalyzer;
        let draft = analyzer
            .analyze_draft(&path_str, &AnalyzeOptions::default())
            .map_err(|e| format!("Project analysis failed: {e}"))?;
        println!("  Profile:   {} (auto-detected)", draft.profile.name.bold());
        draft.profile
    };

    println!("{}", "─".repeat(60).dimmed());

    // Filter runnable steps
    let enabled_steps: Vec<LaunchStep> = profile
        .steps
        .into_iter()
        .filter(|s| s.enabled)
        .collect();

    if enabled_steps.is_empty() {
        println!(
            "\n{}",
            "⚠ No runnable services or steps found for this project."
                .yellow()
                .bold()
        );
        println!("Run 'stkpil analyze' to inspect the directory or create a profile in StackPilot Desktop.\n");
        return Ok(());
    }

    // Dry run
    if args.dry_run {
        println!("\n{}", "📋 Dry-Run: The following steps would be launched:".bold());
        for (i, step) in enabled_steps.iter().enumerate() {
            println!("  [{}] {} ({:?})", i + 1, step.label.bold(), step.kind);
        }
        println!();
        return Ok(());
    }

    // Step selection if interactive and multiple steps exist
    let selected_steps: Vec<LaunchStep> = if args.select || (!args.all && enabled_steps.len() > 1) {
        let options: Vec<String> = enabled_steps
            .iter()
            .map(|s| match &s.kind {
                StepKind::RunCommand { command, .. } => format!("{}: {}", s.label, command),
                StepKind::RunScript { script, .. } => format!("{}: {}", s.label, script),
                StepKind::WaitForPort { host, port, .. } => {
                    format!("{}: wait for {}:{}", s.label, host, port)
                }
                StepKind::OpenUrl { url } => format!("{}: open {}", s.label, url),
                StepKind::OpenApplication { path, .. } => format!("{}: open app {}", s.label, path),
                _ => format!("{}: other step", s.label),
            })
            .collect();

        let defaults: Vec<usize> = (0..options.len()).collect();

        let prompt = inquire::MultiSelect::new("Select services to launch (Space to toggle, Enter to launch):", options.clone())
            .with_default(&defaults);

        let selected = prompt.prompt().map_err(|e| format!("Selection prompt error: {e}"))?;

        if selected.is_empty() {
            println!("{}", "No services selected. Exiting.".yellow());
            return Ok(());
        }

        enabled_steps
            .into_iter()
            .enumerate()
            .filter(|(i, _)| selected.contains(&options[*i]))
            .map(|(_, s)| s)
            .collect()
    } else {
        enabled_steps
    };

    println!("\n{}", "⚡ Launching services... (Press Ctrl+C to terminate all services)".bold().green());
    println!("{}", "─".repeat(60).dimmed());

    // Tracking for graceful cancellation
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let child_pids: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let child_pids_clone = child_pids.clone();

    // Spawn Ctrl+C listener
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        running_clone.store(false, Ordering::SeqCst);
        println!(
            "\n\n{}",
            "🛑 Received shutdown signal (Ctrl+C). Terminating all running services..."
                .bold()
                .red()
        );

        let pids = child_pids_clone.lock().await.clone();
        for pid in pids {
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(&["/F", "/T", "/PID", &pid.to_string()])
                    .output();
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = std::process::Command::new("kill")
                    .args(&["-TERM", &pid.to_string()])
                    .output();
            }
        }
    });

    let mut color_idx = 0;
    let mut tasks = Vec::new();

    for step in selected_steps {
        let color = PALETTE[color_idx % PALETTE.len()];
        color_idx += 1;

        let label = step.label.clone();
        let project_dir = project_path.clone();
        let running_ref = running.clone();
        let pids_ref = child_pids.clone();
        let no_open = args.no_open;

        match step.kind {
            StepKind::RunCommand {
                command,
                command_spec: _,
            } => {
                let working_dir = if let Some(dir) = step.working_directory {
                    project_dir.join(dir)
                } else {
                    project_dir
                };

                let task = tokio::spawn(async move {
                    run_service_command(
                        &label,
                        &command,
                        &working_dir,
                        color,
                        running_ref,
                        pids_ref,
                    )
                    .await;
                });
                tasks.push(task);
            }
            StepKind::RunScript { script, shell: _ } => {
                let working_dir = if let Some(dir) = step.working_directory {
                    project_dir.join(dir)
                } else {
                    project_dir
                };

                let task = tokio::spawn(async move {
                    run_service_command(
                        &label,
                        &script,
                        &working_dir,
                        color,
                        running_ref,
                        pids_ref,
                    )
                    .await;
                });
                tasks.push(task);
            }
            StepKind::WaitForPort { host, port, .. } => {
                let task = tokio::spawn(async move {
                    let prefix = format!("[{}]", label).color(color).bold();
                    println!("{} Waiting for port {}:{}...", prefix, host, port);

                    let start = tokio::time::Instant::now();
                    let timeout = Duration::from_secs(60);

                    loop {
                        if !running_ref.load(Ordering::SeqCst) {
                            break;
                        }
                        if tokio::time::Instant::now() - start > timeout {
                            println!("{} {} Port {}:{} timed out after 60s", prefix, "✖".red(), host, port);
                            break;
                        }

                        let addr = format!("{}:{}", host, port);
                        let is_ready = tokio::task::spawn_blocking(move || {
                            std::net::TcpStream::connect(addr).is_ok()
                        })
                        .await
                        .unwrap_or(false);

                        if is_ready {
                            println!(
                                "{} {} Port {}:{} is now ready and accepting connections!",
                                prefix,
                                "✔".green().bold(),
                                host,
                                port
                            );
                            break;
                        }

                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                });
                tasks.push(task);
            }
            StepKind::OpenUrl { url } => {
                if !no_open {
                    let task = tokio::spawn(async move {
                        // Small delay to allow servers to start
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        if running_ref.load(Ordering::SeqCst) {
                            let prefix = format!("[{}]", label).color(color).bold();
                            println!("{} Opening browser at {}", prefix, url.blue().underline());
                            let _ = webbrowser::open(&url);
                        }
                    });
                    tasks.push(task);
                }
            }
            StepKind::OpenApplication { path, args } => {
                let task = tokio::spawn(async move {
                    let prefix = format!("[{}]", label).color(color).bold();
                    println!("{} Launching external application: {}", prefix, path);
                    let mut cmd = std::process::Command::new(&path);
                    if let Some(args_list) = args {
                        cmd.args(&args_list);
                    }
                    let _ = cmd.spawn();
                });
                tasks.push(task);
            }
            _ => {}
        }
    }

    // Wait for all services or shutdown
    for task in tasks {
        let _ = task.await;
    }

    println!("\n{}", "✔ All development services stopped.".bold().green());
    Ok(())
}

async fn run_service_command(
    label: &str,
    cmd_str: &str,
    working_dir: &Path,
    color: Color,
    running: Arc<AtomicBool>,
    child_pids: Arc<Mutex<Vec<u32>>>,
) {
    let prefix = format!("[{}]", label).color(color).bold();

    println!("{} Spawning: {}", prefix, cmd_str.dimmed());

    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd.exe");
        c.args(&["/C", cmd_str]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(&["-c", cmd_str]);
        c
    };

    command.current_dir(working_dir);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            println!("{} {} Failed to start: {}", prefix, "✖".red(), e);
            return;
        }
    };

    if let Some(pid) = child.id() {
        child_pids.lock().await.push(pid);
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let prefix_out = prefix.clone();
    let stdout_task = tokio::spawn(async move {
        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                println!("{} {}", prefix_out, line);
            }
        }
    });

    let prefix_err = prefix.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(err) = stderr {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("{} {}", prefix_err, line);
            }
        }
    });

    // Wait for child process exit or cancellation
    let _ = child.wait().await;
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if running.load(Ordering::SeqCst) {
        println!("{} Service exited.", prefix);
    }
}
