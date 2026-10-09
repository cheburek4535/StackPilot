use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use regex::Regex;

use crate::modules::project_environment::models::ToolOverride;
use crate::platform::command_resolver::resolve_executable;

/// Resolves an installed tool by its ID (e.g. "node", "python", "cargo", "git").
/// Returns a `ToolOverride` with the absolute executable path and detected version.
pub fn resolve_tool(tool_id: &str) -> Option<ToolOverride> {
    let normalized_id = tool_id.trim().to_lowercase();
    if normalized_id.is_empty() {
        return None;
    }

    // 1. Direct candidate executable names
    let candidates = get_candidates_for_tool(&normalized_id);

    for candidate in &candidates {
        if let Some(path) = resolve_executable(candidate, None) {
            let version = detect_tool_version(&path, &normalized_id);
            return Some(ToolOverride {
                executable_path: Some(path.to_string_lossy().into_owned()),
                version,
                path_entries: Vec::new(),
                env_vars: HashMap::new(),
            });
        }
    }

    // 2. Fallback: check Toolchain definitions if available
    let defs = crate::modules::toolchain::defs::load_definitions();
    if let Some(def) = defs.iter().find(|d| {
        d.id.eq_ignore_ascii_case(&normalized_id)
            || d.extended.aliases.iter().any(|a| a.eq_ignore_ascii_case(&normalized_id))
    }) {
        // Try version probe first elements
        for probe in &def.detection.version_probes {
            if let Some(bin) = probe.first() {
                if let Some(path) = resolve_executable(bin, None) {
                    let version = detect_tool_version(&path, &normalized_id);
                    return Some(ToolOverride {
                        executable_path: Some(path.to_string_lossy().into_owned()),
                        version,
                        path_entries: Vec::new(),
                        env_vars: HashMap::new(),
                    });
                }
            }
        }

        // Try known paths
        for kp in &def.detection.known_paths {
            let expanded = expand_env_path(kp);
            let p = Path::new(&expanded);
            if p.is_file() {
                let version = detect_tool_version(p, &normalized_id);
                return Some(ToolOverride {
                    executable_path: Some(p.to_string_lossy().into_owned()),
                    version,
                    path_entries: Vec::new(),
                    env_vars: HashMap::new(),
                });
            }
        }
    }

    None
}

/// Resolve a list of tool IDs into a map of `ToolOverride`s.
/// Automatically resolves bundled dependencies (e.g., node -> npm, python -> pip, rust -> cargo).
pub fn resolve_tools_for_environment(tool_ids: &[String]) -> HashMap<String, ToolOverride> {
    let mut result: HashMap<String, ToolOverride> = HashMap::new();

    for id in tool_ids {
        if let Some(t_override) = resolve_tool(id) {
            result.insert(id.clone(), t_override);
        }
    }

    // Automatically check bundled companions
    if (result.contains_key("node") || result.contains_key("nodejs")) && !result.contains_key("npm") {
        if let Some(npm_override) = resolve_tool("npm") {
            result.insert("npm".to_string(), npm_override);
        }
    }

    if (result.contains_key("python") || result.contains_key("python3")) && !result.contains_key("pip") {
        if let Some(pip_override) = resolve_tool("pip") {
            result.insert("pip".to_string(), pip_override);
        }
    }

    if (result.contains_key("rust") || result.contains_key("cargo")) && !result.contains_key("cargo") {
        if let Some(cargo_override) = resolve_tool("cargo") {
            result.insert("cargo".to_string(), cargo_override);
        }
    }

    result
}

/// Return primary binary names to check for a given tool identifier.
fn get_candidates_for_tool(tool_id: &str) -> Vec<&'static str> {
    match tool_id {
        "node" | "nodejs" => vec!["node"],
        "npm" => vec!["npm"],
        "pnpm" => vec!["pnpm"],
        "yarn" => vec!["yarn"],
        "bun" => vec!["bun"],
        "deno" => vec!["deno"],
        "python" | "python3" => vec!["python", "python3", "py"],
        "pip" | "pip3" => vec!["pip", "pip3"],
        "rust" | "cargo" | "rustc" => vec!["cargo", "rustc"],
        "go" | "golang" => vec!["go"],
        "git" => vec!["git"],
        "docker" => vec!["docker"],
        "dotnet" => vec!["dotnet"],
        "java" | "jdk" => vec!["java"],
        "php" => vec!["php"],
        "composer" => vec!["composer"],
        "ruby" => vec!["ruby"],
        "gem" => vec!["gem"],
        "flutter" => vec!["flutter"],
        "dart" => vec!["dart"],
        "gradle" => vec!["gradle"],
        "maven" | "mvn" => vec!["mvn"],
        _ => vec![],
    }
}

/// Detect tool version by executing probe commands.
fn detect_tool_version(exe_path: &Path, tool_id: &str) -> Option<String> {
    let args_to_try: Vec<&[&str]> = if tool_id == "go" || tool_id == "golang" {
        vec![&["version"], &["--version"], &["-v"]]
    } else {
        vec![&["--version"], &["-v"], &["version"]]
    };

    for args in args_to_try {
        let mut cmd = Command::new(exe_path);
        cmd.args(args);
        cmd.stdin(std::process::Stdio::null());

        #[cfg(target_os = "windows")]
        {
            // CREATE_NO_WINDOW = 0x08000000 to avoid console flash
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let text = if !stdout.trim().is_empty() {
                    stdout.trim()
                } else {
                    stderr.trim()
                };

                if let Some(v) = extract_version_from_text(text) {
                    return Some(v);
                }
            }
        }
    }

    None
}

/// Extract clean semver version string (e.g. "20.11.0", "3.12.2") from tool output.
fn extract_version_from_text(text: &str) -> Option<String> {
    let re = Regex::new(r"(\d+\.\d+(?:\.\d+)?(?:-[0-9a-zA-Z.]+)?|\d+)").ok()?;
    if let Some(cap) = re.captures(text) {
        if let Some(m) = cap.get(1) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Expand environment variables like %LOCALAPPDATA% or $HOME in a path string.
fn expand_env_path(path: &str) -> String {
    let mut result = path.to_string();
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("LOCALAPPDATA") {
            result = result.replace("%LOCALAPPDATA%", &appdata);
        }
        if let Ok(prog) = std::env::var("ProgramFiles") {
            result = result.replace("%ProgramFiles%", &prog);
        }
        if let Ok(userprof) = std::env::var("USERPROFILE") {
            result = result.replace("%USERPROFILE%", &userprof);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if result.starts_with("~/") || result.starts_with("~\\") {
            result = format!("{}{}", home, &result[1..]);
        }
        result = result.replace("$HOME", &home);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_version_standard() {
        assert_eq!(extract_version_from_text("v20.11.1"), Some("20.11.1".to_string()));
        assert_eq!(extract_version_from_text("Python 3.12.2"), Some("3.12.2".to_string()));
        assert_eq!(extract_version_from_text("cargo 1.77.0 (aedd2179d 2024-03-14)"), Some("1.77.0".to_string()));
        assert_eq!(extract_version_from_text("go version go1.22.1 windows/amd64"), Some("1.22.1".to_string()));
    }

    #[test]
    fn get_candidates() {
        assert_eq!(get_candidates_for_tool("node"), vec!["node"]);
        assert!(get_candidates_for_tool("python").contains(&"python"));
        assert!(get_candidates_for_tool("unknown_tool_xyz").is_empty());
    }
}
