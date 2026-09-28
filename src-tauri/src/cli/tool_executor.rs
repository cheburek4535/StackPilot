use std::sync::Arc;
use std::time::Duration;

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use tokio::task::JoinSet;

use crate::modules::toolchain::core::metadata::MetadataStore;
use crate::modules::toolchain::defs::load_definitions;
use crate::modules::toolchain::engine::{
    self, build_plan, run_job, EngineRequest, EngineTaskStatus, JobEngine, JobEvent,
    JobEventPayload, OperationKind, Phase, PlanInputs, RealRunner, TaskAction, TaskRunner,
    TcxEventSink, ToolRequest,
};
use crate::modules::toolchain::models::{InstallSource, InstalledToolInfo, ToolDefinition, ToolStatus};
use crate::platform::paths::get_app_data_dir;

/// Returns sources defined for the given OS (windows, linux, macos)
pub fn tool_sources_for_os<'a>(def: &'a ToolDefinition, os: &str) -> &'a [InstallSource] {
    match os {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[],
    }
}

/// Normalizes tool IDs and aliases (e.g. "postgres" -> "postgresql", "js" -> "node").
pub fn normalize_tool_id(input: &str, defs: &[ToolDefinition]) -> String {
    let lower = input.trim().to_lowercase();
    // Direct match by ID
    if let Some(def) = defs.iter().find(|d| d.id.to_lowercase() == lower) {
        return def.id.clone();
    }
    // Match by display name
    if let Some(def) = defs.iter().find(|d| d.display.to_lowercase() == lower) {
        return def.id.clone();
    }
    // Common aliases
    match lower.as_str() {
        "postgres" | "pg" | "psql" => "postgresql".to_string(),
        "js" | "nodejs" => "node".to_string(),
        "ts" | "typescript" => "node".to_string(),
        "py" | "python3" => "python".to_string(),
        "golang" => "go".to_string(),
        "rustlang" => "rust".to_string(),
        "compose" | "docker-compose" => "docker".to_string(),
        "gh" | "github" => "gh".to_string(),
        "vscode" | "code" => "vscode".to_string(),
        _ => lower,
    }
}

/// Progress sink reporting live task events to console with indicatif
struct CliJobSink {
    pb: ProgressBar,
}

impl TcxEventSink for CliJobSink {
    fn emit(&self, event: &JobEvent) {
        match &event.payload {
            JobEventPayload::JobStarted {
                operation,
                total_tasks,
            } => {
                self.pb.set_message(format!(
                    "Executing {} for {} package(s)...",
                    operation.as_str(),
                    total_tasks
                ));
            }
            JobEventPayload::TaskStarted { index, total } => {
                self.pb.set_message(format!(
                    "[{}/{}] Preparing {}...",
                    index + 1,
                    total,
                    event.tool_id.bold()
                ));
            }
            JobEventPayload::TaskPhase { phase } => {
                let phase_desc = match phase {
                    Phase::Validating => "Validating package...",
                    Phase::Preparing => "Preparing environment...",
                    Phase::Downloading => "Downloading files...",
                    Phase::Verifying => "Verifying checksum integrity...",
                    Phase::Installing => "Installing package...",
                    Phase::Configuring => "Configuring...",
                    Phase::UpdatingPath => "Updating PATH environment variables...",
                    Phase::CheckingHealth => "Running post-install verification...",
                    Phase::Completed => "Complete",
                    Phase::Failed => "Failed",
                    Phase::Cancelled => "Cancelled",
                    Phase::Interrupted => "Interrupted",
                };
                self.pb
                    .set_message(format!("{}: {}", event.tool_id.bold(), phase_desc));
            }
            JobEventPayload::Progress { line } => {
                let clean = line.trim();
                if !clean.is_empty() {
                    self.pb
                        .set_message(format!("{}: {}", event.tool_id.bold(), clean));
                }
            }
            JobEventPayload::PathUpdated { record } => {
                self.pb.println(format!(
                    "  {} Updated PATH for {}: {}",
                    "➜".cyan(),
                    event.tool_id.bold(),
                    record.added.join("; ").dimmed()
                ));
            }
            JobEventPayload::TaskCompleted { status } => match status {
                EngineTaskStatus::Succeeded { version } => {
                    self.pb.println(format!(
                        "  {} {} (v{})",
                        "✔".green().bold(),
                        event.tool_id.bold(),
                        version.cyan()
                    ));
                }
                EngineTaskStatus::Failed { error } => {
                    self.pb.println(format!(
                        "  {} {} failed: {}",
                        "✖".red().bold(),
                        event.tool_id.bold(),
                        error
                    ));
                }
                EngineTaskStatus::Cancelled => {
                    self.pb.println(format!(
                        "  {} {} cancelled",
                        "⚠".yellow().bold(),
                        event.tool_id.bold()
                    ));
                }
                _ => {}
            },
            JobEventPayload::JobFinished { .. } => {}
        }
    }
}

