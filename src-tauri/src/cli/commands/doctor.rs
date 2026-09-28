use colored::Colorize;

use crate::cli::args::DoctorArgs;
use crate::cli::tool_executor::{execute_tool_operation, scan_tools};
use crate::modules::toolchain::engine::OperationKind;
use crate::modules::toolchain::models::{ToolDefinition, ToolStatus};

pub async fn execute(args: DoctorArgs) -> Result<(), String> {
    println!("\n{}", "🩺 Running StackPilot Doctor System Audit...".bold().cyan());
    println!("{}", "Detecting compilers, runtimes, package managers, and development tools...\n".dimmed());

    let category_filter = if args.category == "all" {
        None
    } else {
        Some(args.category.as_str())
    };

    let scanned = scan_tools(category_filter).await;

    if args.json {
        let json_items: Vec<_> = scanned
            .iter()
            .map(|(def, status)| {
                serde_json::json!({
                    "id": def.id,
                    "display": def.display,
                    "category": def.category,
                    "status": format!("{:?}", status),
                    "is_ok": status.is_ok(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_items).unwrap());
        return Ok(());
    }

    // Categorize
    let mut installed_count = 0;
    let mut update_count = 0;
    let mut missing_count = 0;
    let mut broken_count = 0;

    let categories = [
        ("language", "🚀 Programming Languages & Runtimes"),
        ("package_manager", "📦 Package Managers"),
        ("database", "🗄 Databases & Storage"),
        ("container", "🐳 Containers & DevOps"),
        ("vcs", "🌿 Version Control & Git"),
        ("editor", "💻 Editors & IDEs"),
        ("utility", "🛠 Utilities & CLI Tools"),
        ("other", "🔧 Other Tools"),
    ];

    for (cat_key, cat_title) in &categories {
        let matching: Vec<&(ToolDefinition, ToolStatus)> = scanned
            .iter()
            .filter(|(def, _)| {
                if *cat_key == "other" {
                    !categories[..categories.len() - 1]
                        .iter()
                        .any(|(k, _)| def.category == *k)
                } else {
                    def.category == *cat_key
                }
            })
            .collect();

        if matching.is_empty() {
            continue;
        }

        println!("{}", cat_title.bold());
        println!("{}", "─".repeat(60).dimmed());

        for (def, status) in matching {
            let (icon, status_str) = match status {
                ToolStatus::Installed { version } => {
                    installed_count += 1;
                    ("✔".green().bold(), format!("v{}", version).green())
                }
                ToolStatus::UpdateAvailable {
                    installed,
                    recommended,
                } => {
                    update_count += 1;
                    (
                        "⬆".yellow().bold(),
                        format!("v{} -> v{} (Update Available)", installed, recommended).yellow(),
                    )
                }
                ToolStatus::Missing => {
                    missing_count += 1;
                    ("✖".red().bold(), "Missing".red())
                }
                ToolStatus::PathBroken { reason } => {
                    broken_count += 1;
                    ("⚠".magenta().bold(), format!("Broken: {}", reason).magenta())
                }
                ToolStatus::ManualInstall { reason } => (
                    "ℹ".dimmed(),
                    format!("Manual: {}", reason.lines().next().unwrap_or("")).dimmed(),
                ),
                ToolStatus::RunInDocker => (
                    "🐳".cyan(),
                    "Managed via Docker Compose".cyan(),
                ),
            };

            let name_display = format!("{} ({})", def.display, def.id);
            println!("  {} {:<32} {}", icon, name_display.bold(), status_str);
        }
        println!();
    }

    // Summary Box
    println!("{}", "┌────────────────────── Diagnostic Summary ──────────────────────┐".cyan());
    println!(
        "│  Installed: {:<4} │ Updates: {:<4} │ Missing: {:<4} │ Broken: {:<4} │",
        installed_count.to_string().green().bold(),
        update_count.to_string().yellow().bold(),
        missing_count.to_string().red().bold(),
        broken_count.to_string().magenta().bold()
    );
    println!("{}", "└────────────────────────────────────────────────────────────────┘".cyan());
    println!();

    // Check if there are actions to take
    let mut candidate_tools: Vec<(String, String)> = Vec::new();
    for (def, status) in &scanned {
        match status {
            ToolStatus::Missing => {
                // Only tools that have automated sources on current OS
                let os = crate::modules::toolchain::platforms::current_platform().os_name();
                if !crate::cli::tool_executor::tool_sources_for_os(def, &os).is_empty() {
                    candidate_tools.push((
                        def.id.clone(),
                        format!("Install {} (~{} MB)", def.display, def.size_mb),
                    ));
                }
            }
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            } => {
                candidate_tools.push((
                    def.id.clone(),
                    format!(
                        "Update {} (v{} -> v{})",
                        def.display, installed, recommended
                    ),
                ));
            }
            _ => {}
        }
    }

    if candidate_tools.is_empty() {
        println!("{}", "✨ Your development environment is in top shape!".green().bold());
        return Ok(());
    }

    // Auto fix if flag passed
    if args.fix {
        println!("{}", "Applying automated fixes for missing/outdated packages...".cyan().bold());
        let tool_ids: Vec<String> = candidate_tools.into_iter().map(|(id, _)| id).collect();
        return execute_tool_operation(OperationKind::Install, &tool_ids, args.yes, false).await;
    }

    // Interactive prompt
    if !args.yes {
        let should_fix = inquire::Confirm::new(
            "Would you like to install or update missing/outdated developer tools?",
        )
        .with_default(true)
        .prompt()
        .map_err(|e| format!("Prompt error: {e}"))?;

        if !should_fix {
            return Ok(());
        }

        let options: Vec<String> = candidate_tools.iter().map(|(_, label)| label.clone()).collect();
        let selected_labels = inquire::MultiSelect::new(
            "Select packages to install or update:",
            options.clone(),
        )
        .prompt()
        .map_err(|e| format!("Selection error: {e}"))?;

        if selected_labels.is_empty() {
            println!("No packages selected.");
            return Ok(());
        }

        let selected_ids: Vec<String> = candidate_tools
            .into_iter()
            .filter(|(_, label)| selected_labels.contains(label))
            .map(|(id, _)| id)
            .collect();

        return execute_tool_operation(OperationKind::Install, &selected_ids, false, false).await;
    }

    Ok(())
}
