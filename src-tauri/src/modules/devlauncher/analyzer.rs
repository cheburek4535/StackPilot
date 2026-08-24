use crate::modules::devlauncher::models::*;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String>;
}

pub struct FsProjectAnalyzer;

impl ProjectAnalyzer for FsProjectAnalyzer {
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String> {
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
        let port_re = regex::Regex::new(r"--port\s+(\d+)").unwrap();

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
                        },
                    });
                    has_docker = true;
                }
                _ => {}
            }

            if filename == "Dockerfile" && !has_dockerfile {
                has_dockerfile = true;
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Build Docker image".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "docker build -t myapp .".into(),
                        working_dir: Some(cwd.clone()),
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Docker container".into(),
                    enabled: !has_docker,
                    action_type: ActionType::RunCommand {
                        command: "docker run -p 8080:80 myapp".into(),
                        working_dir: Some(cwd.clone()),
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

                // Выбираем ТОЛЬКО существующий npm-скрипт: "dev" → "start" →
                // "serve" → первый из списка. Раньше тут подставлялся
                // несуществующий "dev" вслепую — профиль падал с
                // "Missing script: dev"/"Missing script: nuxt".
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
                } else if let Some(scripts) = scripts {
                    let chosen = ["dev", "start", "serve"]
                        .iter()
                        .find(|k| scripts.get(*k).is_some())
                        .map(|k| k.to_string())
                        .or_else(|| scripts.as_object().and_then(|m| m.keys().next()).cloned());
                    match chosen {
                        Some(key) => Some(format!("npm run {}", key)),
                        // Скриптов нет вообще: пробуем прямой запуск main
                        None => value
                            .get("main")
                            .and_then(|m| m.as_str())
                            .map(|main| format!("node {}", main)),
                    }
                } else {
                    // Секции scripts нет — main, если есть, иначе нечего запускать
                    value
                        .get("main")
                        .and_then(|m| m.as_str())
                        .map(|main| format!("node {}", main))
                };

                if let Some(run_cmd) = run_cmd {
                    let mut port: u16 = 3000;
                    if let Some(caps) = port_re.captures(&run_cmd) {
                        if let Ok(p) = caps[1].parse() {
                            port = p;
                        }
                    }

                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Run Node.js project".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: run_cmd,
                            working_dir: Some(cwd.clone()),
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
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Django DB migrate".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python manage.py migrate".into(),
                        working_dir: Some(cwd.clone()),
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
                    },
                });
                launch_actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Hot reload .NET".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "dotnet watch".into(),
                        working_dir: Some(cwd.clone()),
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

        // IDE detection
        let (ide_name, ide_label, ide_fallback, ide_fb_label) = if has_python_project {
            (
                "pycharm",
                "Open in PyCharm",
                "code",
                "Open in VS Code (PyCharm not found)",
            )
        } else if has_sln {
            ("devenv", "Open in Visual Studio", "code", "Open in VS Code")
        } else {
            ("code", "Open in VS Code", "", "")
        };

        if which::which(ide_name).is_ok() {
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
        } else if !ide_fallback.is_empty() && which::which(ide_fallback).is_ok() {
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
