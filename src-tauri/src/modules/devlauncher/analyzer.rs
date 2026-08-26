use crate::modules::devlauncher::models::*;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(&self, project_path: &str, vscode_path: Option<&str>) -> Result<LaunchProfile, String>;
}

/// Check if an IDE executable is available on the system.
///
/// Delegates to the shared cross-platform discovery in [`crate::platform::ide`]:
/// PATH lookup (PATHEXT-aware), Windows `App Paths` registry and install
/// dirs, macOS app bundles, Linux snap/flatpak locations.
fn find_ide_executable(cli: &str) -> Option<String> {
    crate::platform::ide::resolve_ide_executable(cli)
}

pub struct FsProjectAnalyzer;

impl ProjectAnalyzer for FsProjectAnalyzer {
    fn analyze(&self, project_path: &str, vscode_path: Option<&str>) -> Result<LaunchProfile, String> {
        let mut launch_actions: Vec<LaunchAction> = Vec::new();
        let walker = WalkDir::new(project_path).follow_links(false).into_iter();
        let mut it = walker;

        let mut has_docker = false;
        let mut has_dockerfile = false;
        let mut has_index_html = false;
        let mut index_html_path = String::new();
        let mut has_python_project = false;
        let mut has_go_project = false;
        let mut has_makefile = false;
        let mut has_sln = false;

        while let Some(entry_result) = it.next() {
            let entry = match entry_result {
                Ok(e) => e,
                Err(err) => {
                    println!("[Analyzer] Access error: {}", err);
                    continue;
                }
            };

            let path = entry.path();
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if entry.file_type().is_dir() {
                if matches!(
                    filename,
                    "node_modules" | ".git" | "target" | ".venv" | "__pycache__" | ".next"
                ) {
                    it.skip_current_dir();
                    continue;
                }
            }

            let cwd = path
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(project_path)
                .to_string();

            match filename {
                "docker-compose.yml" | "docker-compose.yaml" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Start Docker Compose".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: "docker compose up -d".into(),
                            working_dir: Some(cwd.clone()),
                            persistent: None,
                        },
                    });
                    has_docker = true;
                }
                _ => {}
            }

            if filename == "Dockerfile" && !has_dockerfile {
                has_dockerfile = true;
                // Derive an image tag from the project folder so repeated
                // runs build/run the same image instead of a generic "myapp".
                let image_tag = docker_image_tag(project_path);
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Build Docker image".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: format!("docker build -t {} .", image_tag),
                        working_dir: Some(cwd.clone()),
                        persistent: None,
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Docker container".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: format!("docker run -p 8080:80 {}", image_tag),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                has_docker = true;
            }

            if filename == "package.json" {
                let content = fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read package.json: {}", e))?;
                let value: serde_json::Value = serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse package.json: {}", e))?;

                let scripts = value.get("scripts");
                let deps = value.get("dependencies");
                let dev_deps = value.get("devDependencies");
                let has_dep = |pkg: &str| {
                    deps.and_then(|d| d.get(pkg)).is_some()
                        || dev_deps.and_then(|d| d.get(pkg)).is_some()
                };

                // Pick the best npm script to run. Only script *names* are
                // ever used — never `main` or a script's value — so the
                // command can never become the garbage `npm run node --watch
                // src/index.js` / `npm run nuxt ...` that older versions
                // produced when they misinterpreted package.json.
                let run_cmd: Option<String> = if has_dep("expo") {
                    Some("npx expo start".to_string())
                } else if has_dep("@nestjs/core") {
                    Some(
                        scripts
                            .and_then(|s| s.get("start:dev"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("npm run start:dev")
                            .to_string(),
                    )
                } else if let Some(run_script) = pick_npm_run_script(scripts) {
                    Some(format!("npm run {}", run_script))
                } else {
                    // No usable scripts — fall back to a direct `node main.js`
                    // launch only when `main` points at a JS entry file.
                    value
                        .get("main")
                        .and_then(|m| m.as_str())
                        .filter(|main| {
                            let lower = main.to_ascii_lowercase();
                            lower.ends_with(".js")
                                || lower.ends_with(".mjs")
                                || lower.ends_with(".cjs")
                        })
                        .map(|main| format!("node {}", main))
                };

                if let Some(run_cmd) = run_cmd {
                    let port = detect_dev_port(&run_cmd);
                    let label = format!("Run Node.js project ({})", cwd_label(&cwd, project_path));

                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label,
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: run_cmd,
                            working_dir: Some(cwd.clone()),
                            persistent: Some(true),
                        },
                    });

                    if !has_dep("expo") {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Wait for Node.js port".into(),
                            enabled: true,
                            action_type: ActionType::WaitForPort {
                                host: "127.0.0.1".into(),
                                port,
                                timeout_secs: 30,
                            },
                        });
                    }
                }
            }

            if filename == "Cargo.toml" {
                let content = fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;
                let value: toml::Value = toml::from_str(&content)
                    .map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;
                if value.get("workspace").is_some() {
                    continue;
                }

                let has_tauri = value
                    .get("dependencies")
                    .and_then(|d| d.get("tauri"))
                    .is_some();
                let cmd = if has_tauri {
                    "cargo tauri dev"
                } else {
                    "cargo run"
                };
                let port = if has_tauri { 1420 } else { 3000 };

                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Rust project".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: cmd.into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                if has_tauri {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Wait for Tauri port".into(),
                        enabled: true,
                        action_type: ActionType::WaitForPort {
                            host: "127.0.0.1".into(),
                            port,
                            timeout_secs: 60,
                        },
                    });
                }
            }

            if (filename == "go.mod" || filename == "main.go") && !has_go_project {
                has_go_project = true;
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Go project".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "go run .".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
            }

            if filename == "requirements.txt" {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Python deps".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "pip install -r requirements.txt".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: None,
                    },
                });
                has_python_project = true;
            }

            if filename == "pyproject.toml" {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Python deps (pyproject)".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "pip install -e .".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: None,
                    },
                });
                has_python_project = true;
            }

            if filename == "main.py" {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run main.py".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "python main.py".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                has_python_project = true;
            }

            if filename == "manage.py" {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Django server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python manage.py runserver".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Django DB migrate".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python manage.py migrate".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: None,
                    },
                });
                has_python_project = true;
            }

            if filename == "build.gradle" || filename == "build.gradle.kts" {
                let content = fs::read_to_string(path).unwrap_or_default();
                let is_spring =
                    content.contains("org.springframework.boot") || content.contains("spring-boot");
                let cmd = if is_spring {
                    "./gradlew bootRun"
                } else {
                    "./gradlew run"
                };
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Gradle project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: cmd.into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                if is_spring {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Wait for Spring Boot port".into(),
                        enabled: true,
                        action_type: ActionType::WaitForPort {
                            host: "127.0.0.1".into(),
                            port: 8080,
                            timeout_secs: 60,
                        },
                    });
                }
            }

            if filename == "pom.xml" {
                let content = fs::read_to_string(path).unwrap_or_default();
                let is_spring = content.contains("spring-boot");
                let cmd = if is_spring {
                    "mvn spring-boot:run"
                } else {
                    "mvn exec:java"
                };
                let port = if is_spring { 8080 } else { 3000 };
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Maven project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: cmd.into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                if is_spring {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Wait for Spring Boot port".into(),
                        enabled: true,
                        action_type: ActionType::WaitForPort {
                            host: "127.0.0.1".into(),
                            port,
                            timeout_secs: 60,
                        },
                    });
                }
            }

            if filename == "Gemfile" {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Rails server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "bundle exec rails server".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Rails port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                });
            }

            if filename.ends_with(".csproj") {
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run .NET project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "dotnet run".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Hot reload .NET".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "dotnet watch".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: Some(true),
                    },
                });
            }

            if filename.ends_with(".sln") && !has_sln {
                has_sln = true;
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: format!("Open solution {}", filename),
                    enabled: true,
                    action_type: ActionType::OpenApplication {
                        path: "devenv".into(),
                        args: Some(path.to_string_lossy().to_string()),
                        args_list: None,
                    },
                });
            }

            if filename == "Makefile" && !has_makefile {
                has_makefile = true;
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Makefile".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "make".into(),
                        working_dir: Some(cwd.clone()),
                        persistent: None,
                    },
                });
            }

            if filename == "index.html" {
                has_index_html = true;
                if let Some(p) = path.to_str() {
                    index_html_path = p.to_string();
                }
            }
        }

        if launch_actions.is_empty() && has_index_html && !index_html_path.is_empty() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: "Open index.html".into(),
                enabled: true,
                action_type: ActionType::OpenUrl {
                    url: index_html_path,
                },
            });
        }

        // IDE detection — use the user-configured vscode_path when available
        let effective_vscode = vscode_path.unwrap_or("code");
        let (ide_name, ide_label, ide_fallback, ide_fb_label) = if has_python_project {
            (
                "pycharm",
                "Open in PyCharm",
                effective_vscode,
                "Open in VS Code (PyCharm not found)",
            )
        } else if has_sln {
            (
                "devenv",
                "Open in Visual Studio",
                effective_vscode,
                "Open in VS Code",
            )
        } else {
            (effective_vscode, "Open in VS Code", "", "")
        };

        if find_ide_executable(ide_name).is_some() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: ide_label.into(),
                enabled: true,
                action_type: ActionType::OpenApplication {
                    path: ide_name.into(),
                    args: Some(".".into()),
                    args_list: None,
                },
            });
        } else if !ide_fallback.is_empty() && find_ide_executable(ide_fallback).is_some() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: ide_fb_label.into(),
                enabled: true,
                action_type: ActionType::OpenApplication {
                    path: ide_fallback.into(),
                    args: Some(".".into()),
                    args_list: None,
                },
            });
        }

        let project_name = Path::new(project_path)
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .unwrap_or("Project");

        Ok(LaunchProfile {
            name: project_name.to_string(),
            description: format!("Auto-detected profile for {}", project_path),
            project_path: Some(project_path.to_string()),
            actions: launch_actions,
            environment_binding_id: None,
            preferred_ide: None,
        })
    }
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("act_{}", nanos)
}