/// Executes install or update operation for a list of tools
pub async fn execute_tool_operation(
    op: OperationKind,
    tool_inputs: &[String],
    yes: bool,
    force_reinstall: bool,
) -> Result<(), String> {
    let defs = load_definitions();
    let app_data_dir = get_app_data_dir();
    let toolchain_dir = app_data_dir.join("toolchain");
    std::fs::create_dir_all(&toolchain_dir)
        .map_err(|e| format!("Failed to create toolchain dir: {e}"))?;

    // Normalize IDs and check existence
    let mut resolved_ids: Vec<String> = Vec::new();
    for input in tool_inputs {
        let norm = normalize_tool_id(input, &defs);
        if !defs.iter().any(|d| d.id == norm) {
            return Err(format!(
                "Unknown tool '{}'. Run 'stkpil list tools' to view available packages.",
                input
            ));
        }
        if !resolved_ids.contains(&norm) {
            resolved_ids.push(norm);
        }
    }

    if resolved_ids.is_empty() {
        return Err("No valid tools specified for this operation".to_string());
    }

    let tool_requests: Vec<ToolRequest> = resolved_ids
        .iter()
        .map(|id| {
            let mut req = ToolRequest::id(id);
            req.force_reinstall = force_reinstall;
            req
        })
        .collect();

    let mut request = EngineRequest::new(op, tool_requests);
    request.confirm_admin_elevation = true;
    request.confirm_unverified_sources = true;

    let detector = engine::DiscoveryDetector;
    let free_space = crate::modules::toolchain::core::disk::free_space_mb(
        &crate::modules::toolchain::core::disk::install_root(),
    )
    .await
    .unwrap_or(u64::MAX);

    let inputs = PlanInputs::new(&defs, &detector).with_free_space(free_space);

    let plan_spinner = ProgressBar::new_spinner();
    plan_spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    plan_spinner.set_message("Resolving package versions and dependency graph...");
    plan_spinner.enable_steady_tick(Duration::from_millis(80));

    let plan = build_plan(&request, &inputs)
        .await
        .map_err(|e| format!("Planning failed: {e}"))?;

    plan_spinner.finish_and_clear();

    // Check if there are actionable tasks
    let actionable_tasks: Vec<_> = plan.tasks.iter().filter(|t| !t.action.is_noop()).collect();
    if actionable_tasks.is_empty() {
        println!(
            "\n{}",
            "✔ All requested packages are already installed and up to date!"
                .green()
                .bold()
        );
        for task in &plan.tasks {
            if let TaskAction::NoOp(reason) = &task.action {
                match reason {
                    engine::NoopReason::AlreadyInstalled { version } => {
                        println!("  {} {} (v{}) is already installed", "✔".green(), task.tool_id.bold(), version);
                    }
                    engine::NoopReason::UpdateUnavailable { version } => {
                        println!("  {} {} (v{}) is on the latest recommended version", "✔".green(), task.tool_id.bold(), version);
                    }
                    engine::NoopReason::DockerManaged => {
                        println!("  {} {} is managed by Docker Compose", "ℹ".blue(), task.tool_id.bold());
                    }
                }
            }
        }
        println!();
        return Ok(());
    }

    // Print summary table before running
    println!("\n{}", "📋 Package Execution Plan:".bold());
    println!("{}", "─".repeat(56).dimmed());
    for task in &actionable_tasks {
        let action_desc = match &task.action {
            TaskAction::InstallNew { target_version } => {
                if let Some(v) = target_version {
                    format!("Install {}", format!("v{}", v).cyan())
                } else {
                    "Install (latest)".cyan().to_string()
                }
            }
            TaskAction::Update {
                current_version,
                target_version,
            } => {
                let target = target_version.as_deref().unwrap_or("latest");
                format!("Update ({} -> {})", current_version.yellow(), target.green())
            }
            TaskAction::RepairPath => "Repair PATH".magenta().to_string(),
            TaskAction::HealthCheck => "Health Check".blue().to_string(),
            TaskAction::NoOp(_) => "No-op".dimmed().to_string(),
        };

        let admin_str = if task.needs_admin {
            " [requires Administrator]".yellow().to_string()
        } else {
            String::new()
        };

        println!(
            "  • {:<16} {:<24} (~{} MB){}",
            task.tool_id.bold(),
            action_desc,
            task.size_mb,
            admin_str
        );
    }
    println!("{}", "─".repeat(56).dimmed());
    println!("Total download size: ~{} MB\n", plan.total_size_mb);

    // Confirmation if interactive
    if !yes {
        let prompt_text = match op {
            OperationKind::Install => "Proceed with installation?",
            OperationKind::Update => "Proceed with package updates?",
            _ => "Proceed with package execution?",
        };
        let proceed = inquire::Confirm::new(prompt_text)
            .with_default(true)
            .prompt()
            .map_err(|e| format!("Confirmation failed: {e}"))?;

        if !proceed {
            println!("{}", "Operation cancelled by user.".yellow());
            return Ok(());
        }
    }

    // Initialize JobEngine and register
    let job_engine = Arc::new(JobEngine::load(&toolchain_dir));
    let metadata_store = Arc::new(std::sync::Mutex::new(MetadataStore::load(&toolchain_dir)));

    let handle = job_engine.register(plan)?;
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(80));

    let sink: Arc<dyn TcxEventSink> = Arc::new(CliJobSink { pb: pb.clone() });
    job_engine.add_sink(sink.clone());

    let runner: Arc<dyn TaskRunner> = Arc::new(RealRunner {
        engine: job_engine.clone(),
        handle: handle.clone(),
    });

    let _secrets = run_job(
        job_engine.clone(),
        handle.clone(),
        Arc::new(defs.clone()),
        runner,
    )
    .await;

    pb.finish_and_clear();
    job_engine.remove_sink(&sink);

    let snapshot = handle.snapshot();

    // Persist successful installations to state.json
    {
        let mut meta = metadata_store.lock().expect("metadata poisoned");
        for task in &snapshot.plan.tasks {
            if let EngineTaskStatus::Succeeded { version } = &task.status {
                if let Some(def) = defs.iter().find(|d| d.id == task.tool_id) {
                    meta.record_tool_installed(
                        &task.tool_id,
                        InstalledToolInfo {
                            path: crate::modules::toolchain::core::discovery::installed_path(def)
                                .unwrap_or_default(),
                            version: version.clone(),
                            installed_at: crate::modules::toolchain::core::console::timestamp(),
                            path_entries: def.path_entries.clone(),
                        },
                    );
                }
            }
        }
        let _ = meta.save();
    }

    // Sync process PATH so new binaries are immediately discoverable
    let _ = crate::modules::toolchain::core::path_service::sync_process_path().await;

    // Report final outcomes
    println!();
    match snapshot.status {
        engine::JobStatus::Succeeded => {
            println!(
                "{}",
                "🎉 All packages were successfully installed and configured!"
                    .green()
                    .bold()
            );
        }
        engine::JobStatus::Partial => {
            println!(
                "{}",
                "⚠ Completed with some errors. See task details above."
                    .yellow()
                    .bold()
            );
        }
        engine::JobStatus::Failed => {
            println!(
                "{}",
                "✖ Package execution failed. See error details above."
                    .red()
                    .bold()
            );
        }
        engine::JobStatus::Cancelled => {
            println!("{}", "⚠ Package execution was cancelled.".yellow().bold());
        }
        engine::JobStatus::Interrupted => {
            println!("{}", "⚠ Package execution was interrupted.".yellow().bold());
        }
        _ => {}
    }

    Ok(())
}

