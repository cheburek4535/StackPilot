use crate::modules::devlauncher::analyzer::{AnalysisConfidence, AnalysisDiagnostic};
use crate::modules::devlauncher::models::*;
use crate::modules::project_creator::models::WizardContext;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Graph builder
// ---------------------------------------------------------------------------

struct GraphBuilder {
    steps: Vec<LaunchStep>,
    next_id: usize,
}

impl GraphBuilder {
    fn new() -> Self {
        Self {
            steps: Vec::new(),
            next_id: 0,
        }
    }

    /// Append a step to the graph and return its deterministic ID.
    /// Positional on purpose: every step carries the same explicit
    /// field set (label, kind, deps, working dir, visibility, mode,
    /// completion, timeout, enabled, metadata).
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        label: &str,
        kind: StepKind,
        depends_on: Vec<String>,
        working_directory: Option<String>,
        visibility: Option<Visibility>,
        execution_mode: Option<ExecutionMode>,
        completion: Option<CompletionPolicy>,
        timeout: Option<u64>,
        enabled: bool,
        metadata: Option<HashMap<String, String>>,
    ) -> String {
        self.next_id += 1;
        let id = format!("step_{:03}", self.next_id);
        let failure_policy = None; // resolved in a second pass below
        self.steps.push(LaunchStep {
            id: id.clone(),
            label: label.to_string(),
            enabled,
            kind,
            depends_on,
            working_directory,
            environment: None,
            visibility,
            execution_mode,
            completion,
            timeout,
            failure_policy,
            retry_policy: None,
            metadata,
            extra: serde_json::Map::new(),
        });
        id
    }

    /// Resolve default failure policies now that the full graph is known:
    /// steps with dependents default to StopRun, leaves to WarnAndContinue.
    fn resolve_failure_policies(&mut self) {
        let mut dependents: HashMap<String, usize> = HashMap::new();
        for step in &self.steps {
            for dep in &step.depends_on {
                *dependents.entry(dep.clone()).or_insert(0) += 1;
            }
        }
        for step in &mut self.steps {
            let has_dependents = dependents.get(&step.id).map(|n| *n > 0).unwrap_or(false);
            step.failure_policy = Some(StepKind::default_failure_policy(has_dependents));
        }
    }
}

// ---------------------------------------------------------------------------
// Step constructors
// ---------------------------------------------------------------------------

fn meta(entries: &[(&str, &str)]) -> Option<HashMap<String, String>> {
    if entries.is_empty() {
        return None;
    }
    let mut map = HashMap::new();
    for (k, v) in entries {
        map.insert(k.to_string(), v.to_string());
    }
    Some(map)
}

/// A long-running service launched in a user-visible native terminal.
fn service_step(
    g: &mut GraphBuilder,
    label: &str,
    command: &str,
    working_dir: Option<&str>,
    depends_on: Vec<String>,
    confidence: &str,
) -> String {
    let mut metadata = HashMap::new();
    metadata.insert("confidence".to_string(), confidence.to_string());
    metadata.insert("visibility".to_string(), "visible_terminal".to_string());
    g.push(
        label,
        StepKind::RunCommand {
            command: command.to_string(),
            command_spec: None,
        },
        depends_on,
        working_dir.map(String::from),
        Some(Visibility::VisibleTerminal),
        Some(ExecutionMode::LongRunning),
        Some(CompletionPolicy::ProcessStarted),
        None,
        true,
        Some(metadata),
    )
}

/// A one-shot command (install, migrate, build). Installation and other
/// state-changing actions are DISABLED unless explicitly enabled.
fn one_shot_step(
    g: &mut GraphBuilder,
    label: &str,
    command: &str,
    working_dir: Option<&str>,
    depends_on: Vec<String>,
    enabled: bool,
) -> String {
    g.push(
        label,
        StepKind::RunCommand {
            command: command.to_string(),
            command_spec: None,
        },
        depends_on,
        working_dir.map(String::from),
        Some(Visibility::Captured),
        Some(ExecutionMode::OneShot),
        Some(CompletionPolicy::ExitSuccess),
        None,
        enabled,
        meta(&[("policy", if enabled { "run" } else { "manual-enable" })]),
    )
}