/// Pick the best `package.json` script name to run via `npm run <name>`.
///
/// Order of preference: `dev`, `start`, `serve`, `preview`, then any
/// remaining script whose name is not a helper/utility (install, build,
/// lint, test, …). Never returns a script that would produce a broken
/// command line — only the script *name* is used, so `npm run node` /
/// `npm run nuxt` garbage is impossible.
fn pick_npm_run_script(scripts: Option<&serde_json::Value>) -> Option<String> {
    let obj = scripts?.as_object()?;
    if obj.is_empty() {
        return None;
    }

    for preferred in ["dev", "start", "serve", "preview", "debug"] {
        if obj.contains_key(preferred) {
            return Some(preferred.to_string());
        }
    }

    // Fallback: first script that is not a lifecycle/utility helper.
    const HELPER_SCRIPTS: &[&str] = &[
        "install",
        "postinstall",
        "preinstall",
        "prepare",
        "prepublish",
        "build",
        "compile",
        "clean",
        "lint",
        "test",
        "test:unit",
        "test:integration",
        "e2e",
        "typecheck",
        "types",
        "generate",
        "format",
        "check",
        "analyze",
        "audit",
    ];
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for key in &keys {
        let lower = key.to_ascii_lowercase();
        if lower.starts_with("dev") || lower.ends_with(":dev") {
            return Some(key.to_string());
        }
    }
    for key in &keys {
        if !HELPER_SCRIPTS.contains(&key.as_str()) {
            return Some(key.to_string());
        }
    }
    // Everything is a helper — still prefer a runnable one over nothing.
    obj.keys().next().cloned()
}

