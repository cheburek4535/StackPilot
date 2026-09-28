use colored::Colorize;

use crate::cli::args::AnalyzeArgs;
use crate::modules::devlauncher::analyzer::{AnalyzeOptions, FsProjectAnalyzer, ProjectAnalyzerV2};
use crate::modules::devlauncher::models::StepKind;

pub fn execute(args: AnalyzeArgs) -> Result<(), String> {
    let project_path = args.path.canonicalize().unwrap_or(args.path);
    let path_str = project_path.to_string_lossy().to_string();

    if !project_path.exists() {
        return Err(format!("Directory '{}' does not exist", path_str));
    }

    let analyzer = FsProjectAnalyzer;
    let draft = analyzer
        .analyze_draft(&path_str, &AnalyzeOptions::default())
        .map_err(|e| format!("Analysis failed: {e}"))?;

    let profile = draft.profile;

    if args.json {
        let json_val = serde_json::json!({
            "project_root": path_str,
            "profile_name": profile.name,
            "description": profile.description,
            "steps": profile.steps,
            "diagnostics": draft.diagnostics,
        });
        println!("{}", serde_json::to_string_pretty(&json_val).unwrap());
        return Ok(());
    }

    println!("\n{}", "🔍 StackPilot Project Stack Analysis".bold().cyan());
    println!("{}", "─".repeat(60).dimmed());
    println!("  Path:        {}", path_str.bold());
    println!("  Name:        {}", profile.name.bold());
    println!("  Description: {}", profile.description.dimmed());
    println!();

    // Check project manifests
    println!("{}", "📦 Detected Manifests & Indicators:".bold());
    let mut manifests_found = false;
    let common_manifests = [
        ("package.json", "Node.js / JavaScript / TypeScript"),
        ("Cargo.toml", "Rust (Cargo)"),
        ("docker-compose.yml", "Docker Compose"),
        ("docker-compose.yaml", "Docker Compose"),
        ("compose.yaml", "Docker Compose"),
        ("pyproject.toml", "Python (Modern PEP 518/621)"),
        ("requirements.txt", "Python (pip)"),
        ("Pipfile", "Python (Pipenv)"),
        ("go.mod", "Go Module"),
        ("pom.xml", "Java (Maven)"),
        ("build.gradle", "Java / Kotlin (Gradle)"),
        (".git", "Git Repository"),
    ];

    for (file, label) in common_manifests {
        if project_path.join(file).exists() {
            println!("  {} {:<22} ({})", "✔".green(), file.bold(), label);
            manifests_found = true;
        }
    }
    if !manifests_found {
        println!("  {}", "No standard manifest files detected in root.".dimmed());
    }
    println!();

    // Discovered services and commands
    println!("{}", "🚀 Discovered Run Configurations & Steps:".bold());
    if profile.steps.is_empty() {
        println!("  {}", "No automatic launch steps generated.".dimmed());
    } else {
        for (i, step) in profile.steps.iter().enumerate() {
            let kind_desc = match &step.kind {
                StepKind::RunCommand { command, .. } => format!("command: {}", command.cyan()),
                StepKind::RunScript { script, .. } => format!("script: {}", script.cyan()),
                StepKind::WaitForPort { host, port, .. } => {
                    format!("wait for port: {}:{}", host, port.to_string().yellow())
                }
                StepKind::WaitForUrl { url, .. } => format!("wait for url: {}", url),
                StepKind::WaitForDocker {} => "wait for docker daemon".to_string(),
                StepKind::Delay { seconds } => format!("delay {}s", seconds),
                StepKind::OpenUrl { url } => format!("open url: {}", url.blue().underline()),
                StepKind::OpenApplication { path, .. } => format!("open app: {}", path),
                StepKind::OpenTerminal { .. } => "open terminal".to_string(),
                StepKind::OpenFolder { path } => format!("open folder: {}", path),
            };

            let dir_info = if let Some(dir) = &step.working_directory {
                format!(" [cwd: {}]", dir.dimmed())
            } else {
                String::new()
            };

            let enabled_str = if step.enabled {
                "✔".green()
            } else {
                "✖".dimmed()
            };

            println!(
                "  [{}] {} {:<24} {}{}",
                i + 1,
                enabled_str,
                step.label.bold(),
                kind_desc,
                dir_info
            );
        }
    }
    println!();

    // Diagnostics / recommendations
    if !draft.diagnostics.is_empty() {
        println!("{}", "💡 Diagnostics & Insights:".bold());
        for diag in &draft.diagnostics {
            let prefix = match diag.severity {
                crate::modules::devlauncher::models::DiagnosticSeverity::Error => "✖".red(),
                crate::modules::devlauncher::models::DiagnosticSeverity::Warning => "⚠".yellow(),
                crate::modules::devlauncher::models::DiagnosticSeverity::Info => "ℹ".blue(),
            };
            println!("  {} {}", prefix, diag.message);
        }
        println!();
    }

    println!(
        "{} Run {} to start these development services.\n",
        "💡 Tip:".yellow().bold(),
        "stkpil launch".bold().green()
    );

    Ok(())
}
