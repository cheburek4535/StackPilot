use std::fs;
use std::path::{Path, PathBuf};

use super::models::EnvironmentBinding;

/// Manages directory structure and executable shims for an isolated environment.
pub struct EnvironmentFsManager;

impl EnvironmentFsManager {
    /// Ensure the environment sandbox directory structure exists and is populated
    /// with shims/proxies for all configured tools.
    pub fn ensure_sandbox(
        base_environments_dir: &Path,
        binding: &EnvironmentBinding,
    ) -> Result<PathBuf, String> {
        let env_dir = base_environments_dir.join(&binding.binding_id);
        let bin_dir = env_dir.join("bin");

        fs::create_dir_all(&bin_dir)
            .map_err(|e| format!("Failed to create environment sandbox dir: {}", e))?;

        // 1. Write environment metadata descriptor
        let meta_file = env_dir.join(".stackpilot-env.json");
        let meta_json = serde_json::to_string_pretty(binding)
            .map_err(|e| format!("Failed to serialize environment metadata: {}", e))?;
        fs::write(&meta_file, meta_json)
            .map_err(|e| format!("Failed to write environment metadata file: {}", e))?;

        // 2. Generate shims for all configured tools
        for (tool_id, tool) in &binding.tool_overrides {
            if let Some(ref exe_str) = tool.executable_path {
                let exe_path = Path::new(exe_str);
                if exe_path.is_file() {
                    Self::create_tool_shim(&bin_dir, tool_id, exe_path)?;
                }
            }
        }

        Ok(env_dir)
    }

    /// Create an executable shim in the environment's bin folder.
    ///
    /// On Windows: generates a `.cmd` batch shim that invokes the target binary.
    /// On Unix: generates a standard executable shell script wrapper.
    pub fn create_tool_shim(bin_dir: &Path, tool_id: &str, target_exe: &Path) -> Result<(), String> {
        let stem = target_exe
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(tool_id);

        let target_str = target_exe.to_string_lossy();

        #[cfg(target_os = "windows")]
        {
            // Windows: create a .cmd shim
            let shim_path = bin_dir.join(format!("{}.cmd", stem));
            let content = format!("@\"{}\" %*\r\n", target_str);
            fs::write(&shim_path, content)
                .map_err(|e| format!("Failed to create Windows shim {}: {}", shim_path.display(), e))?;

            // If the tool ID is different from the file stem (e.g. "nodejs" vs "node"),
            // also create a shim for the tool_id.
            if !tool_id.eq_ignore_ascii_case(stem) {
                let alias_path = bin_dir.join(format!("{}.cmd", tool_id));
                let alias_content = format!("@\"{}\" %*\r\n", target_str);
                let _ = fs::write(alias_path, alias_content);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let shim_path = bin_dir.join(stem);
            let content = format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", target_str);
            fs::write(&shim_path, content)
                .map_err(|e| format!("Failed to create Unix shim {}: {}", shim_path.display(), e))?;
            let mut perms = fs::metadata(&shim_path)
                .map_err(|e| format!("Failed to read metadata: {}", e))?
                .permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&shim_path, perms);
        }

        Ok(())
    }

    /// Remove an environment's sandbox directory and all its shims.
    pub fn remove_sandbox(base_environments_dir: &Path, binding_id: &str) -> Result<(), String> {
        let env_dir = base_environments_dir.join(binding_id);
        if env_dir.exists() {
            fs::remove_dir_all(&env_dir)
                .map_err(|e| format!("Failed to remove sandbox dir {}: {}", env_dir.display(), e))?;
        }
        Ok(())
    }

