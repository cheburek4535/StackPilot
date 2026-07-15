// ============================================================
// ProjectAnalyzer — анализатор проектов.
//
// Исправленная версия после code review.
// Изменения:
//   1. Исправлен баг: index_html_sys_path никогда не
//      присваивался — OpenUrl для index.html был с пустым URL.
//   2. Добавлены: Dockerfile, go.mod, pyproject.toml, Makefile, .sln
//   3. Для Dockerfile добавлено 2 действия: build и run
//   4. Порт для Node.js проектов динамический (парсинг --port)
// ============================================================

use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::models::*;

// --------------------------------------------------
// Трейт ProjectAnalyzer
// --------------------------------------------------
pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String>;
}

pub struct FsProjectAnalyzer;

impl ProjectAnalyzer for FsProjectAnalyzer {
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String> {
        let mut launch_actions: Vec<LaunchAction> = Vec::new();
        let walker = WalkDir::new(project_path).follow_links(false).into_iter();
        let mut it = walker;

        // Флаги для отслеживания найденных технологий
        let mut has_docker = false;
        let mut has_dockerfile = false;
        let mut has_index_html = false;
        let mut index_html_path = String::new();
        let mut has_python_project = false;
        let mut has_go_project = false;
        let mut has_makefile = false;
        let mut has_sln = false;

        // Регулярка для поиска порта в npm-скриптах
        let port_re = regex::Regex::new(r"--port\s+(\d+)").unwrap();

        while let Some(entry_result) = it.next() {
            let entry = match entry_result {
                Ok(e) => e,
                Err(err) => {
                    println!("[ProjectAnalyzer] Ошибка доступа: {}", err);
                    continue;
                }
            };

            let path = entry.path();
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            // Пропускаем служебные папки walkdir
            if entry.file_type().is_dir() {
                if matches!(
                    filename,
                    "node_modules" | ".git" | "target" | ".venv" | "__pycache__" | ".next"
                ) {
                    it.skip_current_dir();
                    continue;
                }
            }

            let current_working_dir = path
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(project_path)
                .to_string();

            // ============================================================
            // DOCKER
            // ============================================================
            match filename {
                "docker-compose.yml" | "docker-compose.yaml" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Запустить Docker Compose".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: "docker compose up -d".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    has_docker = true;
                }
                "Dockerfile" => {
                    // Отдельно: просто Dockerfile (без compose)
                    if !has_dockerfile {
                        has_dockerfile = true;
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Собрать Docker образ".into(),
                            enabled: !has_docker,
                            action_type: ActionType::RunCommand {
                                command: "docker build -t myapp .".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Запустить Docker контейнер".into(),
                            enabled: !has_docker,
                            action_type: ActionType::RunCommand {
                                command: "docker run -p 8080:80 myapp".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                        has_docker = true;
                    }
                }
                // ============================================================
                // NODE.JS
                // ============================================================
                "package.json" => {
                    let content = fs::read_to_string(path)
                        .map_err(|e| format!("Ошибка чтения package.json: {}", e))?;
                    let value: serde_json::Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Ошибка парсинга package.json: {}", e))?;

                    let scripts = value.get("scripts");
                    let deps = value.get("dependencies");
                    let dev_deps = value.get("devDependencies");

                    let has_dep = |pkg: &str| {
                        deps.and_then(|d| d.get(pkg)).is_some()
                            || dev_deps.and_then(|d| d.get(pkg)).is_some()
                    };

                    // Определяем команду запуска
                    let run_command = if has_dep("expo") {
                        "npx expo start".to_string()
                    } else if has_dep("@nestjs/core") {
                        scripts
                            .and_then(|s| s.get("start:dev"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("npm run start:dev")
                            .to_string()
                    } else {
                        let chosen = scripts
                            .and_then(|s| s.get("dev").or_else(|| s.get("start")))
                            .and_then(|c| c.as_str())
                            .unwrap_or("dev");
                        format!("npm run {}", chosen)
                    };

                    // Парсим порт из команды
                    let mut port: u16 = 3000;
                    if let Some(caps) = port_re.captures(&run_command) {
                        if let Ok(p) = caps[1].parse() {
                            port = p;
                        }
                    }

                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: format!("Запустить Node.js проект"),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: run_command,
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });

                    if !has_dep("expo") {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Ожидание порта Node.js".into(),
                            enabled: true,
                            action_type: ActionType::WaitForPort {
                                host: "127.0.0.1".into(),
                                port,
                                timeout_secs: 30,
                            },
                        });
                    }
                }
                // ============================================================
                // RUST
                // ============================================================
                "Cargo.toml" => {
                    let content = fs::read_to_string(path)
                        .map_err(|e| format!("Ошибка чтения Cargo.toml: {}", e))?;
                    let value: toml::Value = toml::from_str(&content)
                        .map_err(|e| format!("Ошибка парсинга Cargo.toml: {}", e))?;

                    // Пропускаем workspace root
                    if value.get("workspace").is_some() {
                        continue;
                    }

                    let has_tauri = value
                        .get("dependencies")
                        .and_then(|d| d.get("tauri"))
                        .is_some();

                    let cmd = if has_tauri { "cargo tauri dev" } else { "cargo run" };

                    let port = if has_tauri { 1420 } else { 3000 };
                    let is_enabled = !has_docker;

                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: format!("Запустить Rust проект"),
                        enabled: is_enabled,
                        action_type: ActionType::RunCommand {
                            command: cmd.into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });

                    if has_tauri {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Ожидание порта Tauri".into(),
                            enabled: true,
                            action_type: ActionType::WaitForPort {
                                host: "127.0.0.1".into(),
                                port,
                                timeout_secs: 60,
                            },
                        });
                    }
                }
                // ============================================================
                // GO
                // ============================================================
                "go.mod" | "main.go" => {
                    if !has_go_project {
                        has_go_project = true;
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Запустить Go проект".into(),
                            enabled: !has_docker,
                            action_type: ActionType::RunCommand {
                                command: "go run .".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                    }
                }
                // ============================================================
                // PYTHON
                // ============================================================
                "requirements.txt" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Установка Python зависимостей".into(),
                        enabled: !has_docker,
                        action_type: ActionType::RunCommand {
                            command: "pip install -r requirements.txt".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    has_python_project = true;
                }
                "pyproject.toml" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Установка Python зависимостей (pyproject)".into(),
                        enabled: !has_docker,
                        action_type: ActionType::RunCommand {
                            command: "pip install -e .".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    has_python_project = true;
                }
                "main.py" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Запустить main.py".into(),
                        enabled: !has_docker,
                        action_type: ActionType::RunCommand {
                            command: "python main.py".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    has_python_project = true;
                }
                "manage.py" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Запустить Django сервер".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: "python manage.py runserver".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Миграция Django БД".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: "python manage.py migrate".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    has_python_project = true;
                }
                // ============================================================
                // JAVA (Gradle)
                // ============================================================
                "build.gradle" | "build.gradle.kts" => {
                    let content = fs::read_to_string(path).unwrap_or_default();
                    let is_spring =
                        content.contains("org.springframework.boot") || content.contains("spring-boot");
                    let cmd = if is_spring {
                        "./gradlew bootRun"
                    } else {
                        "./gradlew run"
                    };

                    let port = if is_spring { 8080 } else { 3000 };

                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: format!("Запустить Gradle проект"),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: cmd.into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });

                    if is_spring {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Ожидание порта Spring Boot".into(),
                            enabled: true,
                            action_type: ActionType::WaitForPort {
                                host: "127.0.0.1".into(),
                                port,
                                timeout_secs: 60,
                            },
                        });
                    }
                }
                // ============================================================
                // JAVA (Maven)
                // ============================================================
                "pom.xml" => {
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
                        label: format!("Запустить Maven проект"),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: cmd.into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });

                    if is_spring {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Ожидание порта Spring Boot".into(),
                            enabled: true,
                            action_type: ActionType::WaitForPort {
                                host: "127.0.0.1".into(),
                                port,
                                timeout_secs: 60,
                            },
                        });
                    }
                }
                // ============================================================
                // RUBY
                // ============================================================
                "Gemfile" => {
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Запустить Rails сервер".into(),
                        enabled: true,
                        action_type: ActionType::RunCommand {
                            command: "bundle exec rails server".into(),
                            working_dir: Some(current_working_dir.clone()),
                        },
                    });
                    launch_actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Ожидание порта Rails".into(),
                        enabled: true,
                        action_type: ActionType::WaitForPort {
                            host: "127.0.0.1".into(),
                            port: 3000,
                            timeout_secs: 30,
                        },
                    });
                }
                // ============================================================
                // MAKEFILE
                // ============================================================
                "Makefile" => {
                    if !has_makefile {
                        has_makefile = true;
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Запустить Makefile".into(),
                            enabled: true,
                            action_type: ActionType::RunCommand {
                                command: "make".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                    }
                }
                // ============================================================
                // index.html (запоминаем путь)
                // ============================================================
                "index.html" => {
                    has_index_html = true;
                    if let Some(p) = path.to_str() {
                        index_html_path = p.to_string();
                    }
                }
                // ============================================================
                // C# и .sln
                // ============================================================
                _ => {
                    // ============================================================
                    // C#
                    // ============================================================
                    if filename.ends_with(".csproj") {
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Запустить .NET проект".into(),
                            enabled: true,
                            action_type: ActionType::RunCommand {
                                command: "dotnet run".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: "Горячая перезагрузка .NET".into(),
                            enabled: true,
                            action_type: ActionType::RunCommand {
                                command: "dotnet watch".into(),
                                working_dir: Some(current_working_dir.clone()),
                            },
                        });
                    }

                    // ============================================================
                    // .sln (Visual Studio Solution)
                    // ============================================================
                    if filename.ends_with(".sln") && !has_sln {
                        has_sln = true;
                        launch_actions.push(LaunchAction {
                            id: generate_id(),
                            label: format!("Открыть решение {}", filename),
                            enabled: true,
                            action_type: ActionType::OpenApplication {
                                path: "devenv".into(),
                                args: Some(path.to_string_lossy().to_string()),
                            },
                        });
                    }
                }
            }
        }

        // ============================================================
        // Если ничего не нашли, но есть index.html — предложим открыть
        // ============================================================
        if launch_actions.is_empty() && has_index_html && !index_html_path.is_empty() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: "Открыть index.html".into(),
                enabled: true,
                action_type: ActionType::OpenUrl {
                    url: index_html_path,
                },
            });
        }

        // ============================================================
        // IDE: определяем, какая IDE лучше подходит
        // ============================================================
        let ide_name: &str;
        let ide_label: &str;
        let ide_fallback: &str;
        let ide_fallback_label: &str;

        if has_python_project {
            ide_name = "pycharm";
            ide_label = "Открыть в PyCharm";
            ide_fallback = "code";
            ide_fallback_label = "Открыть в VS Code (PyCharm не найден)";
        } else if has_sln {
            ide_name = "devenv";
            ide_label = "Открыть в Visual Studio";
            ide_fallback = "code";
            ide_fallback_label = "Открыть в VS Code";
        } else {
            ide_name = "code";
            ide_label = "Открыть в VS Code";
            ide_fallback = "";
            ide_fallback_label = "";
        }

        if which::which(ide_name).is_ok() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: ide_label.into(),
                enabled: true,
                action_type: ActionType::OpenApplication {
                    path: ide_name.into(),
                    args: Some(".".into()),
                },
            });
        } else if !ide_fallback.is_empty() && which::which(ide_fallback).is_ok() {
            launch_actions.push(LaunchAction {
                id: generate_id(),
                label: ide_fallback_label.into(),
                enabled: true,
                action_type: ActionType::OpenApplication {
                    path: ide_fallback.into(),
                    args: Some(".".into()),
                },
            });
        }

        // ============================================================
        // Формируем имя проекта
        // ============================================================
        let project_name = Path::new(project_path)
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .unwrap_or("Проект");

        Ok(LaunchProfile {
            name: format!("Проект {}", project_name),
            description: format!("Автоматически найденный профиль для {}", project_path),
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