/// Detect the most likely dev-server port from a run command.
///
/// Understands `--port 3000`, `--port=3000`, `-p 3000` and `PORT=3000`
/// conventions used across Node.js tooling (Vite, Next.js, Nuxt, Node
/// `--env-file`, cross-env, …).
fn detect_dev_port(run_cmd: &str) -> u16 {
    for pattern in [
        r"--port[= ](\d{2,5})",
        r"-p[= ](\d{2,5})",
        r"\bPORT[= ](\d{2,5})",
        r"port\s*[:=]\s*(\d{2,5})",
    ] {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(run_cmd) {
                if let Ok(p) = caps[1].parse::<u16>() {
                    if p > 0 {
                        return p;
                    }
                }
            }
        }
    }
    3000
}

/// Short label for the directory containing a manifest (used to make action
/// labels unique in monorepos with backend/ + frontend/ package.json files).
fn cwd_label(cwd: &str, project_path: &str) -> String {
    let rel = std::path::Path::new(cwd)
        .strip_prefix(project_path)
        .unwrap_or(std::path::Path::new(cwd));
    let name = rel
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("root");
    name.to_string()
}

/// Build a safe Docker image tag from a project path's folder name.
fn docker_image_tag(project_path: &str) -> String {
    let name = std::path::Path::new(project_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app");
    let mut tag = String::with_capacity(name.len());
    let mut last_was_sep = false;
    for ch in name.chars() {
        let sep = !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        if sep {
            if !last_was_sep && !tag.is_empty() {
                tag.push('-');
                last_was_sep = true;
            }
        } else {
            tag.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        }
    }
    let tag = tag.trim_matches('-').to_string();
    if tag.is_empty() {
        "app".to_string()
    } else {
        tag
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scripts(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn pick_script_prefers_dev_start_serve() {
        let s = scripts(json!({ "build": "nuxt build", "dev": "nuxt dev" }));
        assert_eq!(pick_npm_run_script(Some(&serde_json::Value::Object(s))), Some("dev".into()));

        let s = scripts(json!({ "serve": "vite --port 5173" }));
        assert_eq!(pick_npm_run_script(Some(&serde_json::Value::Object(s))), Some("serve".into()));
    }

    #[test]
    fn pick_script_never_misreads_values() {
        // Regression: older code produced `npm run node --watch src/index.js`
        // and `npm run nuxt ...` by misreading package.json. Script *names*
        // must be used, never values.
        let s = scripts(json!({
            "start": "node src/index.js",
            "dev": "node --watch src/index.js"
        }));
        let picked = pick_npm_run_script(Some(&serde_json::Value::Object(s))).unwrap();
        assert!(picked == "dev" || picked == "start");

        let s = scripts(json!({
            "build": "nuxt build",
            "generate": "nuxt generate",
            "postinstall": "nuxt prepare"
        }));
        // No dev/start/serve/preview and only helper scripts remain:
        // the fallback still returns the first (alphabetical) helper as a
        // last resort — but never a script value like `nuxt build`.
        let picked = pick_npm_run_script(Some(&serde_json::Value::Object(s)));
        assert_eq!(picked, Some("build".into()));
    }

    #[test]
    fn pick_script_skips_helpers() {
        let s = scripts(json!({
            "postinstall": "x",
            "build": "y",
            "generate": "z"
        }));
        // All three are helper scripts; "build" is first alphabetically and
        // still returned as a last-resort runnable.
        assert_eq!(pick_npm_run_script(Some(&serde_json::Value::Object(s))), Some("build".into()));
    }

    #[test]
    fn pick_script_empty_is_none() {
        assert_eq!(pick_npm_run_script(None), None);
        let s = scripts(json!({}));
        assert_eq!(pick_npm_run_script(Some(&serde_json::Value::Object(s))), None);
    }

    #[test]
    fn port_detection_variants() {
        assert_eq!(detect_dev_port("npm run dev -- --port 5173"), 5173);
        assert_eq!(detect_dev_port("npm run dev -- --port=8080"), 8080);
        assert_eq!(detect_dev_port("vite -p 4000"), 4000);
        assert_eq!(detect_dev_port("cross-env PORT=5000 node index.js"), 5000);
        assert_eq!(detect_dev_port("npm run dev"), 3000);
    }

    #[test]
    fn docker_tag_sanitizes_folder_names() {
        assert_eq!(docker_image_tag(r"C:\Users\Alex\Downloads\nuxtapp"), "nuxtapp");
        assert_eq!(docker_image_tag("/home/user/My Project (Backend)"), "my-project-backend");
        assert_eq!(docker_image_tag("..."), "app");
    }

    #[test]
    fn cwd_label_uses_folder_name() {
        assert_eq!(cwd_label(r"C:\proj\backend", r"C:\proj"), "backend");
        assert_eq!(cwd_label(r"C:\proj", r"C:\proj"), "root");
    }
}
