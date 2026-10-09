use std::env;
use std::path::PathBuf;
use colored::Colorize;

use crate::cli::args::{EnvArgs, EnvBindArgs, EnvExportArgs, EnvRunArgs, EnvShellArgs, EnvSubcommands};
use crate::modules::project_environment::models::EnvironmentBinding;
use crate::modules::project_environment::resolver::resolve_with_diagnostics;
use crate::modules::project_environment::service::{
    EnvironmentBindingService, JsonEnvironmentBindingService,
};
use crate::platform::paths::get_app_data_dir;

pub async fn execute(args: EnvArgs) -> Result<(), String> {
    let app_data_dir = get_app_data_dir();
    let envs_dir = app_data_dir.join("project_environments");
    let service = JsonEnvironmentBindingService::new(envs_dir);

    match args.action {
        EnvSubcommands::List => execute_list(&service),
        EnvSubcommands::Run(run_args) => execute_run(&service, run_args),
        EnvSubcommands::Shell(shell_args) => execute_shell(&service, shell_args),
        EnvSubcommands::Export(export_args) => execute_export(&service, export_args),
        EnvSubcommands::Bind(bind_args) => execute_bind(&service, bind_args),
    }
}

fn execute_list(service: &dyn EnvironmentBindingService) -> Result<(), String> {
    let bindings = service.list()?;

    println!("\n{}", "📦 StackPilot Project Environments".bold().cyan());
    println!("{}", "─".repeat(70).dimmed());

    if bindings.is_empty() {
        println!("  No environments found. Use {} to create one or run the app UI.", "stkpil create".yellow());
        println!();
        return Ok(());
    }

    for b in &bindings {
        let name = b.name.as_deref().unwrap_or(&b.binding_id);
        let mode_str = if b.is_default {
            "Host Global (Default)".cyan()
        } else if b.is_isolated() {
            "Isolated Sandbox".yellow()
        } else {
            "Custom Global".blue()
        };

        println!(
            "  {} {} [{}]",
            "•".cyan(),
            name.bold(),
            mode_str
        );
        println!("    ID:           {}", b.binding_id.dimmed());
        if let Some(ref desc) = b.description {
            println!("    Description:  {}", desc.dimmed());
        }

        if !b.tool_overrides.is_empty() {
            let tools_str = b
                .tool_overrides
                .iter()
                .map(|(k, t)| {
                    if let Some(ref ver) = t.version {
                        format!("{}@{}", k, ver)
                    } else {
                        k.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("    Tools:        {}", tools_str);
        }

        if !b.bound_projects.is_empty() {
            println!(
                "    Projects ({}): {}",
                b.bound_projects.len(),
                b.bound_projects.join(", ").dimmed()
            );
        }
        println!();
    }

    Ok(())
}

fn resolve_target_binding(
    service: &dyn EnvironmentBindingService,
    specified_env: Option<&str>,
) -> Result<EnvironmentBinding, String> {
    if let Some(identifier) = specified_env {
        // Try exact ID match first
        if let Ok(b) = service.get(identifier) {
            return Ok(b);
        }
        // Try name match (case-insensitive)
        let all = service.list()?;
        for b in all {
            if b.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(identifier)) {
                return Ok(b);
            }
        }
        return Err(format!("Environment '{}' not found", identifier));
    }

    // Try finding environment for current directory
    let current_dir = env::current_dir()
        .map_err(|e| format!("Cannot get current directory: {}", e))?;
    let path_str = current_dir.to_string_lossy();

    if let Ok(Some(b)) = service.find_by_project_path(&path_str) {
        return Ok(b);
    }

    // Fallback to default environment
    service.get_or_create_default()
}

fn execute_run(
    service: &dyn EnvironmentBindingService,
    args: EnvRunArgs,
) -> Result<(), String> {
    let binding = resolve_target_binding(service, args.env.as_deref())?;
    let env_name = binding.name.as_deref().unwrap_or(&binding.binding_id);

    eprintln!(
        "{} Running in environment «{}»...",
        "⚡".yellow(),
        env_name.bold()
    );

    let (overlay, _) = resolve_with_diagnostics(&binding);

    let mut cmd = std::process::Command::new(&args.command);
    cmd.args(&args.args);

    // Apply PATH prepends
    if !overlay.path_prepend.is_empty() {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<PathBuf> = overlay.path_prepend.iter().map(PathBuf::from).collect();
        for p in env::split_paths(&current_path) {
            paths.push(p);
        }
        if let Ok(new_path) = env::join_paths(paths) {
            cmd.env("PATH", new_path);
        }
    }

    // Apply vars_set
    for (k, v) in &overlay.vars_set {
        cmd.env(k, v);
    }

    // Apply vars_remove
    for k in &overlay.vars_remove {
        cmd.env_remove(k);
    }

    cmd.env("_STACKPILOT_ENV", env_name);

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| format!("Failed to spawn command '{}': {}", args.command, e))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn execute_shell(
    service: &dyn EnvironmentBindingService,
    args: EnvShellArgs,
) -> Result<(), String> {
    let binding = resolve_target_binding(service, args.env.as_deref())?;
    let env_name = binding.name.as_deref().unwrap_or(&binding.binding_id);

    eprintln!(
        "\n{} Entering interactive shell for environment «{}»",
        "🚀".cyan(),
        env_name.bold()
    );
    eprintln!("  Type 'exit' to leave this environment.\n");

    let (overlay, _) = resolve_with_diagnostics(&binding);

    #[cfg(target_os = "windows")]
    let shell_exe = env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    #[cfg(not(target_os = "windows"))]
    let shell_exe = env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".to_string()
        } else {
            "/bin/bash".to_string()
        }
    });

    let mut cmd = std::process::Command::new(&shell_exe);

    // Apply PATH prepends
    if !overlay.path_prepend.is_empty() {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<PathBuf> = overlay.path_prepend.iter().map(PathBuf::from).collect();
        for p in env::split_paths(&current_path) {
            paths.push(p);
        }
        if let Ok(new_path) = env::join_paths(paths) {
            cmd.env("PATH", new_path);
        }
    }

    // Apply vars_set
    for (k, v) in &overlay.vars_set {
        cmd.env(k, v);
    }

    // Apply vars_remove
    for k in &overlay.vars_remove {
        cmd.env_remove(k);
    }

    cmd.env("_STACKPILOT_ENV", env_name);

    #[cfg(target_os = "windows")]
    {
        let prompt = format!("({}) $P$G", env_name);
        cmd.env("PROMPT", prompt);
    }

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|e| format!("Failed to spawn shell '{}': {}", shell_exe, e))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn execute_export(
    service: &dyn EnvironmentBindingService,
    args: EnvExportArgs,
) -> Result<(), String> {
    let binding = resolve_target_binding(service, args.env.as_deref())?;
    let target_dir = match args.dir {
        Some(d) => d,
        None => env::current_dir().map_err(|e| format!("Cannot get current directory: {}", e))?,
    };

    let target_str = target_dir.to_string_lossy().to_string();
    let res = service.export_standalone(&binding.binding_id, Some(&target_str))?;

    println!("\n{}", "📦 Standalone Environment Scripts Exported".bold().green());
    println!("{}", "─".repeat(70).dimmed());
    println!("  Environment:   {}", binding.name.as_deref().unwrap_or(&binding.binding_id).bold());
    println!("  Target folder: {}", res.target_dir.cyan());
    println!("  Created files:");
    for f in &res.created_files {
        println!("    • {}", f.bold());
    }
    println!();
    println!("  Usage without StackPilot:");
    println!("    Windows cmd:        call activate.bat");
    println!("    Windows PowerShell: . .\\activate.ps1");
    println!("    POSIX / Linux / Mac: source ./activate.sh");
    println!();

    Ok(())
}

fn execute_bind(
    service: &dyn EnvironmentBindingService,
    args: EnvBindArgs,
) -> Result<(), String> {
    let binding = resolve_target_binding(service, Some(&args.env))?;
    let target_dir = match args.path {
        Some(p) => p.canonicalize().unwrap_or(p),
        None => env::current_dir().map_err(|e| format!("Cannot get current directory: {}", e))?,
    };

    let target_str = target_dir.to_string_lossy().to_string();
    let updated = service.bind_project(&binding.binding_id, &target_str)?;

    println!(
        "\n{} Bound project '{}' to environment '{}' [{}]\n",
        "✓".green(),
        target_str.cyan(),
        updated.name.as_deref().unwrap_or(&updated.binding_id).bold(),
        updated.binding_id.dimmed()
    );

    Ok(())
}