/// A readiness wait on a TCP port.
fn wait_port_step(
    g: &mut GraphBuilder,
    label: &str,
    port: u16,
    timeout_secs: u64,
    depends_on: String,
    confidence: &str,
) -> String {
    g.push(
        label,
        StepKind::WaitForPort {
            host: "127.0.0.1".to_string(),
            port,
        },
        vec![depends_on],
        None,
        None,
        None,
        Some(CompletionPolicy::PortOpen {
            host: "127.0.0.1".to_string(),
            port,
            timeout_secs,
        }),
        Some(timeout_secs),
        true,
        meta(&[("confidence", confidence), ("source", "framework default")]),
    )
}

fn open_app_step(
    g: &mut GraphBuilder,
    label: &str,
    path: &str,
    args: Vec<String>,
    depends_on: Vec<String>,
) -> String {
    g.push(
        label,
        StepKind::OpenApplication {
            path: path.to_string(),
            args: if args.is_empty() { None } else { Some(args) },
        },
        depends_on,
        None,
        Some(Visibility::Detached),
        None,
        Some(CompletionPolicy::ExternalLaunchAccepted),
        None,
        true,
        meta(&[("kind", "external_app")]),
    )
}

fn open_url_step(
    g: &mut GraphBuilder,
    label: &str,
    url: &str,
    depends_on: Vec<String>,
    source: &str,
) -> String {
    g.push(
        label,
        StepKind::OpenUrl {
            url: url.to_string(),
        },
        depends_on,
        None,
        Some(Visibility::Detached),
        None,
        Some(CompletionPolicy::ExternalLaunchAccepted),
        None,
        true,
        meta(&[("source", source), ("confidence", "medium")]),
    )
}

fn wait_docker_step(g: &mut GraphBuilder) -> String {
    g.push(
        "Wait for Docker daemon",
        StepKind::WaitForDocker {},
        Vec::new(),
        None,
        None,
        None,
        None,
        Some(30),
        true,
        meta(&[
            ("confidence", "high"),
            ("source", "docker tooling selected"),
        ]),
    )
}

fn open_terminal_step(g: &mut GraphBuilder) -> String {
    g.push(
        "Open a terminal",
        StepKind::OpenTerminal {
            command: String::new(),
        },
        Vec::new(),
        None,
        Some(Visibility::VisibleTerminal),
        Some(ExecutionMode::LongRunning),
        Some(CompletionPolicy::ProcessStarted),
        None,
        true,
        meta(&[("kind", "plain_terminal")]),
    )
}

// ---------------------------------------------------------------------------
// Framework knowledge
// ---------------------------------------------------------------------------

/// Tools that are deployed via docker-compose, with their default host port.
const DOCKER_TOOL_PORTS: &[(&str, u16)] = &[
    ("postgresql", 5432),
    ("postgres", 5432),
    ("redis", 6379),
    ("mongodb", 27017),
    ("mysql", 3306),
    ("elasticsearch", 9200),
    ("rabbitmq", 5672),
    ("kafka", 9092),
    ("redpanda", 9092),
];

/// Whether a wizard tool id is deployed as a compose service.
fn is_docker_tool(tool: &str) -> bool {
    DOCKER_TOOL_PORTS.iter().any(|(id, _)| *id == tool)
}