/// Scans and audits tools concurrently, with live progress spinner
pub async fn scan_tools(
    category_filter: Option<&str>,
) -> Vec<(ToolDefinition, ToolStatus)> {
    let defs = load_definitions();
    let os = crate::modules::toolchain::platforms::current_platform().os_name();

    // Filter tools if requested
    let filtered_defs: Vec<ToolDefinition> = defs
        .into_iter()
        .filter(|d| {
            // Filter by OS availability: tool must have detection or sources for this OS
            let det = d.effective_detection(&os);
            let has_detection = !det.version_probes.is_empty()
                || !det.known_paths.is_empty()
                || !det.registry_keys.is_empty();
            let sources = tool_sources_for_os(d, &os);
            let has_sources = !sources.is_empty() || d.manual_install.is_some();
            if !has_detection && !has_sources {
                return false;
            }

            if let Some(cat) = category_filter {
                let cat_lower = cat.to_lowercase();
                if cat_lower != "all" {
                    return match cat_lower.as_str() {
                        "lang" | "languages" | "language" => d.category == "language",
                        "pm" | "package-managers" | "package_manager" => {
                            d.category == "package_manager"
                        }
                        "db" | "databases" | "database" => d.category == "database",
                        "devops" | "containers" | "cloud" => {
                            d.category == "container" || d.category == "vcs" || d.category == "cli"
                        }
                        "editors" | "ide" | "editor" => d.category == "editor",
                        other => d.category.to_lowercase() == other,
                    };
                }
            }
            true
        })
        .collect();

    let total = filtered_defs.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} scanning {msg}...")
            .unwrap()
            .progress_chars("━╸ "),
    );
    pb.enable_steady_tick(Duration::from_millis(80));

    let mut join_set = JoinSet::new();
    for def in filtered_defs {
        join_set.spawn(async move {
            let status = crate::modules::toolchain::core::discovery::detect_tool(&def).await;
            (def, status)
        });
    }

    let mut results = Vec::new();
    while let Some(res) = join_set.join_next().await {
        if let Ok((def, status)) = res {
            pb.set_message(def.display.clone());
            pb.inc(1);
            results.push((def, status));
        }
    }

    pb.finish_and_clear();

    // Sort alphabetically by display name
    results.sort_by(|a, b| a.0.display.cmp(&b.0.display));
    results
}
