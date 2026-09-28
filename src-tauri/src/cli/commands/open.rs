use colored::Colorize;
use std::path::Path;

use crate::cli::args::OpenArgs;

pub fn execute(args: OpenArgs) -> Result<(), String> {
    let target_path = if args.path.as_os_str().is_empty() || args.path.as_os_str() == "." {
        std::env::current_dir().unwrap_or(args.path)
    } else {
        std::fs::canonicalize(&args.path).unwrap_or(args.path)
    };

    if !target_path.exists() {
        return Err(format!("Path does not exist: {}", target_path.display()));
    }

    println!(
        "Opening {} in {}...",
        target_path.display().to_string().cyan(),
        args.editor.bold()
    );

    open_in_ide(&target_path, &args.editor)
}

pub fn open_in_ide(path: &Path, ide_cli: &str) -> Result<(), String> {
    let path_str = path.to_string_lossy().to_string();
    let resolved = crate::platform::ide::resolve_ide_executable(ide_cli).ok_or_else(|| {
        format!(
            "Editor/CLI '{}' was not found on this system. Make sure it is installed and available in PATH.",
            ide_cli
        )
    })?;

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if crate::platform::paths::is_batch_file(&resolved) {
            let args = crate::platform::command::batch_shim_cmd_line(&resolved, &[&path_str]);
            let mut cmd = Command::new("cmd");
            cmd.args(args);
            cmd.spawn().map_err(|e| e.to_string())?;
        } else {
            Command::new(&resolved)
                .arg(&path_str)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::process::Command;
        let (program, mut launcher_args) =
            crate::platform::app_launcher::split_launcher_string(&resolved);
        launcher_args.push(path_str);
        Command::new(&program)
            .args(&launcher_args)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    println!("{}", "✔ IDE launched successfully.".green());
    Ok(())
}