fn has_docker_tools(ctx: &WizardContext) -> bool {
    ctx.tools.iter().any(|t| is_docker_tool(t))
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

/// Working directory for a framework side: `./backend` / `./frontend` only
/// when the wizard configured a split architecture (both sides present);
/// otherwise the project root (None). The wizard itself creates these
/// directories, so they are explicit relative paths anchored to the
/// profile's `project_root`.
fn side_dir(ctx: &WizardContext, side: &str) -> Option<String> {
    let has_backend = !backend_frameworks(ctx).is_empty();
    let has_frontend = !frontend_frameworks(ctx).is_empty();
    if has_backend && has_frontend {
        Some(format!("./{}", side))
    } else {
        None
    }
}

fn has_language(ctx: &WizardContext, lang: &str) -> bool {
    ctx.languages.iter().any(|l| l == lang)
}

/// Pick the wizard's most likely preferred IDE.
fn preferred_ide_for(ctx: &WizardContext) -> Option<PreferredIde> {
    if has_language(ctx, "python") {
        Some(PreferredIde::Pycharm)
    } else if has_language(ctx, "go") {
        Some(PreferredIde::Goland)
    } else if has_language(ctx, "java") || has_language(ctx, "kotlin") {
        Some(PreferredIde::Idea)
    } else if has_language(ctx, "swift") || has_language(ctx, "objective-c") {
        Some(PreferredIde::Xcode)
    } else if has_language(ctx, "csharp") || has_language(ctx, "fsharp") || has_language(ctx, "vb")
    {
        Some(PreferredIde::VisualStudio)
    } else if has_language(ctx, "typescript") || has_language(ctx, "javascript") {
        Some(PreferredIde::Webstorm)
    } else {
        Some(PreferredIde::Vscode)
    }
}

/// Resolve an IDE/app CLI to an executable when possible.
fn resolve_app(name: &str) -> Option<String> {
    crate::platform::ide::resolve_ide_executable(name)
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a coherent V2 step graph from a wizard context.
///
/// The graph is structured like a real development session:
///   1. IDE/tool windows open first (independent roots).
///   2. Docker infrastructure: Docker Desktop → wait for daemon →
///      compose up → wait for database readiness.
///   3. Backend service starts (visible terminal), gated by the
///      infrastructure they need.
///   4. Readiness waits on backend ports, then API docs open.
///   5. Frontend dev servers (visible terminal) run in parallel.
///   6. A plain terminal is opened for ad-hoc commands.
///
/// Nothing is forced: IDE and tool steps are only emitted when the
/// application is resolvable on the host; install/migrate actions are
/// DISABLED by default (state-changing); the tools remain configurable.
pub fn build_profile_v2_from_context(
    ctx: &WizardContext,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
) -> LaunchProfileV2 {
    let project_path = ctx
        .project_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
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

    let mut g = GraphBuilder::new();
    let docker = ctx.docker || has_docker_tools(ctx);
    let has_backend = !backend_frameworks(ctx).is_empty();
    let has_frontend = !frontend_frameworks(ctx).is_empty();

    // --- 1. IDE / tool windows (roots) ---
    let effective_vscode = "code";
    let mut ide_opened = false;
    if let Some(resolved) = resolve_app(effective_vscode) {
        open_app_step(
            &mut g,
            "Open VS Code",
            &resolved,
            vec![".".to_string()],
            Vec::new(),
        );
        ide_opened = true;
    } else {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::Medium,
            "VS Code not resolvable; 'Open VS Code' step omitted (tools stay configurable)"
                .to_string(),
            None,
        ));
    }
    let has_mobile = frontend_frameworks(ctx)
        .iter()
        .any(|f| f == "expo" || f == "react-native");
    if has_mobile {
        let studio = if cfg!(target_os = "windows") {
            "studio64"
        } else if cfg!(target_os = "macos") {
            "studio"
        } else {
            "android-studio"
        };
        if let Some(resolved) = resolve_app(studio) {
            open_app_step(
                &mut g,
                "Open Android Studio",
                &resolved,
                vec![".".to_string()],
                Vec::new(),
            );
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Medium,
                "Android Studio not resolvable; mobile IDE step omitted".to_string(),
                None,
            ));
        }
    }
    let _ = ide_opened;

    // --- 2. Docker infrastructure ---
    let docker_wait: Option<String>;
    let mut compose: Option<String> = None;
    if docker {
        let docker_app = if cfg!(target_os = "windows") {
            "Docker Desktop"
        } else {
            "docker"
        };
        if let Some(resolved) = resolve_app(docker_app) {
            open_app_step(
                &mut g,
                "Open Docker Desktop",
                &resolved,
                Vec::new(),
                Vec::new(),
            );
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Low,
                "Docker Desktop not resolvable; daemon wait still applies".to_string(),
                None,
            ));
        }
        docker_wait = Some(wait_docker_step(&mut g));

        compose = Some(g.push(
            "Start Docker Compose",
            StepKind::RunCommand {
                command: "docker compose up -d".to_string(),
                command_spec: None,
            },
            vec![docker_wait.clone().unwrap()],
            None,
            Some(Visibility::Captured),
            Some(ExecutionMode::OneShot),
            Some(CompletionPolicy::ExitSuccess),
            None,
            true,
            meta(&[("kind", "infrastructure")]),
        ));

        // Database readiness waits for compose-managed tools.
        let mut db_waits: Vec<String> = Vec::new();
        for tool in &ctx.tools {
            if !is_docker_tool(tool) {
                continue;
            }
            // Tools deployed locally are NOT in the compose file — no wait.
            if ctx.local_infra_tools.iter().any(|t| t == tool) {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::High,
                    format!(
                        "Tool '{}' is configured for local installation; no compose wait generated",
                        tool
                    ),
                    None,
                ));
                continue;
            }
            let port = DOCKER_TOOL_PORTS
                .iter()
                .find(|(id, _)| id == tool)
                .map(|(_, p)| *p)
                .unwrap_or(0);
            if port == 0 {
                continue;
            }
            let id = wait_port_step(
                &mut g,
                &format!("Wait for {} readiness", tool),
                port,
                60,
                compose.clone().unwrap(),
                "low",
            );
            db_waits.push(id);
        }

        // DBeaver opens after the daemon is up (if resolvable).
        if !db_waits.is_empty() {
            let dbeaver = if cfg!(target_os = "windows") {
                "dbeaver"
            } else {
                "dbeaver-ce"
            };
            if let Some(resolved) = resolve_app(dbeaver) {
                open_app_step(
                    &mut g,
                    "Open DBeaver",
                    &resolved,
                    Vec::new(),
                    vec![docker_wait.clone().unwrap()],
                );
            } else {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::Low,
                    "DBeaver not resolvable; database client step omitted".to_string(),
                    None,
                ));
            }
        }
    }

    // --- 3. Backend services ---
    let mut backend_waits: Vec<String> = Vec::new();
    for fw in &backend_frameworks(ctx) {
        let dir = side_dir(ctx, "backend");
        let infra: Vec<String> = compose.clone().into_iter().collect();
        match fw.as_str() {
            "express" | "fastify" | "hono" | "nestjs" | "nest" => {
                let install = one_shot_step(
                    &mut g,
                    "Install backend dependencies",
                    "npm install",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start backend",
                    if fw == "nestjs" || fw == "nest" {
                        "npm run start:dev"
                    } else {
                        "npm run dev"
                    },
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let port = 3000;
                let wait = wait_port_step(&mut g, "Wait for backend port", port, 30, start, "low");
                backend_waits.push(wait.clone());
                if fw == "nestjs" || fw == "nest" {
                    let docs = open_url_step(
                        &mut g,
                        "Open NestJS Swagger docs",
                        &format!("http://localhost:{}/docs", port),
                        vec![wait],
                        "nestjs @nestjs/swagger default /docs",
                    );
                    let _ = docs;
                }
            }
            "fastapi" | "flask" | "litestar" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Python dependencies",
                    "pip install -r requirements.txt",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let cmd = if fw == "fastapi" {
                    "uvicorn main:app --reload"
                } else if fw == "flask" {
                    "flask run --debug"
                } else {
                    "litestar run --reload"
                };
                let start = service_step(
                    &mut g,
                    "Start Python backend",
                    cmd,
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let port = if fw == "flask" { 5000 } else { 8000 };
                let wait = wait_port_step(&mut g, "Wait for backend port", port, 30, start, "low");
                backend_waits.push(wait.clone());
                if fw == "fastapi" {
                    let docs = open_url_step(
                        &mut g,
                        "Open FastAPI docs",
                        &format!("http://localhost:{}/docs", port),
                        vec![wait],
                        "fastapi default /docs",
                    );
                    let _ = docs;
                }
            }
            "django" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Python dependencies",
                    "pip install -r requirements.txt",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let migrate = one_shot_step(
                    &mut g,
                    "Run Django database migrations (disabled by default)",
                    "python manage.py migrate",
                    dir.as_deref(),
                    vec![install],
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Django server",
                    "python manage.py runserver",
                    dir.as_deref(),
                    vec![migrate],
                    "high",
                );
                let wait = wait_port_step(&mut g, "Wait for Django port", 8000, 30, start, "low");
                backend_waits.push(wait);
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    AnalysisConfidence::High,
                    "Django migrate step is DISABLED (state-changing action); enable it manually"
                        .to_string(),
                    None,
                ));
            }
            "gin" | "echo" | "fiber" | "chi" | "gorilla" | "actix-web" => {
                let start = service_step(
                    &mut g,
                    "Start Go backend",
                    "go run .",
                    dir.as_deref(),
                    infra,
                    "high",
                );
                let port = 8080;
                let wait =
                    wait_port_step(&mut g, "Wait for Go backend port", port, 30, start, "low");
                backend_waits.push(wait.clone());
                // Swagger (swag) is the common Go API-docs convention.
                let docs = open_url_step(
                    &mut g,
                    "Open Swagger docs",
                    &format!("http://localhost:{}/swagger/index.html", port),
                    vec![wait],
                    "swag default /swagger/index.html",
                );
                let _ = docs;
            }
            "axum" | "actix" | "rocket" | "warp" | "tauri" => {
                let cmd = if fw == "tauri" {
                    "cargo tauri dev"
                } else {
                    "cargo run"
                };
                let start = service_step(
                    &mut g,
                    "Start Rust backend",
                    cmd,
                    dir.as_deref(),
                    infra,
                    "high",
                );
                if fw != "tauri" {
                    let wait = wait_port_step(
                        &mut g,
                        "Wait for Rust backend port",
                        3000,
                        30,
                        start,
                        "low",
                    );
                    backend_waits.push(wait);
                }
            }
            "spring-boot" | "spring" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Java dependencies",
                    "./gradlew build -x test",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Spring Boot",
                    "./gradlew bootRun",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait =
                    wait_port_step(&mut g, "Wait for Spring Boot port", 8080, 60, start, "low");
                backend_waits.push(wait.clone());
                let docs = open_url_step(
                    &mut g,
                    "Open Spring Boot Swagger docs",
                    "http://localhost:8080/swagger-ui/index.html",
                    vec![wait],
                    "springdoc default /swagger-ui/index.html",
                );
                let _ = docs;
            }
            "aspnet" | "blazor" => {
                let start = service_step(
                    &mut g,
                    "Start .NET backend",
                    "dotnet run",
                    dir.as_deref(),
                    infra,
                    "high",
                );
                let wait =
                    wait_port_step(&mut g, "Wait for .NET backend port", 5000, 30, start, "low");
                backend_waits.push(wait);
            }
            "rails" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Ruby dependencies",
                    "bundle install",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Rails server",
                    "bin/rails server",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(&mut g, "Wait for Rails port", 3000, 30, start, "low");
                backend_waits.push(wait);
            }
            _ => {}
        }
    }

    // --- 4. Frontend services (parallel roots unless they need infra) ---
    for fw in &frontend_frameworks(ctx) {
        let dir = side_dir(ctx, "frontend");
        match fw.as_str() {
            "nextjs" | "next" | "nuxt" | "nuxtjs" | "vite" | "vite-react" | "vite-vue"
            | "vite-svelte" | "react" | "vue" | "svelte" | "solid" => {
                let install = one_shot_step(
                    &mut g,
                    "Install frontend dependencies",
                    "npm install",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start frontend dev server",
                    "npm run dev",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let port = if fw == "next" || fw == "nextjs" || fw == "nuxt" || fw == "nuxtjs" {
                    3000
                } else if fw == "angular" {
                    4200
                } else {
                    5173
                };
                let wait = wait_port_step(&mut g, "Wait for frontend port", port, 30, start, "low");
                let _ = wait;
            }
            "angular" => {
                let install = one_shot_step(
                    &mut g,
                    "Install frontend dependencies",
                    "npm install",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Angular dev server",
                    "npm run start",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(&mut g, "Wait for Angular port", 4200, 30, start, "low");
                let _ = wait;
            }
            "expo" | "react-native" => {
                let start = service_step(
                    &mut g,
                    if fw == "expo" {
                        "Start Expo"
                    } else {
                        "Start React Native"
                    },
                    if fw == "expo" {
                        "npx expo start"
                    } else {
                        "npx react-native start"
                    },
                    dir.as_deref(),
                    Vec::new(),
                    "high",
                );
                let wait = wait_port_step(&mut g, "Wait for Metro bundler", 8081, 30, start, "low");
                let _ = wait;
            }
            "electron" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Electron dependencies",
                    "npm install",
                    dir.as_deref(),
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Electron app",
                    "npm run dev",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let _ = start;
            }
            _ => {}
        }
    }

    // --- 5. Language fallback when no frameworks were selected ---
    if !has_backend && !has_frontend {
        build_language_steps(ctx, &mut g);
    }

    // --- 6. Open a plain terminal for ad-hoc commands ---
    open_terminal_step(&mut g);

    g.resolve_failure_policies();

    // Attach per-step diagnostics as metadata and collect warnings.
    for step in &g.steps {
        if !step.enabled {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Warning,
                AnalysisConfidence::High,
                format!("Step '{}' is DISABLED; enable it manually", step.label),
                Some(step.id.clone()),
            ));
        }
    }

    let description = build_description(ctx);
    LaunchProfileV2 {
        schema_version: PROFILE_SCHEMA_VERSION.to_string(),
        id: generate_stable_id(),
        name: project_name,
        description,
        project_root: if project_path.is_empty() {
            None
        } else {
            Some(project_path)
        },
        steps: g.steps,
        environment_binding_id: ctx.environment_binding_id.clone(),
        preferred_ide: preferred_ide_for(ctx),
        default_execution_mode: None,
        extra: serde_json::Map::new(),
    }
}

