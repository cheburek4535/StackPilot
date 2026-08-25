use crate::modules::devlauncher::models::*;
use crate::modules::project_creator::models::WizardContext;

/// Builds a LaunchProfile directly from WizardContext without filesystem
/// analysis. The Project Creator already knows the full stack, so we map
/// frameworks, tools, and project structure into precise launch actions.
pub fn build_profile_from_context(ctx: &WizardContext) -> LaunchProfile {
    let project_path = ctx
        .project_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let project_name = ctx
        .project_name
        .as_deref()
        .or_else(|| {
            ctx.project_path
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
        })
        .unwrap_or("Project")
        .to_string();

    let mut actions: Vec<LaunchAction> = Vec::new();

    // 1. Docker infrastructure (if docker tools selected)
    if ctx.docker || has_docker_tools(ctx) {
        actions.push(LaunchAction {
            id: generate_id(),
            label: "Start Docker Compose".into(),
            enabled: true,
            action_type: ActionType::RunCommand {
                command: "docker compose up -d".into(),
                working_dir: None,
                persistent: None,
            },
        });
    }

    // 2. Backend actions based on framework
    let backend_frameworks = backend_frameworks(ctx);
    let frontend_frameworks = frontend_frameworks(ctx);

    for fw_id in &backend_frameworks {
        match fw_id.as_str() {
            // Node.js / Express / NestJS / Fastify
            "express" | "fastify" | "hono" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install backend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for backend port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                });
            }
            "nestjs" | "nest" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install backend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start NestJS backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run start:dev".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for NestJS port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                });
            }
            // Python / FastAPI / Django / Flask
            "fastapi" | "flask" | "litestar" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Python deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "pip install -r requirements.txt".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                let cmd = if fw_id == "fastapi" {
                    "uvicorn main:app --reload"
                } else if fw_id == "flask" {
                    "flask run --debug"
                } else {
                    "litestar run --reload"
                };
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Python backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: cmd.into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                let port = if fw_id == "flask" { 5000 } else { 8000 };
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for backend port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port,
                        timeout_secs: 30,
                    },
                });
            }
            "django" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Python deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "pip install -r requirements.txt".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Django DB migrate".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python manage.py migrate".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Django server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python manage.py runserver".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Django port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 8000,
                        timeout_secs: 30,
                    },
                });
            }
            // Go
            "gin" | "echo" | "fiber" | "chi" | "gorilla" | "hono-go" | "actix-web" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Go backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "go run .".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Go port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 8080,
                        timeout_secs: 30,
                    },
                });
            }
            // Rust
            "axum" | "actix" | "rocket" | "warp" | "tauri" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Rust backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: if fw_id == "tauri" {
                            "cargo tauri dev".into()
                        } else {
                            "cargo run".into()
                        },
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                if fw_id != "tauri" {
                    actions.push(LaunchAction {
                        id: generate_id(),
                        label: "Wait for Rust port".into(),
                        enabled: true,
                        action_type: ActionType::WaitForPort {
                            host: "127.0.0.1".into(),
                            port: 3000,
                            timeout_secs: 30,
                        },
                    });
                }
            }
            // Spring Boot (Java/Kotlin)
            "spring-boot" | "spring" => {
                let cmd = if has_kotlin(ctx) {
                    "./gradlew bootRun"
                } else {
                    "./gradlew bootRun"
                };
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Spring Boot".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: cmd.into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Spring port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 8080,
                        timeout_secs: 60,
                    },
                });
            }
            // .NET
            "aspnet" | "blazor" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start .NET backend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "dotnet run".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for .NET port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 5000,
                        timeout_secs: 30,
                    },
                });
            }
            // Rails
            "rails" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Ruby deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "bundle install".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Rails server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "bin/rails server".into(),
                        working_dir: subdir_path(ctx, "backend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
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
            _ => {}
        }
    }

    // 3. Frontend actions based on framework
    for fw_id in &frontend_frameworks {
        match fw_id.as_str() {
            "nextjs" | "next" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install frontend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Next.js frontend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Next.js port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                });
            }
            "nuxt" | "nuxtjs" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install frontend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Nuxt frontend".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Nuxt port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 3000,
                        timeout_secs: 30,
                    },
                });
            }
            "vite" | "vite-react" | "vite-vue" | "vite-svelte" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install frontend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Vite dev server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for Vite port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 5173,
                        timeout_secs: 30,
                    },
                });
            }
            "react" | "vue" | "svelte" | "angular" | "solid" => {
                // SPA frameworks without their own dev server — likely using Vite
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install frontend deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start frontend dev server".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: subdir_path(ctx, "frontend"),
                        persistent: Some(true),
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Wait for frontend port".into(),
                    enabled: true,
                    action_type: ActionType::WaitForPort {
                        host: "127.0.0.1".into(),
                        port: 5173,
                        timeout_secs: 30,
                    },
                });
            }
            // Mobile / Desktop shells
            "expo" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Expo".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npx expo start".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "electron" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Electron app".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            _ => {}
        }
    }

    // 4. Standalone fullstack frameworks (not split backend/frontend)
    if backend_frameworks.is_empty() && frontend_frameworks.is_empty() {
        // No frameworks selected — fall back to language-based actions
        build_language_actions(ctx, &mut actions);
    }

    // 5. Tool-specific actions (beyond docker)
    for tool_id in &ctx.tools {
        match tool_id.as_str() {
            "airflow" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Airflow".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "airflow standalone".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "kafka" | "redpanda" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Start Kafka/Redpanda".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "docker compose up -d".into(),
                        working_dir: None,
                        persistent: None,
                    },
                });
            }
            _ => {}
        }
    }

    // 6. Open API docs if applicable
    if has_web_backend(ctx) {
        actions.push(LaunchAction {
            id: generate_id(),
            label: "Open API docs".into(),
            enabled: false, // disabled by default, user can enable
            action_type: ActionType::OpenUrl {
                url: "http://localhost:8000/docs".into(),
            },
        });
    }

    // 7. Determine preferred IDE
    let preferred_ide = if has_python(ctx) && has_docker_tools(ctx) {
        Some(PreferredIde::Pycharm)
    } else {
        Some(PreferredIde::Vscode)
    };

    let description = build_description(ctx);

    LaunchProfile {
        name: project_name,
        description,
        project_path: Some(project_path),
        actions,
        environment_binding_id: ctx.environment_binding_id.clone(),
        preferred_ide,
    }
}