    /// Export standalone activation scripts into the target directory.
    /// Creates activate.bat, deactivate.bat, activate.ps1, and activate.sh.
    pub fn export_standalone(
        target_dir: &Path,
        binding: &EnvironmentBinding,
        overlay: &crate::platform::environment::EnvironmentOverlay,
    ) -> Result<Vec<String>, String> {
        fs::create_dir_all(target_dir)
            .map_err(|e| format!("Failed to create export target directory: {}", e))?;

        let env_name = binding.name.as_deref().unwrap_or(&binding.binding_id);
        let mut created = Vec::new();

        // 1. Windows activate.bat
        let win_path_prepend = overlay.path_prepend.join(";");
        let mut bat_content = format!(
            "@echo off\r\n\
            REM StackPilot Standalone Environment Activation\r\n\
            REM Environment: {name} ({id})\r\n\
            REM Works independently without StackPilot installed.\r\n\r\n\
            if not defined _SP_OLD_PATH set \"_SP_OLD_PATH=%PATH%\"\r\n\
            if not defined _SP_OLD_PROMPT set \"_SP_OLD_PROMPT=%PROMPT%\"\r\n\r\n",
            name = env_name,
            id = binding.binding_id
        );
        if !win_path_prepend.is_empty() {
            bat_content.push_str(&format!("set \"PATH={};%PATH%\"\r\n", win_path_prepend));
        }
        for (k, v) in &overlay.vars_set {
            bat_content.push_str(&format!("set \"{}={}\"\r\n", k, v));
        }
        for k in &overlay.vars_remove {
            bat_content.push_str(&format!("set \"{}=\"\r\n", k));
        }
        bat_content.push_str(&format!(
            "set \"_STACKPILOT_ENV={name}\"\r\n\
            prompt ({name}) $P$G\r\n\
            echo [StackPilot] Environment \"{name}\" activated.\r\n\
            echo Run \"call deactivate.bat\" or close this window to exit.\r\n",
            name = env_name
        ));

        let bat_path = target_dir.join("activate.bat");
        fs::write(&bat_path, bat_content)
            .map_err(|e| format!("Failed to write activate.bat: {}", e))?;
        created.push("activate.bat".to_string());

        // 2. Windows deactivate.bat
        let mut deact_bat = String::from(
            "@echo off\r\n\
            REM StackPilot Standalone Environment Deactivation\r\n\r\n\
            if defined _SP_OLD_PATH (\r\n\
                set \"PATH=%_SP_OLD_PATH%\"\r\n\
                set \"_SP_OLD_PATH=\"\r\n\
            )\r\n\
            if defined _SP_OLD_PROMPT (\r\n\
                prompt %_SP_OLD_PROMPT%\r\n\
                set \"_SP_OLD_PROMPT=\"\r\n\
            )\r\n"
        );
        for (k, _) in &overlay.vars_set {
            deact_bat.push_str(&format!("set \"{}=\"\r\n", k));
        }
        deact_bat.push_str(
            "set \"_STACKPILOT_ENV=\"\r\n\
            echo [StackPilot] Environment deactivated.\r\n"
        );
        let deact_bat_path = target_dir.join("deactivate.bat");
        fs::write(&deact_bat_path, deact_bat)
            .map_err(|e| format!("Failed to write deactivate.bat: {}", e))?;
        created.push("deactivate.bat".to_string());

        // 3. PowerShell activate.ps1
        let mut ps1_content = format!(
            "# StackPilot Standalone Environment Activation for PowerShell\r\n\
            # Environment: {name} ({id})\r\n\
            # Usage: . .\\activate.ps1\r\n\r\n\
            if (-not $env:_SP_OLD_PATH) {{\r\n\
                $env:_SP_OLD_PATH = $env:PATH\r\n\
            }}\r\n",
            name = env_name,
            id = binding.binding_id
        );
        if !win_path_prepend.is_empty() {
            ps1_content.push_str(&format!(
                "$env:PATH = \"{}\" + [System.IO.Path]::PathSeparator + $env:PATH\r\n",
                win_path_prepend
            ));
        }
        for (k, v) in &overlay.vars_set {
            ps1_content.push_str(&format!("$env:{} = \"{}\"\r\n", k, v.replace('"', "`\"")));
        }
        for k in &overlay.vars_remove {
            ps1_content.push_str(&format!("Remove-Item env:{} -ErrorAction SilentlyContinue\r\n", k));
        }
        ps1_content.push_str(&format!(
            "$env:_STACKPILOT_ENV = \"{name}\"\r\n\r\n\
            function global:deactivate {{\r\n\
                if ($env:_SP_OLD_PATH) {{\r\n\
                    $env:PATH = $env:_SP_OLD_PATH\r\n\
                    Remove-Item env:_SP_OLD_PATH -ErrorAction SilentlyContinue\r\n\
                }}\r\n",
            name = env_name
        ));
        for (k, _) in &overlay.vars_set {
            ps1_content.push_str(&format!("    Remove-Item env:{} -ErrorAction SilentlyContinue\r\n", k));
        }
        ps1_content.push_str(
            "    Remove-Item env:_STACKPILOT_ENV -ErrorAction SilentlyContinue\r\n\
                Remove-Item function:deactivate -ErrorAction SilentlyContinue\r\n\
                Write-Host \"[StackPilot] Environment deactivated.\" -ForegroundColor Yellow\r\n\
            }\r\n\r\n"
        );
        ps1_content.push_str(&format!(
            "Write-Host \"[StackPilot] Environment '{name}' activated.\" -ForegroundColor Green\r\n\
            Write-Host \"Run 'deactivate' to exit the environment.\" -ForegroundColor Gray\r\n",
            name = env_name
        ));
        let ps1_path = target_dir.join("activate.ps1");
        fs::write(&ps1_path, ps1_content)
            .map_err(|e| format!("Failed to write activate.ps1: {}", e))?;
        created.push("activate.ps1".to_string());

        // 4. POSIX activate.sh (Linux, macOS, Git Bash, WSL)
        let unix_path_prepend = overlay.path_prepend.iter()
            .map(|p| p.replace('\\', "/"))
            .collect::<Vec<_>>()
            .join(":");
        let mut sh_content = format!(
            "#!/usr/bin/env sh\n\
            # StackPilot Standalone Environment Activation (POSIX sh/bash/zsh)\n\
            # Environment: {name} ({id})\n\
            # Usage: source ./activate.sh\n\n\
            if [ -z \"$_SP_OLD_PATH\" ]; then\n\
                export _SP_OLD_PATH=\"$PATH\"\n\
            fi\n\
            if [ -z \"$_SP_OLD_PS1\" ]; then\n\
                export _SP_OLD_PS1=\"$PS1\"\n\
            fi\n\n",
            name = env_name,
            id = binding.binding_id
        );
        if !unix_path_prepend.is_empty() {
            sh_content.push_str(&format!("export PATH=\"{}:$PATH\"\n", unix_path_prepend));
        }
        for (k, v) in &overlay.vars_set {
            sh_content.push_str(&format!("export {}=\"{}\"\n", k, v.replace('"', "\\\"")));
        }
        for k in &overlay.vars_remove {
            sh_content.push_str(&format!("unset {}\n", k));
        }
        sh_content.push_str(&format!(
            "export _STACKPILOT_ENV=\"{name}\"\n\
            export PS1=\"({name}) $PS1\"\n\n\
            deactivate() {{\n\
                if [ -n \"$_SP_OLD_PATH\" ]; then\n\
                    export PATH=\"$_SP_OLD_PATH\"\n\
                    unset _SP_OLD_PATH\n\
                fi\n\
                if [ -n \"$_SP_OLD_PS1\" ]; then\n\
                    export PS1=\"$_SP_OLD_PS1\"\n\
                    unset _SP_OLD_PS1\n\
                fi\n",
            name = env_name
        ));
        for (k, _) in &overlay.vars_set {
            sh_content.push_str(&format!("    unset {}\n", k));
        }
        sh_content.push_str(
            "    unset _STACKPILOT_ENV\n\
                unset -f deactivate\n\
                echo \"[StackPilot] Environment deactivated.\"\n\
            }\n\n"
        );
        sh_content.push_str(&format!(
            "echo \"[StackPilot] Environment \\\"{name}\\\" activated.\"\n\
            echo \"Run 'deactivate' to exit the environment.\"\n",
            name = env_name
        ));
        let sh_path = target_dir.join("activate.sh");
        fs::write(&sh_path, sh_content)
            .map_err(|e| format!("Failed to write activate.sh: {}", e))?;

        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&sh_path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&sh_path, perms);
            }
        }
        created.push("activate.sh".to_string());

        Ok(created)
    }
}
