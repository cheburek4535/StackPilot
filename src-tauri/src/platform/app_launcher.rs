//! Native application resolver and launcher.
//!
//! Resolves application names (VS Code, JetBrains IDEs, Docker Desktop,
//! etc.) to platform-specific executable paths and launches them.
//!
//! # Flatpak handling
//!
//! A value such as `"flatpak run <id>"` is never treated as a literal
//! executable path containing spaces. Instead it is resolved as a
//! structured launcher with `program = "flatpak"` and
//! `args = ["run", "<id>"]`.

use std::path::{Path, PathBuf};

use super::command_resolver::{resolve_executable, PathOverlay};
use super::paths::is_batch_file;

// ---------------------------------------------------------------------------
// Structured application launcher
// ---------------------------------------------------------------------------

/// A fully resolved application launcher: the program and arguments
/// needed to launch the application. This is the output of the
/// resolution process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationLauncher {
    /// The program to spawn (absolute path or resolvable name).
    pub program: String,
    /// Arguments to pass after the program.
    pub args: Vec<String>,
    /// Whether this launcher represents a flatpak command.
    pub is_flatpak: bool,
    /// Whether the application was found on this system.
    pub found: bool,
    /// Diagnostic messages from resolution.
    pub diagnostics: Vec<String>,
}

/// Policy for how an application should be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPolicy {
    /// Reuse an existing application instance if possible.
    ReuseExisting,
    /// Always open a new window/instance.
    AlwaysNew,
    /// Open with a specific project path.
    OpenProject,
    /// Open a specific file or folder.
    OpenFileOrFolder,
    /// Launch detached (fire-and-forget, no output tracking).
    Detached,
}

impl Default for LaunchPolicy {
    fn default() -> Self {
        Self::Detached
    }
}

// ---------------------------------------------------------------------------
// Application registry — known applications and their resolution hints
// ---------------------------------------------------------------------------

/// Known application identifiers and their platform-specific resolution.
struct KnownApp {
    name: &'static str,
    cli_names: &'static [&'static str],
    #[cfg(target_os = "windows")]
    registry_names: &'static [&'static str],
    #[cfg(target_os = "macos")]
    app_bundles: &'static [(&'static str, &'static str)],
    #[cfg(target_os = "linux")]
    flatpak_ids: &'static [(&'static str, &'static str)],
}

