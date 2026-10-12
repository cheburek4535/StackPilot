use colored::Colorize;

use crate::cli::args::{InstallArgs, StatusArgs, UpdateArgs};
use crate::cli::tool_executor::{execute_tool_operation, normalize_tool_id, scan_tools};
use crate::modules::toolchain::defs::load_definitions;
use crate::modules::toolchain::engine::OperationKind;
use crate::modules::toolchain::models::ToolStatus;

pub async fn execute_install(args: InstallArgs) -> Result<(), String> {
    execute_tool_operation(
        OperationKind::Install,
        &args.tools,
        args.yes,
        args.force_reinstall,
    )
    .await
}

pub async fn execute_update(args: UpdateArgs) -> Result<(), String> {
    if !args.tools.is_empty() {
        return execute_tool_operation(OperationKind::Update, &args.tools, args.yes, false).await;
    }

    // Check all tools for available updates
    println!(
        "\n{}",
        "🔍 Checking for available package updates...".bold().cyan()
    );
    let scanned = scan_tools(None).await;

    let outdated: Vec<_> = scanned
        .into_iter()
        .filter(|(_, status)| matches!(status, ToolStatus::UpdateAvailable { .. }))
        .collect();

    if outdated.is_empty() {
        println!(
            "\n{}",
            "✨ All installed developer tools and runtimes are up to date!"
                .green()
                .bold()
        );
        return Ok(());
    }

    println!("\n{}", "⬆ Found packages with updates available:".bold());
    println!("{}", "─".repeat(50).dimmed());
    for (def, status) in &outdated {
        if let ToolStatus::UpdateAvailable {
            installed,
            recommended,
        } = status
        {
            println!(
                "  • {:<16} (v{} -> v{})",
                def.id.bold(),
                installed.yellow(),
                recommended.green()
            );
        }
    }
    println!("{}", "─".repeat(50).dimmed());

    let tool_ids: Vec<String> = outdated.iter().map(|(d, _)| d.id.clone()).collect();

    if !args.all && !args.yes {
        let options: Vec<String> = outdated
            .iter()
            .map(|(d, s)| match s {
                ToolStatus::UpdateAvailable {
                    installed,
                    recommended,
                } => format!("{} (v{} -> v{})", d.display, installed, recommended),
                _ => d.display.clone(),
            })
            .collect();

        let selected = inquire::MultiSelect::new(
            "Select packages to update (Enter to confirm, Space to toggle):",
            options,
        )
        .prompt()
        .map_err(|e| format!("Selection error: {e}"))?;

        if selected.is_empty() {
            println!("No packages selected for update.");
            return Ok(());
        }

        let selected_ids: Vec<String> = outdated
            .into_iter()
            .filter(|(d, _)| selected.iter().any(|s| s.starts_with(&d.display)))
            .map(|(d, _)| d.id)
            .collect();

        return execute_tool_operation(OperationKind::Update, &selected_ids, false, false).await;
    }

    execute_tool_operation(OperationKind::Update, &tool_ids, args.yes, false).await
}

pub async fn execute_status(args: StatusArgs) -> Result<(), String> {
    let defs = load_definitions();
    let os = crate::modules::toolchain::platforms::current_platform().os_name();

    if let Some(tool_name) = args.tool {
        let norm_id = normalize_tool_id(&tool_name, &defs);
        let Some(def) = defs.iter().find(|d| d.id == norm_id) else {
            return Err(format!(
                "Tool '{}' not recognized. Use 'stkpil list tools' to see all available tools.",
                tool_name
            ));
        };

        let status = crate::modules::toolchain::core::discovery::detect_tool(def).await;

        if args.json {
            let info = serde_json::json!({
                "id": def.id,
                "display": def.display,
                "category": def.category,
                "status": format!("{:?}", status),
                "is_installed": status.is_ok(),
                "recommended_version": def.versions.recommended,
                "size_mb": def.size_mb,
            });
            println!("{}", serde_json::to_string_pretty(&info).unwrap());
            return Ok(());
        }

        println!("\n{}", format!("📦 Tool Status: {}", def.display).bold().cyan());
        println!("{}", "─".repeat(50).dimmed());
        println!("  ID:          {}", def.id.bold());
        println!("  Category:    {}", def.category);
        println!("  Package Size: ~{} MB", def.size_mb);

        match &status {
            ToolStatus::Installed { version } => {
                println!("  Status:      {} (v{})", "Installed".green().bold(), version.cyan());
            }
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            } => {
                println!(
                    "  Status:      {} (Installed: v{}, Recommended: v{})",
                    "Update Available".yellow().bold(),
                    installed.yellow(),
                    recommended.green()
                );
            }
            ToolStatus::Missing => {
                println!("  Status:      {}", "Missing / Not Installed".red().bold());
            }
            ToolStatus::PathBroken { reason } => {
                println!(
                    "  Status:      {} ({})",
                    "Broken".magenta().bold(),
                    reason
                );
            }
            ToolStatus::ManualInstall { reason } => {
                println!("  Status:      Manual installation ({})", reason);
            }
            ToolStatus::RunInDocker => {
                println!("  Status:      Managed by Docker Compose");
            }
        }

        if let Some(rec) = &def.versions.recommended {
            println!("  Recommended: v{}", rec);
        }

        let sources = crate::cli::tool_executor::tool_sources_for_os(def, &os);
        if !sources.is_empty() {
            println!("\n  Available Sources on {}:", os);
            for src in sources {
                println!("    • [{:?}] {}", src.kind, src.id);
            }
        }

        if let Some(notes) = &def.notes {
            println!("\n  Notes: {}", notes.dimmed());
        }

        println!();
        return Ok(());
    }

    // List summary status of tools
    let filter = args.category.as_deref();
    let scanned = scan_tools(filter).await;

    if args.json {
        let json_items: Vec<_> = scanned
            .iter()
            .map(|(def, status)| {
                serde_json::json!({
                    "id": def.id,
                    "display": def.display,
                    "category": def.category,
                    "status": format!("{:?}", status),
                    "installed": status.is_ok(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_items).unwrap());
        return Ok(());
    }

    println!("\n{}", "📊 Developer Environment Status:".bold().cyan());
    println!("{}", "─".repeat(60).dimmed());

    for (def, status) in scanned {
        let (icon, stat_str) = match status {
            ToolStatus::Installed { version } => ("✔".green(), format!("v{}", version).green()),
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            } => (
                "⬆".yellow(),
                format!("v{} -> v{}", installed, recommended).yellow(),
            ),
            ToolStatus::Missing => ("✖".red(), "Missing".red()),
            ToolStatus::PathBroken { .. } => ("⚠".magenta(), "Broken".magenta()),
            ToolStatus::ManualInstall { .. } => ("ℹ".dimmed(), "Manual".dimmed()),
            ToolStatus::RunInDocker => ("🐳".cyan(), "Docker".cyan()),
        };

        println!("  {} {:<28} {}", icon, def.display.bold(), stat_str);
    }
    println!();

    Ok(())
}