// ---- Helpers ----

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("act_{}", nanos)
}

fn has_docker_tools(ctx: &WizardContext) -> bool {
    ctx.tools.iter().any(|t| {
        matches!(
            t.as_str(),
            "postgresql" | "redis" | "mongodb" | "mysql" | "elasticsearch" | "rabbitmq"
        )
    })
}

fn backend_frameworks(ctx: &WizardContext) -> Vec<String> {
    ctx.frameworks
        .iter()
        .filter(|fw| is_backend_framework(fw))
        .cloned()
        .collect()
}

fn frontend_frameworks(ctx: &WizardContext) -> Vec<String> {
    ctx.frameworks
        .iter()
        .filter(|fw| is_frontend_framework(fw))
        .cloned()
        .collect()
}

fn is_backend_framework(fw: &str) -> bool {
    matches!(
        fw,
        "express"
            | "fastify"
            | "hono"
            | "nestjs"
            | "nest"
            | "fastapi"
            | "flask"
            | "django"
            | "litestar"
            | "gin"
            | "echo"
            | "fiber"
            | "chi"
            | "gorilla"
            | "axum"
            | "actix"
            | "rocket"
            | "warp"
            | "spring-boot"
            | "spring"
            | "aspnet"
            | "blazor"
            | "rails"
            | "tauri"
    )
}

fn is_frontend_framework(fw: &str) -> bool {
    matches!(
        fw,
        "nextjs"
            | "next"
            | "nuxt"
            | "nuxtjs"
            | "vite"
            | "vite-react"
            | "vite-vue"
            | "vite-svelte"
            | "react"
            | "vue"
            | "svelte"
            | "angular"
            | "solid"
            | "expo"
            | "electron"
            | "react-native"
    )
}

/// If the project has split architecture, return the subdir for the given side.
/// Otherwise return None (actions run in project root).
fn subdir_path(ctx: &WizardContext, side: &str) -> Option<String> {
    // Split architecture is detected by having both backend and frontend frameworks
    let has_backend = backend_frameworks(ctx).len() > 0;
    let has_frontend = frontend_frameworks(ctx).len() > 0;
    if has_backend && has_frontend {
        Some(format!("./{}", side))
    } else {
        None
    }
}

fn has_kotlin(ctx: &WizardContext) -> bool {
    ctx.languages.iter().any(|l| l == "kotlin")
}

fn has_python(ctx: &WizardContext) -> bool {
    ctx.languages.iter().any(|l| l == "python")
}

fn has_web_backend(ctx: &WizardContext) -> bool {
    backend_frameworks(ctx).iter().any(|fw| {
        matches!(
            fw.as_str(),
            "fastapi"
                | "flask"
                | "django"
                | "litestar"
                | "express"
                | "fastify"
                | "hono"
                | "nestjs"
                | "nest"
                | "spring-boot"
                | "spring"
                | "aspnet"
                | "rails"
                | "gin"
                | "echo"
                | "fiber"
                | "axum"
                | "actix"
                | "rocket"
        )
    })
}

fn build_language_actions(ctx: &WizardContext, actions: &mut Vec<LaunchAction>) {
    for lang in &ctx.languages {
        match lang.as_str() {
            "python" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Python deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "pip install -r requirements.txt".into(),
                        working_dir: None,
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Python project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "python main.py".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "go" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Go project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "go run .".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "rust" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Rust project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "cargo run".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "typescript" | "javascript" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Install Node.js deps".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm install".into(),
                        working_dir: None,
                        persistent: None,
                    },
                });
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Node.js project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "npm run dev".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "java" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Java project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "./gradlew run".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "csharp" | "fsharp" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run .NET project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "dotnet run".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "ruby" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Ruby project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "ruby main.rb".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            "kotlin" => {
                actions.push(LaunchAction {
                    id: generate_id(),
                    label: "Run Kotlin project".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "./gradlew run".into(),
                        working_dir: None,
                        persistent: Some(true),
                    },
                });
            }
            _ => {}
        }
    }
}

fn build_description(ctx: &WizardContext) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(pt) = &ctx.project_type {
        parts.push(pt.clone());
    }

    let langs: Vec<String> = ctx.languages.iter().take(3).cloned().collect();
    if !langs.is_empty() {
        parts.push(langs.join(", "));
    }

    let fw_count = ctx.frameworks.len();
    if fw_count > 0 {
        parts.push(format!("{} framework{}", fw_count, if fw_count > 1 { "s" } else { "" }));
    }

    if ctx.docker {
        parts.push("docker".into());
    }

    if parts.is_empty() {
        "Project profile".into()
    } else {
        parts.join(" · ")
    }
}
