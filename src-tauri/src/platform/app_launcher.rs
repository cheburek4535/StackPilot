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
// Application registry вЂ” known applications and their resolution hints
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
        // "Docker Desktop" (display name, used by the analyzer on Windows)
        // and "docker-desktop" (Linux CLI) both resolve to the Desktop app.
        // Matching lowercases the input, so the names are lowercase here.
        cli_names: &["docker-desktop", "docker desktop", "dockerdesktop"],
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

    // Browsers
    apps.push(KnownApp {
        name: "Google Chrome",
        cli_names: &["chrome", "google-chrome", "google-chrome-stable"],
        #[cfg(target_os = "windows")]
        registry_names: &["chrome.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Google Chrome.app", "Contents/MacOS/Google Chrome")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("google-chrome", "com.google.Chrome")],
    });
    apps.push(KnownApp {
        name: "Microsoft Edge",
        cli_names: &["msedge", "microsoft-edge", "edge"],
        #[cfg(target_os = "windows")]
        registry_names: &["msedge.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Microsoft Edge.app", "Contents/MacOS/Microsoft Edge")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("microsoft-edge", "com.microsoft.Edge")],
    });
    apps.push(KnownApp {
        name: "Mozilla Firefox",
        cli_names: &["firefox"],
        #[cfg(target_os = "windows")]
        registry_names: &["firefox.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Firefox.app", "Contents/MacOS/firefox")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("firefox", "org.mozilla.firefox")],
    });
    apps.push(KnownApp {
        name: "Brave",
        cli_names: &["brave", "brave-browser"],
        #[cfg(target_os = "windows")]
        registry_names: &["brave.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Brave Browser.app", "Contents/MacOS/Brave Browser")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("brave", "com.brave.Browser")],
    });
    apps.push(KnownApp {
        name: "Opera",
        cli_names: &["opera", "opera-stable"],
        #[cfg(target_os = "windows")]
        registry_names: &["opera.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Opera.app", "Contents/MacOS/Opera")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("opera", "com.opera.Opera")],
    });
    apps.push(KnownApp {
        name: "Yandex Browser",
        cli_names: &["yandex", "yandex-browser"],
        #[cfg(target_os = "windows")]
        registry_names: &["browser.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Yandex.app", "Contents/MacOS/Yandex")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });
    apps.push(KnownApp {
        name: "Chromium",
        cli_names: &["chromium", "chromium-browser"],
        #[cfg(target_os = "windows")]
        registry_names: &["chrome.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("Chromium.app", "Contents/MacOS/Chromium")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("chromium", "org.chromium.Chromium")],
    });

    // Database viewers
    apps.push(KnownApp {
        name: "DataGrip",
        cli_names: &["datagrip", "datagrip64"],
        #[cfg(target_os = "windows")]
        registry_names: &["datagrip64.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("DataGrip.app", "Contents/MacOS/datagrip")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("datagrip", "com.jetbrains.DataGrip")],
    });
    apps.push(KnownApp {
        name: "HeidiSQL",
        cli_names: &["heidisql"],
        #[cfg(target_os = "windows")]
        registry_names: &["heidisql.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });
    apps.push(KnownApp {
        name: "TablePlus",
        cli_names: &["tableplus"],
        #[cfg(target_os = "windows")]
        registry_names: &["TablePlus.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("TablePlus.app", "Contents/MacOS/TablePlus")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
    });
    apps.push(KnownApp {
        name: "DB Browser for SQLite",
        cli_names: &["sqlitebrowser", "db-browser-for-sqlite"],
        #[cfg(target_os = "windows")]
        registry_names: &["SQLiteBrowser.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[(
            "DB Browser for SQLite.app",
            "Contents/MacOS/DB Browser for SQLite",
        )],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[("sqlitebrowser", "org.sqlitebrowser.sqlitebrowser")],
    });
    apps.push(KnownApp {
        name: "pgAdmin 4",
        cli_names: &["pgadmin4"],
        #[cfg(target_os = "windows")]
        registry_names: &["pgAdmin4.exe"],
        #[cfg(target_os = "macos")]
        app_bundles: &[("pgAdmin 4.app", "Contents/MacOS/pgAdmin4")],
        #[cfg(target_os = "linux")]
        flatpak_ids: &[],
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
/// * `name` вЂ” application name, CLI name, or absolute path.
/// * `custom_path` вЂ” optional user-configured absolute path override.
/// * `path_overlay` вЂ” additional PATH entries to search.
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

    // 1. Custom path override вЂ” highest priority.
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

    // 2. Flatpak invocation string (e.g. "flatpak run com.visualstudio.code").
    // Проверяется ДО ветки «строка с разделителем — это путь»: invocation
    // может содержать аргументы-пути ("--project /tmp/proj"), но сама
    // строка путём не является.
    if is_flatpak_invocation(trimmed) {
        return parse_flatpak_invocation(trimmed);
    }

    // 3. Absolute/relative path with separator — check directly.
    if trimmed.contains('/') || trimmed.contains('\\') {
        return resolve_path_input(trimmed, path_overlay);
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

// ---------------------------------------------------------------------------
// Application detection (for selection UIs)
// ---------------------------------------------------------------------------

/// A detected application: stable id (CLI name), display name and the
/// resolved executable path (or structured launcher for flatpak).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetectedApp {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// Result of the `detect_applications` command: everything the selection
/// UIs (Settings, profile page) need to let the user pick applications.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DetectedApplications {
    pub browsers: Vec<DetectedApp>,
    pub db_viewers: Vec<DetectedApp>,
    pub vscode: Option<DetectedApp>,
}

/// All supported database viewer applications and their display names.
pub fn db_viewer_candidates() -> &'static [(&'static str, &'static str)] {
    &[
        ("dbeaver", "DBeaver"),
        ("dbeaver-ce", "DBeaver"),
        ("datagrip", "DataGrip"),
        ("heidisql", "HeidiSQL"),
        ("tableplus", "TablePlus"),
        ("sqlitebrowser", "DB Browser for SQLite"),
        ("pgadmin4", "pgAdmin 4"),
    ]
}

/// All supported browser applications and their display names.
pub fn browser_candidates() -> &'static [(&'static str, &'static str)] {
    &[
        ("chrome", "Google Chrome"),
        ("msedge", "Microsoft Edge"),
        ("firefox", "Mozilla Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("yandex", "Yandex Browser"),
        ("chromium", "Chromium"),
    ]
}

/// Detect a single application by CLI name; `name` is the display name.
pub fn detect_app(cli: &str, name: &str) -> Option<DetectedApp> {
    let launcher = resolve_application(cli, None, None);
    if !launcher.found {
        return None;
    }
    let path = if launcher.is_flatpak {
        format!("{} {}", launcher.program, launcher.args.join(" "))
    } else {
        launcher.program
    };
    Some(DetectedApp {
        id: cli.to_string(),
        name: name.to_string(),
        path,
    })
}

/// Detect every supported browser present on the host.
pub fn detect_browsers() -> Vec<DetectedApp> {
    let mut seen = std::collections::HashSet::new();
    browser_candidates()
        .iter()
        .filter_map(|(cli, name)| detect_app(cli, name))
        .filter(|app| seen.insert(app.path.clone()))
        .collect()
}

/// Detect every supported database viewer present on the host.
pub fn detect_db_viewers() -> Vec<DetectedApp> {
    let mut seen = std::collections::HashSet::new();
    db_viewer_candidates()
        .iter()
        .filter_map(|(cli, name)| detect_app(cli, name))
        .filter(|app| seen.insert(app.path.clone()))
        .collect()
}

/// Detect the user-configured VS Code (settings path) or the default CLI.
pub fn detect_vscode(configured_path: Option<&str>) -> Option<DetectedApp> {
    let cli = match configured_path {
        Some(p) if !p.trim().is_empty() => p.trim(),
        _ => "code",
    };
    detect_app(cli, "VS Code")
}

/// Open a URL in the configured browser (explicit path/CLI) or in the OS
/// default browser when none is configured. Structured launchers (flatpak)
/// are honored through `resolve_application`.
pub fn open_url_in_browser(url: &str, browser_path: Option<&str>) -> Result<(), String> {
    match browser_path {
        Some(path) if !path.trim().is_empty() => {
            let launcher = resolve_application(path, None, None);
            if !launcher.found {
                return Err(format!(
                    "Configured browser '{}' was not found. Install it or fix the path in Settings.",
                    path.trim()
                ));
            }
            let mut args = launcher.args;
            args.push(url.to_string());
            let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            std::process::Command::new(&launcher.program)
                .args(&args_refs)
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("Failed to launch browser '{}': {}", launcher.program, e))
        }
        _ => webbrowser::open(url).map_err(|e| format!("Failed to open browser: {}", e)),
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
        // "run" обязан остаться в аргументах: `flatpak <id>` без run —
        // нерабочая команда.
        let mut args = vec!["run".to_string()];
        args.extend(tokens[2..].iter().cloned());
        ApplicationLauncher {
            program: "flatpak".to_string(),
            args,
            is_flatpak: true,
            found: true, // Assume found; actual check via `flatpak info`
            diagnostics: vec![
                format!(
                    "Parsed as flatpak invocation with ID: {}",
                    tokens[2]
                )
            ],
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
#[allow(dead_code)]
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
            "Android Studio" => {
                // Standard per-user install and JetBrains-Toolbox-style
                // install (Android Studio can be managed by Toolbox too).
                candidates.push(
                    base.join("Programs")
                        .join("Android Studio")
                        .join("bin")
                        .join("studio64.exe"),
                );
            }
            "DBeaver" => {
                // Modern DBeaver installers (22+) install per-user here.
                candidates.push(base.join("DBeaver").join("dbeaver.exe"));
                candidates.push(base.join("Programs").join("DBeaver").join("dbeaver.exe"));
            }
            "Google Chrome" => {
                candidates.push(
                    base.join("Google")
                        .join("Chrome")
                        .join("Application")
                        .join("chrome.exe"),
                );
            }
            "Microsoft Edge" => {
                candidates.push(
                    base.join("Microsoft")
                        .join("Edge")
                        .join("Application")
                        .join("msedge.exe"),
                );
            }
            "Mozilla Firefox" => {
                candidates.push(base.join("Mozilla Firefox").join("firefox.exe"));
            }
            "Brave" => {
                candidates.push(
                    base.join("BraveSoftware")
                        .join("Brave-Browser")
                        .join("Application")
                        .join("brave.exe"),
                );
            }
            "Opera" => {
                candidates.push(base.join("Opera").join("opera.exe"));
            }
            "Yandex Browser" => {
                candidates.push(
                    base.join("Yandex")
                        .join("YandexBrowser")
                        .join("Application")
                        .join("browser.exe"),
                );
            }
            "HeidiSQL" => {
                candidates.push(base.join("HeidiSQL").join("heidisql.exe"));
            }
            "DB Browser for SQLite" => {
                candidates.push(
                    base.join("Programs")
                        .join("DB Browser for SQLite")
                        .join("DB Browser for SQLite.exe"),
                );
            }
            "pgAdmin 4" => {
                candidates.push(base.join("pgAdmin 4").join("bin").join("pgAdmin4.exe"));
            }
            "Docker Desktop" => {
                candidates.push(base.join("Docker").join("Docker Desktop.exe"));
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
            "Android Studio" => {
                candidates.push(
                    pf.join("Android")
                        .join("Android Studio")
                        .join("bin")
                        .join("studio64.exe"),
                );
            }
            "DBeaver" => {
                candidates.push(pf.join("DBeaver").join("dbeaver.exe"));
            }
            "Google Chrome" => {
                candidates.push(
                    pf.join("Google")
                        .join("Chrome")
                        .join("Application")
                        .join("chrome.exe"),
                );
            }
            "Microsoft Edge" => {
                candidates.push(
                    pf.join("Microsoft")
                        .join("Edge")
                        .join("Application")
                        .join("msedge.exe"),
                );
            }
            "Mozilla Firefox" => {
                candidates.push(pf.join("Mozilla Firefox").join("firefox.exe"));
            }
            "Brave" => {
                candidates.push(
                    pf.join("BraveSoftware")
                        .join("Brave-Browser")
                        .join("Application")
                        .join("brave.exe"),
                );
            }
            "Opera" => {
                candidates.push(pf.join("Opera").join("opera.exe"));
            }
            "Yandex Browser" => {
                candidates.push(
                    pf.join("Yandex")
                        .join("YandexBrowser")
                        .join("Application")
                        .join("browser.exe"),
                );
            }
            "HeidiSQL" => {
                candidates.push(pf.join("HeidiSQL").join("heidisql.exe"));
            }
            "DB Browser for SQLite" => {
                candidates.push(
                    pf.join("DB Browser for SQLite")
                        .join("DB Browser for SQLite.exe"),
                );
            }
            "pgAdmin 4" => {
                candidates.push(pf.join("pgAdmin 4").join("bin").join("pgAdmin4.exe"));
            }
            "Docker Desktop" => {
                candidates.push(pf.join("Docker").join("Docker").join("Docker Desktop.exe"));
                candidates.push(
                    pf.join("Docker")
                        .join("Docker")
                        .join("resources")
                        .join("Docker Desktop.exe"),
                );
            }
            _ => {}
        }
    }

    if let Some(pf86) = &program_files_x86 {
        let pf86 = PathBuf::from(pf86);
        match app.name {
            "DBeaver" => {
                candidates.push(pf86.join("DBeaver").join("dbeaver.exe"));
            }
            "HeidiSQL" => {
                candidates.push(pf86.join("HeidiSQL").join("heidisql.exe"));
            }
            _ => {
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

    // Last resort: Start Menu shortcuts. Covers installs that register
    // neither App Paths nor a well-known location (per-user installs,
    // some Store/MSIX layouts).
    start_menu_target(app.name)
}

/// Resolve an application through its Start Menu shortcut (`.lnk`) on
/// Windows. Only directly spawnable targets are returned (payloads inside
/// the protected `WindowsApps` directory need package activation and are
/// handled by [`start_menu_launcher`]).
#[cfg(target_os = "windows")]
fn start_menu_target(app_name: &str) -> Option<String> {
    let launcher = start_menu_launcher(app_name)?;
    if !launcher.args.is_empty() {
        // Structured launcher (WindowsApps alias) вЂ” the path-only resolver
        // would drop its arguments.
        return None;
    }
    Some(launcher.program)
}

/// Resolve an application through its Start Menu shortcut (`.lnk`) on
/// Windows, returning a structured launcher:
/// - classic shortcuts resolve to their target exe;
/// - targets inside the protected `WindowsApps` directory (Store/MSIX
///   installs) cannot be spawned directly, so the package is re-launched
///   through `explorer.exe shell:AppsFolder\<PackageFamilyName>!<AppId>`,
///   derived from the package folder name.
#[cfg(target_os = "windows")]
pub fn start_menu_launcher(app_name: &str) -> Option<ApplicationLauncher> {
    let script = format!(
        r#"$sh = New-Object -ComObject WScript.Shell
$name = '*{name}*'
$roots = @("$env:APPDATA\Microsoft\Windows\Start Menu\Programs", "$env:ProgramData\Microsoft\Windows\Start Menu\Programs")
$targets = foreach ($r in $roots) {{
  Get-ChildItem -LiteralPath $r -Recurse -Filter '*.lnk' -ErrorAction SilentlyContinue |
    Where-Object {{ $_.BaseName -like $name -or $_.Name -like $name -or $_.FullName -like $name }} |
    ForEach-Object {{ try {{ $sh.CreateShortcut($_.FullName).TargetPath }} catch {{ $null }} }}
}}
$targets | Where-Object {{ $_ }} | Select-Object -Unique -First 1"#,
        name = app_name.replace('*', "").replace('?', "")
    );
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    crate::platform::suppress_child_console(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let target = text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())?
        .to_string();
    let target_path = Path::new(&target);
    if target_path.is_file() {
        return Some(ApplicationLauncher {
            program: target,
            args: Vec::new(),
            is_flatpak: false,
            found: true,
            diagnostics: Vec::new(),
        });
    }
    // WindowsApps payloads are not directly spawnable. Re-launch the
    // package through its AppsFolder alias when the folder name reveals
    // the package family name (e.g. Docker.DockerDesktop_4.28.0.133772).
    if let Some(alias) = windows_apps_folder_alias(&target_path) {
        return Some(ApplicationLauncher {
            program: "explorer".to_string(),
            args: vec![alias],
            is_flatpak: false,
            found: true,
            diagnostics: Vec::new(),
        });
    }
    None
}

/// Derive the `shell:AppsFolder\<PFN>!<AppId>` alias for an executable
/// inside the protected WindowsApps directory.
#[cfg(target_os = "windows")]
fn windows_apps_folder_alias(target: &Path) -> Option<String> {
    let mut pfn: Option<String> = None;
    let mut exe_name: Option<String> = None;
    for comp in target.components() {
        if let std::path::Component::Normal(seg) = comp {
            let seg = seg.to_string_lossy().to_string();
            // First WindowsApps child is the package folder: <PFN>_<version>.
            if pfn.is_none() && seg.contains('_') && seg.contains('.') {
                pfn = Some(seg.split('_').next().unwrap_or(&seg).to_string());
                continue;
            }
            if pfn.is_some() {
                exe_name = Some(seg.trim_end_matches(".exe").replace(' ', "").to_string());
            }
        }
    }
    Some(format!("shell:AppsFolder\\{}!{}", pfn?, exe_name?))
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
        let mut cmd = std::process::Command::new("reg");
        cmd.args(["query", &key, "/ve"]);
        crate::platform::suppress_child_console(&mut cmd);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(path) = extract_registry_default_value(&text) {
            return Some(path);
        }
    }
    None
}

/// Extract the `(Default)` value from `reg query` output. The value may
/// contain spaces ("Docker Desktop.exe", "Android Studio\bin\studio64.exe"),
/// so the path is everything after the value-type token вЂ” never a
/// whitespace-split segment.
#[cfg(target_os = "windows")]
fn extract_registry_default_value(text: &str) -> Option<String> {
    for line in text.lines().rev() {
        let line = line.trim();
        let idx = line.find("REG_")?;
        let mut path = line[idx + 4..].trim().to_string();
        // REG_EXPAND_SZ values are printed as `@path`.
        if let Some(stripped) = path.strip_prefix('@') {
            path = stripped.trim().to_string();
        }
        if path.is_empty() {
            continue;
        }
        let path = path.trim_matches('"');
        let p = Path::new(path);
        if p.is_file() {
            return Some(path.to_string());
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
    let mut cmd = std::process::Command::new(&vswhere);
    cmd.args([
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Workload.Universal",
        "-property",
        "installationPath",
    ]);
    crate::platform::suppress_child_console(&mut cmd);
    let out = cmd.output().ok()?;
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
#[allow(dead_code)]
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
    #[cfg(target_os = "linux")]
    fn flatpak_invocation_parsed_correctly() {
        let result = resolve_application("flatpak run com.visualstudio.code", None, None);
        assert!(result.is_flatpak);
        assert_eq!(result.program, "flatpak");
        assert_eq!(result.args, vec!["run", "com.visualstudio.code"]);
    }

    #[test]
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
    fn structured_launcher_not_single_path() {
        let result = resolve_application("flatpak run com.visualstudio.code", None, None);
        // The program must be "flatpak", not "flatpak run com.visualstudio.code"
        assert_eq!(result.program, "flatpak");
        assert!(!result.program.contains(" "));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn custom_path_override() {
        // Файл создаём во временной директории: тест не должен зависеть
        // от того, установлено ли приложение в системе.
        let dir = std::env::temp_dir().join(format!("sp-app-ovr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("mycustomapp");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755));
        }
        let result = resolve_application("code", Some(exe.to_str().unwrap()), None);
        assert_eq!(result.program, exe.to_string_lossy());
        assert!(result.found);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
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

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_default_value_parses_paths_with_spaces() {
        // Simulated `reg query` output вЂ” the value contains spaces, which
        // the old whitespace-split parsing truncated at the first space.
        let out = "\n\
            HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Docker Desktop.exe\n\
            \x20   (Default)    REG_SZ    C:\\Program Files\\Docker\\Docker\\Docker Desktop.exe\n\n";
        let extracted = extract_registry_default_value(out);
        // The path must parse (on this machine the file exists), and the
        // parse must never truncate at the first space.
        if let Some(path) = extracted {
            assert!(path.contains("Docker Desktop.exe"), "path: {}", path);
            assert!(!path.starts_with("C:\\Program"), "truncated: {}", path);
        }
        // REG_EXPAND_SZ with @-prefixed value must parse the same way.
        let expand = "\n\
            HKEY_CURRENT_USER\\...\\App Paths\\code.exe\n\
            \x20   (Default)    REG_EXPAND_SZ    @C:\\Program Files\\Microsoft VS Code\\Code.exe\n\n";
        let extracted = extract_registry_default_value(expand);
        if let Some(path) = extracted {
            assert!(path.contains("Code.exe"), "path: {}", path);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn real_windows_apps_resolve() {
        // Machine probe: these are the apps the devlauncher analyzer needs
        // on Windows. Resolution must find real installs (this machine has
        // Android Studio, DBeaver and Docker Desktop installed).
        let pf = std::env::var("PROGRAMFILES").unwrap_or_default();
        log::debug!("PROGRAMFILES='{}'", pf);
        log::debug!(
            "docker exe exists: {}",
            std::path::Path::new(r"C:\Program Files\Docker\Docker\Docker Desktop.exe").is_file()
        );
        for name in ["studio64", "Docker Desktop", "dbeaver"] {
            let result = resolve_application(name, None, None);
            log::debug!(
                "'{}' -> found={} program='{}' diag={:?}",
                name, result.found, result.program, result.diagnostics
            );
            assert!(
                result.found,
                "'{}' must resolve on this machine (diagnostics: {:?})",
                name, result.diagnostics
            );
        }
    }
}
