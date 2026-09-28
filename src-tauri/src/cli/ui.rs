use colored::Colorize;
use std::path::Path;

use crate::modules::project_creator::models::WizardContext;
use crate::modules::project_creator::validate::{StackIssue, StackSeverity};

pub fn print_banner() {
    println!(
        "\n{}\n",
        "  ⚡ StackPilot CLI  —  Project Scaffold & Workspace Engine"
            .bold()
            .cyan()
    );
}

pub fn print_project_summary(
    ctx: &WizardContext,
    dest_path: &Path,
    ide_to_open: Option<&str>,
    dry_run: bool,
) {
    println!("{}", "╭─ Project Summary ──────────────────────────────────────────".dimmed());
    
    let name = ctx.project_name.as_deref().unwrap_or("<unnamed>");
    println!("│  {}: {}", "Name".bold(), name.bright_green());
    println!("│  {}: {}", "Target".bold(), dest_path.display().to_string().bright_blue());
    
    if let Some(ref pt) = ctx.project_type {
        println!("│  {}: {}", "Type".bold(), pt.bright_cyan());
    }
    
    if !ctx.backend_languages.is_empty() || !ctx.frameworks.is_empty() {
        let be_fw: Vec<_> = ctx.frameworks.iter().filter(|f| is_backend_or_side(f)).cloned().collect();
        let be_desc = if be_fw.is_empty() {
            ctx.backend_languages.join(", ")
        } else {
            format!("{} ({})", be_fw.join(", "), ctx.backend_languages.join(", "))
        };
        if !be_desc.is_empty() {
            println!("│  {}: {}", "Backend".bold(), be_desc.yellow());
        }
    }

    if !ctx.frontend_languages.is_empty() || !ctx.frameworks.is_empty() {
        let fe_fw: Vec<_> = ctx.frameworks.iter().filter(|f| !is_backend_or_side(f)).cloned().collect();
        let fe_desc = if fe_fw.is_empty() {
            ctx.frontend_languages.join(", ")
        } else {
            format!("{} ({})", fe_fw.join(", "), ctx.frontend_languages.join(", "))
        };
        if !fe_desc.is_empty() {
            println!("│  {}: {}", "Frontend".bold(), fe_desc.magenta());
        }
    }

    if !ctx.tools.is_empty() {
        println!("│  {}: {}", "Tools/Infra".bold(), ctx.tools.join(", ").bright_white());
    }

    println!("│  {}: {}", "Git Init".bold(), if ctx.git_init { "Yes".green() } else { "No".dimmed() });
    println!("│  {}: {}", "Docker".bold(), if ctx.docker { "Yes".green() } else { "No".dimmed() });

    if let Some(ide) = ide_to_open {
        println!("│  {}: {}", "Open in IDE".bold(), ide.bright_yellow());
    }

    if dry_run {
        println!("│  {}", "[DRY RUN MODE: No files will be written to disk]".bold().yellow());
    }

    println!("{}\n", "╰────────────────────────────────────────────────────────────".dimmed());
}

fn is_backend_or_side(fw: &str) -> bool {
    matches!(
        fw,
        "django"
            | "fastapi"
            | "flask"
            | "actix"
            | "actix-web"
            | "axum"
            | "rocket"
            | "nest"
            | "nestjs"
            | "express"
            | "koa"
            | "gin"
            | "fiber"
            | "echo"
            | "spring-boot"
            | "laravel"
            | "aspnet-core"
            | "phoenix"
            | "aiogram"
            | "telegraf"
            | "zig-cli"
    )
}

pub fn print_issues(issues: &[StackIssue]) {
    for issue in issues {
        match issue.severity {
            StackSeverity::Error => {
                eprintln!("  {} {}", "✖ ERROR:".bold().red(), issue.message);
            }
            StackSeverity::Warning => {
                println!("  {} {}", "⚠ WARNING:".bold().yellow(), issue.message);
            }
        }
    }
}

pub fn print_success(project_name: &str, dest_path: &Path) {
    println!(
        "\n{} {}",
        "🎉 Project scaffolded successfully:".bold().green(),
        project_name.bold().bright_white()
    );
    println!("   {} {}", "Location:".dimmed(), dest_path.display().to_string().cyan());
    println!("\n{}", "Next steps:".bold());
    println!("   {} {}", "$".dimmed(), format!("cd {}", dest_path.display()).bright_white());
    println!("   {} {}\n", "$".dimmed(), "code .".bright_white());
}