fn build_language_steps(ctx: &WizardContext, g: &mut GraphBuilder) {
    for lang in &ctx.languages {
        match lang.as_str() {
            "python" => {
                let install = one_shot_step(
                    g,
                    "Install Python dependencies",
                    "pip install -r requirements.txt",
                    None,
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    g,
                    "Run Python project",
                    "python main.py",
                    None,
                    vec![install],
                    "medium",
                );
                let _ = start;
            }
            "go" => {
                let start =
                    service_step(g, "Run Go project", "go run .", None, Vec::new(), "medium");
                let wait = wait_port_step(g, "Wait for Go port", 8080, 30, start, "low");
                let _ = wait;
            }
            "rust" => {
                let start = service_step(
                    g,
                    "Run Rust project",
                    "cargo run",
                    None,
                    Vec::new(),
                    "medium",
                );
                let _ = start;
            }
            "typescript" | "javascript" => {
                let install = one_shot_step(
                    g,
                    "Install Node.js dependencies",
                    "npm install",
                    None,
                    Vec::new(),
                    false,
                );
                let start = service_step(
                    g,
                    "Run Node.js project",
                    "npm run dev",
                    None,
                    vec![install],
                    "medium",
                );
                let _ = start;
            }
            "java" | "kotlin" => {
                let start = service_step(
                    g,
                    "Run JVM project",
                    "./gradlew run",
                    None,
                    Vec::new(),
                    "medium",
                );
                let _ = start;
            }
            "csharp" | "fsharp" => {
                let start = service_step(
                    g,
                    "Run .NET project",
                    "dotnet run",
                    None,
                    Vec::new(),
                    "medium",
                );
                let _ = start;
            }
            "ruby" => {
                let start = service_step(
                    g,
                    "Run Ruby project",
                    "ruby main.rb",
                    None,
                    Vec::new(),
                    "medium",
                );
                let _ = start;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy entry point
// ---------------------------------------------------------------------------

/// Legacy API: build a legacy `LaunchProfile` from the context. Used by the
/// existing `build_profile_from_context` Tauri command for backward
/// compatibility; the V2 graph is preserved internally.
///
/// Retained even though commands now use `build_profile_v2_from_context`,
/// so third-party/embedded callers keep a stable entry point.
#[allow(dead_code)]
pub fn build_profile_from_context(ctx: &WizardContext) -> LaunchProfile {
    let mut diagnostics: Vec<AnalysisDiagnostic> = Vec::new();
    let v2 = build_profile_v2_from_context(ctx, &mut diagnostics);
    LaunchProfile::from(v2)
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
        parts.push(format!(
            "{} framework{}",
            fw_count,
            if fw_count > 1 { "s" } else { "" }
        ));
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ctx(languages: &[&str], frameworks: &[&str], tools: &[&str], docker: bool) -> WizardContext {
        WizardContext {
            project_path: Some(Path::new("/proj").to_path_buf()),
            project_name: Some("TestApp".to_string()),
            is_existing: false,
            project_type: Some("rest-api".to_string()),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            backend_languages: Vec::new(),
            frontend_languages: Vec::new(),
            frameworks: frameworks.iter().map(|s| s.to_string()).collect(),
            tools: tools.iter().map(|s| s.to_string()).collect(),
            local_infra_tools: Vec::new(),
            features: Vec::new(),
            infrastructure: Vec::new(),
            docker,
            testing: false,
            ci: false,
            git_init: false,
            vscode_config: false,
            answers: Default::default(),
            environment_binding_id: None,
        }
    }

    fn build(c: &WizardContext) -> (LaunchProfileV2, Vec<AnalysisDiagnostic>) {
        let mut diagnostics = Vec::new();
        let profile = build_profile_v2_from_context(c, &mut diagnostics);
        (profile, diagnostics)
    }

    #[test]
    fn expo_plus_go_plus_docker_coherent_graph() {
        let c = ctx(
            &["typescript", "go"],
            &["expo", "gin"],
            &["postgresql"],
            true,
        );
        let (profile, _diags) = build(&c);
        let labels: Vec<&str> = profile.steps.iter().map(|s| s.label.as_str()).collect();
        let has = |needle: &str| labels.iter().any(|l| l.contains(needle));

        // Conceptually similar to the contract example graph.
        assert!(has("Docker daemon"), "{:?}", labels);
        assert!(has("Start Docker Compose"), "{:?}", labels);
        assert!(has("postgresql readiness"), "{:?}", labels);
        assert!(has("Go backend"), "{:?}", labels);
        assert!(has("Go backend port"), "{:?}", labels);
        assert!(has("Swagger docs"), "{:?}", labels);
        assert!(has("Start Expo"), "{:?}", labels);
        assert!(has("Metro bundler"), "{:?}", labels);
        assert!(has("Open a terminal"), "{:?}", labels);

        // Dependency graph invariants.
        let compose = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Docker Compose"))
            .unwrap();
        let daemon = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Docker daemon"))
            .unwrap();
        assert!(compose.depends_on.contains(&daemon.id));
        let db = profile
            .steps
            .iter()
            .find(|s| s.label.contains("postgresql readiness"))
            .unwrap();
        assert!(db.depends_on.contains(&compose.id));
        let go = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Go backend") && !s.label.contains("port"))
            .unwrap();
        assert!(go.depends_on.contains(&compose.id));
        let go_wait = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Go backend port"))
            .unwrap();
        assert!(go_wait.depends_on.contains(&go.id));
        let swagger = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Swagger docs"))
            .unwrap();
        assert!(swagger.depends_on.contains(&go_wait.id));

        // No `enabled: !has_docker` suppression: docker, backend, frontend
        // coexist, all enabled.
        assert!(compose.enabled && go.enabled);
        let expo = profile
            .steps
            .iter()
            .find(|s| s.label == "Start Expo")
            .unwrap();
        assert!(expo.enabled);

        // Graph validates (no cycles, no missing deps).
        let result = crate::modules::devlauncher::validation::validate_profile_v2(&profile);
        assert!(result.valid, "{:?}", result.diagnostics);
    }

    #[test]
    fn api_docs_enabled_when_inferable() {
        let c = ctx(&["python"], &["fastapi"], &[], false);
        let (profile, _) = build(&c);
        let docs = profile
            .steps
            .iter()
            .find(|s| s.label.contains("FastAPI docs"))
            .expect("docs step");
        assert!(docs.enabled);
        match &docs.kind {
            StepKind::OpenUrl { url } => assert_eq!(url, "http://localhost:8000/docs"),
            other => panic!("expected OpenUrl, got {:?}", other),
        }
    }

    #[test]
    fn no_frameworks_falls_back_to_language_steps() {
        let c = ctx(&["go"], &[], &[], false);
        let (profile, _) = build(&c);
        assert!(profile
            .steps
            .iter()
            .any(|s| s.label.contains("Run Go project")));
    }

    #[test]
    fn install_and_migrate_steps_disabled_by_default() {
        let c = ctx(&["python"], &["django"], &[], false);
        let (profile, diags) = build(&c);
        let migrate = profile
            .steps
            .iter()
            .find(|s| s.label.contains("migrations"))
            .expect("migrate step");
        assert!(!migrate.enabled);
        assert_eq!(
            migrate.metadata.as_ref().unwrap().get("policy").unwrap(),
            "manual-enable"
        );
        // Diagnostics mention the disabled step.
        assert!(diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning));
    }

    #[test]
    fn steps_carry_explicit_fields() {
        let c = ctx(&["typescript"], &["nestjs"], &["redis"], true);
        let (profile, _) = build(&c);
        for step in &profile.steps {
            assert!(!step.label.is_empty());
            assert!(!step.id.is_empty());
            assert!(step.failure_policy.is_some(), "{}", step.label);
            match step.kind {
                StepKind::RunCommand { .. } => {
                    assert!(
                        step.visibility.is_some() && step.completion.is_some(),
                        "{}",
                        step.label
                    );
                    assert!(step.execution_mode.is_some(), "{}", step.label);
                }
                StepKind::WaitForPort { .. } => {
                    assert!(step.timeout.is_some(), "{}", step.label);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn local_infra_tools_skip_compose_waits() {
        let mut c = ctx(&["typescript"], &["nestjs"], &["redis"], true);
        c.local_infra_tools = vec!["redis".to_string()];
        let (profile, diags) = build(&c);
        assert!(!profile
            .steps
            .iter()
            .any(|s| s.label.contains("redis readiness")));
        assert!(diags
            .iter()
            .any(|d| d.message.contains("local installation")));
    }

    #[test]
    fn legacy_entry_point_round_trips() {
        let c = ctx(
            &["typescript", "go"],
            &["expo", "gin"],
            &["postgresql"],
            true,
        );
        let legacy = build_profile_from_context(&c);
        assert_eq!(legacy.name, "TestApp");
        assert_eq!(legacy.project_path.as_deref(), Some("/proj"));
        assert!(!legacy.actions.is_empty());
        // Preferred IDE preserved.
        assert!(matches!(legacy.preferred_ide, Some(PreferredIde::Goland)));
    }

    #[test]
    fn preferred_ide_selection() {
        let c = ctx(&["python"], &["fastapi"], &[], false);
        assert!(matches!(preferred_ide_for(&c), Some(PreferredIde::Pycharm)));
        let c = ctx(&["go"], &["gin"], &[], false);
        assert!(matches!(preferred_ide_for(&c), Some(PreferredIde::Goland)));
        let c = ctx(&["csharp"], &["aspnet"], &[], false);
        assert!(matches!(
            preferred_ide_for(&c),
            Some(PreferredIde::VisualStudio)
        ));
        let c = ctx(&["typescript"], &["expo"], &[], false);
        assert!(matches!(
            preferred_ide_for(&c),
            Some(PreferredIde::Webstorm)
        ));
    }
}