/// Well-known applications and their CLI names.
fn known_applications() -> Vec<KnownApp> {
    let mut apps = Vec::new();

    // VS Code family
    apps.push(KnownApp {
        name: "VS Code",
        cli_names: &["code", "code-insiders"],
        #[cfg(target_os = "windows")]
        registry_names: &["Code.exe", "Code - Insiders.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[
            ("Visual Studio Code.app", "Contents/Resources/app/bin/code"),
            (
                "Visual Studio Code - Insiders.app",
                "Contents/Resources/app/bin/code",
            ),
        ],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[
            ("code", "com.visualstudio.code"),
            ("code-insiders", "com.visualstudio.code.insiders"),
        ],
    });

    // Cursor
    apps.push(KnownApp {
        name: "Cursor",
        cli_names: &["cursor"],
        #[cfg(target_os = "windows")]
        registry_names: &["Cursor.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Cursor.app", "Contents/MacOS/Cursor")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("cursor", "com.todesktop.230313mzl4w4u92")],
    });

    // Windsurf
    apps.push(KnownApp {
        name: "Windsurf",
        cli_names: &["windsurf"],
        #[cfg(target_os = "windows")]
        registry_names: &["Windsurf.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Windsurf.app", "Contents/MacOS/windsurf")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });

    // PyCharm
    apps.push(KnownApp {
        name: "PyCharm",
        cli_names: &["pycharm", "pycharm64", "pycharm-ce", "pycharm-ce64"],
        #[cfg(target_os = "windows")]
        registry_names: &["pycharm64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[
            ("PyCharm.app", "Contents/MacOS/pycharm"),
            ("PyCharm CE.app", "Contents/MacOS/pycharm"),
        ],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[
            ("pycharm", "com.jetbrains.PyCharm-Community"),
            ("pycharm-ce", "com.jetbrains.PyCharm-Community"),
        ],
    });

    // GoLand
    apps.push(KnownApp {
        name: "GoLand",
        cli_names: &["goland", "goland64"],
        #[cfg(target_os = "windows")]
        registry_names: &["goland64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("GoLand.app", "Contents/MacOS/goland")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("goland", "com.jetbrains.GoLand")],
    });

    // IntelliJ IDEA
    apps.push(KnownApp {
        name: "IntelliJ IDEA",
        cli_names: &["idea", "idea64", "idea-ce", "idea-ce64"],
        #[cfg(target_os = "windows")]
        registry_names: &["idea64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[
            ("IntelliJ IDEA.app", "Contents/MacOS/idea"),
            ("IntelliJ IDEA CE.app", "Contents/MacOS/idea"),
        ],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[
            ("idea", "com.jetbrains.IntelliJ-IDEA-Community"),
            ("idea-ce", "com.jetbrains.IntelliJ-IDEA-Community"),
        ],
    });

    // WebStorm
    apps.push(KnownApp {
        name: "WebStorm",
        cli_names: &["webstorm", "webstorm64"],
        #[cfg(target_os = "windows")]
        registry_names: &["webstorm64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("WebStorm.app", "Contents/MacOS/webstorm")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("webstorm", "com.jetbrains.WebStorm")],
    });

    // Visual Studio
    apps.push(KnownApp {
        name: "Visual Studio",
        cli_names: &["devenv"],
        #[cfg(target_os = "windows")]
        registry_names: &["devenv.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });

    // Android Studio
    apps.push(KnownApp {
        name: "Android Studio",
        cli_names: &["studio", "android-studio"],
        #[cfg(target_os = "windows")]
        registry_names: &["studio64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Android Studio.app", "Contents/MacOS/studio")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("android-studio", "com.google.AndroidStudio")],
    });

    // Docker Desktop
    apps.push(KnownApp {
        name: "Docker Desktop",
        cli_names: &["docker-desktop"],
        #[cfg(target_os = "windows")]
        registry_names: &["Docker Desktop.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Docker.app", "Contents/MacOS/Docker")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });

    // DBeaver
    apps.push(KnownApp {
        name: "DBeaver",
        cli_names: &["dbeaver", "dbeaver-ce"],
        #[cfg(target_os = "windows")]
        registry_names: &["dbeaver.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("DBeaver.app", "Contents/MacOS/DBeaver")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("dbeaver", "io.dbeaver.DBeaverCommunity")],
    });

    apps
}

// ---------------------------------------------------------------------------
// Resolution API
// ---------------------------------------------------------------------------

/// Resolve an application name or path to a structured launcher.
///
/// This is the authoritative application resolution function. It handles:
/// - Direct executable names (e.g. `"code"`, `"pycharm"`)
/// - Absolute executable paths (e.g. `"/usr/bin/code"`)
/// - Windows `.exe` / `.cmd` shims
/// - macOS `.app` bundles
/// - Linux flatpak applications (structured, not concatenated)
/// - Configured custom paths
///
/// # Arguments
///
/// * `name` — application name, CLI name, or absolute path.
/// * `custom_path` — optional user-configured absolute path override.
/// * `path_overlay` — additional PATH entries to search.
pub fn resolve_application(
    name: &str,
    custom_path: Option<&str>,
    path_overlay: Option<&PathOverlay>,
) -> ApplicationLauncher {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return ApplicationLauncher {
            program: String::new(),
            args: Vec::new(),
            is_flatpak: false,
            found: false,
            diagnostics: vec!["Empty application name".to_string()],
        };
    }

    // 1. Custom path override — highest priority.
    if let Some(custom) = custom_path {
        let custom_trimmed = custom.trim();
        if !custom_trimmed.is_empty() {
            if is_flatpak_invocation(custom_trimmed) {
                return parse_flatpak_invocation(custom_trimmed);
            }
            let p = Path::new(custom_trimmed);
            if p.is_file() {
                return ApplicationLauncher {
                    program: custom_trimmed.to_string(),
                    args: Vec::new(),
                    is_flatpak: false,
                    found: true,
                    diagnostics: vec![format!("Using custom path: {}", custom_trimmed)],
                };
            }
            // Try with extensions on Windows.
            #[cfg(target_os = "windows")]
            {
                for suffix in [".exe", ".cmd", ".bat"] {
                    let candidate = format!("{}{}", custom_trimmed, suffix);
                    if Path::new(&candidate).is_file() {
                        return ApplicationLauncher {
                            program: candidate,
                            args: Vec::new(),
                            is_flatpak: false,
                            found: true,
                            diagnostics: vec![format!(
                                "Using custom path (resolved): {}",
                                custom_trimmed
                            )],
                        };
                    }
                }
            }
        }
    }

    // 2. Absolute/relative path with separator — check directly.
    if trimmed.contains('/') || trimmed.contains('\\') {
        return resolve_path_input(trimmed, path_overlay);
    }

    // 3. Flatpak invocation string (e.g. "flatpak run com.visualstudio.code").
    if is_flatpak_invocation(trimmed) {
        return parse_flatpak_invocation(trimmed);
    }

    // 4. Known application lookup.
    let lower = trimmed.to_ascii_lowercase();
    let apps = known_applications();
    for app in &apps {
        if app.cli_names.iter().any(|cn| lower.starts_with(cn)) {
            let result = resolve_known_app(app, trimmed, path_overlay);
            if result.found {
                return result;
            }
        }
    }

    // 5. Generic PATH lookup.
    if let Some(path) = resolve_executable(trimmed, path_overlay) {
        let path_str = path.to_string_lossy().into_owned();
        let is_batch = is_batch_file(&path_str);
        let program = if is_batch {
            format!("cmd /C \"{}\"", path_str)
        } else {
            path_str
        };
        return ApplicationLauncher {
            program,
            args: Vec::new(),
            is_flatpak: false,
            found: true,
            diagnostics: Vec::new(),
        };
    }

    ApplicationLauncher {
        program: trimmed.to_string(),
        args: Vec::new(),
        is_flatpak: false,
        found: false,
        diagnostics: vec![format!(
            "Application '{}' not found. Install it or use an absolute path.",
            trimmed
        )],
    }
}

/// Resolve a path-like input (contains `/` or `\`).
fn resolve_path_input(trimmed: &str, _path_overlay: Option<&PathOverlay>) -> ApplicationLauncher {
    let p = Path::new(trimmed);
    if p.is_file() {
        return ApplicationLauncher {
            program: trimmed.to_string(),
            args: Vec::new(),
            is_flatpak: false,
            found: true,
            diagnostics: Vec::new(),
        };
    }

    // Windows: try common extensions.
    #[cfg(target_os = "windows")]
    {
        for suffix in [".exe", ".cmd", ".bat"] {
            let candidate = format!("{}{}", trimmed, suffix);
            if Path::new(&candidate).is_file() {
                return ApplicationLauncher {
                    program: candidate,
                    args: Vec::new(),
                    is_flatpak: false,
                    found: true,
                    diagnostics: Vec::new(),
                };
            }
        }
    }

    ApplicationLauncher {
        program: trimmed.to_string(),
        args: Vec::new(),
        is_flatpak: false,
        found: false,
        diagnostics: vec![format!(
            "Application path '{}' does not point to an existing file.",
            trimmed
        )],
    }
}

/// Resolve a known application using platform-specific locations.
fn resolve_known_app(
    app: &KnownApp,
    _cli_name: &str,
    _path_overlay: Option<&PathOverlay>,
) -> ApplicationLauncher {
    let mut diagnostics = Vec::new();

    // PATH lookup first.
    for cn in app.cli_names {
        if let Some(path) = _path_overlay.and_then(|po| {
            for entry in &po.entries {
                let candidate = Path::new(entry).join(cn);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            None
        }) {
            return ApplicationLauncher {
                program: path.to_string_lossy().into_owned(),
                args: Vec::new(),
                is_flatpak: false,
                found: true,
                diagnostics: Vec::new(),
            };
        }

        if let Some(path) = resolve_executable(cn, _path_overlay) {
            let path_str = path.to_string_lossy().into_owned();
            let is_batch = is_batch_file(&path_str);
            return ApplicationLauncher {
                program: path_str,
                args: Vec::new(),
                is_flatpak: false,
                found: true,
                diagnostics: if is_batch {
                    vec![format!("Resolved via PATH (batch shim): {}", app.name)]
                } else {
                    Vec::new()
                },
            };
        }
    }

    // Platform-specific resolution.
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = resolve_windows_app(app) {
            return ApplicationLauncher {
                program: path,
                args: Vec::new(),
                is_flatpak: false,
                found: true,
                diagnostics,
            };
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(path) = resolve_macos_app(app) {
            return ApplicationLauncher {
                program: path,
                args: Vec::new(),
                is_flatpak: false,
                found: true,
                diagnostics,
            };
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Try flatpak as fallback.
        for (cli_pattern, flatpak_id) in app.flatpak_ids {
            let lower_cli = _cli_name.to_ascii_lowercase();
            if lower_cli.starts_with(cli_pattern) {
                if is_flatpak_installed(flatpak_id) {
                    diagnostics.push(format!("Found via flatpak: {} ({})", app.name, flatpak_id));
                    return ApplicationLauncher {
                        program: "flatpak".to_string(),
                        args: vec!["run".to_string(), flatpak_id.to_string()],
                        is_flatpak: true,
                        found: true,
                        diagnostics,
                    };
                }
            }
        }
    }

    diagnostics.push(format!("{} not found on this system", app.name));
    ApplicationLauncher {
        program: String::new(),
        args: Vec::new(),
        is_flatpak: false,
        found: false,
        diagnostics,
    }
}

/// Check if a string looks like a flatpak invocation.
fn is_flatpak_invocation(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.starts_with("flatpak run ")
        || trimmed.starts_with("flatpak ") && trimmed.contains(" run ")
}

/// Parse a flatpak invocation string into a structured launcher.
fn parse_flatpak_invocation(s: &str) -> ApplicationLauncher {
    let tokens: Vec<String> = s.split_whitespace().map(String::from).collect();

    if tokens.len() >= 3 && tokens[0] == "flatpak" && tokens[1] == "run" {
        ApplicationLauncher {
            program: "flatpak".to_string(),
            args: tokens[2..].to_vec(),
            is_flatpak: true,
            found: true, // Assume found; actual check via `flatpak info`
            diagnostics: vec![format!(
                "Parsed as flatpak invocation with ID: {}",
                tokens[2]
            )],
        }
    } else {
        ApplicationLauncher {
            program: tokens.first().cloned().unwrap_or_default(),
            args: tokens.into_iter().skip(1).collect(),
            is_flatpak: false,
            found: false,
            diagnostics: vec!["Malformed flatpak invocation".to_string()],
        }
    }
}

/// Check if a flatpak application is installed.
fn is_flatpak_installed(flatpak_id: &str) -> bool {
    std::process::Command::new("flatpak")
        .args(["info", flatpak_id])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Platform-specific resolution helpers
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn resolve_windows_app(app: &KnownApp) -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok();
    let program_files = std::env::var("PROGRAMFILES").ok();
    let program_files_x86 = std::env::var("PROGRAMFILES(X86)").ok();

    // Try registry App Paths.
    for reg_name in app.registry_names {
        if let Some(path) = registry_app_path(reg_name) {
            return Some(path);
        }
    }

    // Try well-known install locations.
    let mut candidates = Vec::new();
    if let Some(base) = &local {
        let base = PathBuf::from(base);
        match app.name {
            "VS Code" => {
                candidates.push(
                    base.join("Programs")
                        .join("Microsoft VS Code")
                        .join("Code.exe"),
                );
                candidates.push(
                    base.join("Programs")
                        .join("Microsoft VS Code Insiders")
                        .join("Code - Insiders.exe"),
                );
            }
            "Cursor" => {
                candidates.push(base.join("Programs").join("cursor").join("Cursor.exe"));
            }
            "Windsurf" => {
                candidates.push(base.join("Programs").join("Windsurf").join("windsurf.exe"));
            }
            _ => {
                // JetBrains Toolbox apps
                for reg_name in app.registry_names {
                    let name_no_ext = reg_name.trim_end_matches(".exe");
                    candidates.push(
                        base.join("Programs")
                            .join(name_no_ext.replace("64", ""))
                            .join("bin")
                            .join(reg_name),
                    );
                }
            }
        }
    }

    if let Some(pf) = &program_files {
        let pf = PathBuf::from(pf);
        match app.name {
            "VS Code" => {
                candidates.push(pf.join("Microsoft VS Code").join("Code.exe"));
            }
            "Cursor" => {
                candidates.push(pf.join("cursor").join("Cursor.exe"));
            }
            _ => {}
        }
    }

    if let Some(pf86) = &program_files_x86 {
        let pf86 = PathBuf::from(pf86);
        for reg_name in app.registry_names {
            let name_no_ext = reg_name.trim_end_matches(".exe");
            candidates.push(
                pf86.join("JetBrains")
                    .join(name_no_ext.replace("64", ""))
                    .join("bin")
                    .join(reg_name),
            );
        }
    }

    // Visual Studio via vswhere.
    if app.name == "Visual Studio" {
        if let Some(p) = vswhere_devenv() {
            return Some(p);
        }
    }

    for cand in candidates {
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn registry_app_path(exe_name: &str) -> Option<String> {
    let key_name = if exe_name.contains('.') {
        exe_name.to_string()
    } else {
        format!("{}.exe", exe_name)
    };
    for hive in ["HKCU", "HKLM"] {
        let key = format!(
            "{}\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
            hive, key_name
        );
        let out = std::process::Command::new("reg")
            .args(["query", &key, "/ve"])
            .output()
            .ok()?;
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        for line in text.lines().rev() {
            let mut parts = line.split_whitespace();
            let _ = parts.next();
            let _ = parts.next();
            if let Some(path) = parts.next() {
                let p = Path::new(path);
                if p.is_file() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn vswhere_devenv() -> Option<String> {
    let mut vswhere = std::env::var("ProgramFiles(x86)").ok()?;
    vswhere.push_str("\\Microsoft Visual Studio\\Installer\\vswhere.exe");
    if !Path::new(&vswhere).is_file() {
        return None;
    }
    let out = std::process::Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Workload.Universal",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let install_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if install_path.is_empty() {
        return None;
    }
    let devenv = Path::new(&install_path)
        .join("Common7")
        .join("IDE")
        .join("devenv.exe");
    if devenv.is_file() {
        Some(devenv.to_string_lossy().into_owned())
    } else {
        None
    }
}

#[cfg(not(target_os = "windows"))]
fn resolve_windows_app(_app: &KnownApp) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn resolve_macos_app(app: &KnownApp) -> Option<String> {
    let home = std::env::var("HOME").ok();
    let mut base_dirs = vec![PathBuf::from("/Applications")];
    if let Some(h) = &home {
        base_dirs.push(PathBuf::from(h).join("Applications"));
    }

    for (bundle, bin) in app.app_bundles {
        for base in &base_dirs {
            let bin_path = base.join(bundle).join(bin);
            if bin_path.is_file() {
                return Some(bin_path.to_string_lossy().into_owned());
            }
        }
    }

    // Homebrew / local binaries.
    for dir in [
        Some("/opt/homebrew/bin"),
        Some("/usr/local/bin"),
        Some("/usr/bin"),
    ] {
        if let Some(dir) = dir {
            for cn in app.cli_names {
                let p = Path::new(dir).join(cn);
                if p.is_file() {
                    return Some(p.to_string_lossy().into_owned());
                }
            }
        }
    }

    None
}

#[cfg(not(target_os = "macos"))]
fn resolve_macos_app(_app: &KnownApp) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_name_returns_not_found() {
        let result = resolve_application("", None, None);
        assert!(!result.found);
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn flatpak_invocation_parsed_correctly() {
        let result = resolve_application("flatpak run com.visualstudio.code", None, None);
        assert!(result.is_flatpak);
        assert_eq!(result.program, "flatpak");
        assert_eq!(result.args, vec!["run", "com.visualstudio.code"]);
    }

    #[test]
    fn flatpak_invocation_with_args() {
        let result = resolve_application(
            "flatpak run com.jetbrains.PyCharm-Community --project /tmp/proj",
            None,
            None,
        );
        assert!(result.is_flatpak);
        assert_eq!(result.program, "flatpak");
        assert_eq!(
            result.args,
            vec![
                "run",
                "com.jetbrains.PyCharm-Community",
                "--project",
                "/tmp/proj"
            ]
        );
    }

    #[test]
    fn structured_launcher_not_single_path() {
        let result = resolve_application("flatpak run com.visualstudio.code", None, None);
        // The program must be "flatpak", not "flatpak run com.visualstudio.code"
        assert_eq!(result.program, "flatpak");
        assert!(!result.program.contains(" "));
    }

    #[test]
    fn custom_path_override() {
        let result = resolve_application("code", Some("/usr/bin/code"), None);
        assert_eq!(result.program, "/usr/bin/code");
        assert!(result.found || !result.diagnostics.is_empty());
    }

    #[test]
    fn path_with_separator_checked_directly() {
        let result = resolve_application("/usr/bin/sh", None, None);
        assert!(result.found);
        assert_eq!(result.program, "/usr/bin/sh");
    }

    #[test]
    fn application_launcher_display() {
        let launcher = ApplicationLauncher {
            program: "flatpak".to_string(),
            args: vec!["run".to_string(), "com.test.App".to_string()],
            is_flatpak: true,
            found: true,
            diagnostics: Vec::new(),
        };
        assert!(launcher.is_flatpak);
        assert_eq!(launcher.program, "flatpak");
    }

    #[test]
    fn is_flatpak_invocation_detection() {
        assert!(is_flatpak_invocation("flatpak run com.test.App"));
        assert!(is_flatpak_invocation("flatpak  run  com.test.App"));
        assert!(!is_flatpak_invocation("flatpak info com.test.App"));
        assert!(!is_flatpak_invocation("code"));
    }

    #[test]
    fn parse_flatpak_malformed() {
        let result = parse_flatpak_invocation("flatpak");
        assert!(!result.found);
    }

    #[test]
    fn launch_policy_default_is_detached() {
        assert_eq!(LaunchPolicy::default(), LaunchPolicy::Detached);
    }
}
