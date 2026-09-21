use crate::modules::devlauncher::analyzer::{
    parse_compose_services, python_ensure_venv_install, python_venv_run, AnalysisConfidence,
    AnalysisDiagnostic,
};
use crate::modules::devlauncher::models::*;
use crate::modules::project_creator::models::WizardContext;
use crate::platform::docker_service::DockerService;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

    /// Attach a retry policy to an already-pushed step.
    fn set_retry_policy(&mut self, id: &str, policy: RetryPolicy) {
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == id) {
            step.retry_policy = Some(policy);
        }
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
            step.failure_policy = Some(step.kind.default_failure_policy(has_dependents));
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

/// A readiness wait on a TCP port. `candidate_ports` are extra ports that
/// also satisfy the wait (dev servers auto-increment when their primary port
/// is taken); the step succeeds when any of them opens.
fn wait_port_step(
    g: &mut GraphBuilder,
    label: &str,
    port: u16,
    candidate_ports: &[u16],
    timeout_secs: u64,
    depends_on: String,
    confidence: &str,
) -> String {
    let mut candidates: Vec<u16> = candidate_ports.to_vec();
    candidates.retain(|p| *p != port);
    candidates.dedup();
    let id = g.push(
        label,
        StepKind::WaitForPort {
            host: "127.0.0.1".to_string(),
            port,
            candidate_ports: candidates.clone(),
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
    );
    // Wait steps are retried with backoff: dev servers routinely need a
    // second chance on cold starts.
    g.set_retry_policy(
        &id,
        RetryPolicy {
            max_retries: 2,
            delay_ms: 2000,
            backoff_multiplier: Some(1.5),
        },
    );
    id
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
        // Cold Docker Desktop boot routinely exceeds 30s.
        Some(120),
        true,
        meta(&[
            ("confidence", "high"),
            ("source", "docker tooling selected"),
        ]),
    )
}

// Framework knowledge
// ---------------------------------------------------------------------------

/// Default timeout for generated port-readiness waits (dev servers need
/// headroom for cold starts; the retry policy re-arms twice with backoff).
const WAIT_PORT_TIMEOUT_SECS: u64 = 90;

/// Default timeout for the "Start Docker Compose" verification. Image
/// builds (npm ci, pip install, ...) routinely take minutes on cold
/// starts, so the step gets generous headroom вЂ” but it still fails fast
/// when the compose command itself exits non-zero (the startup probe
/// reports the real exit code as soon as the build dies).
const COMPOSE_UP_TIMEOUT_SECS: u64 = 600;

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

/// Compose tools that need a longer readiness timeout than the generic
/// 60s. Kafka (and its drop-in Redpanda) are the slowest containers to
/// become reachable: the image pull + broker bootstrap routinely exceeds
/// a minute on cold starts, so 60s times out while the broker is still
/// starting. The wait accepts `29092` as an alternative because many
/// Kafka/KRaft compose templates expose the host-facing listener there.
fn tool_wait_timeout_secs(tool: &str) -> u64 {
    if matches!(tool, "kafka" | "redpanda") {
        180
    } else {
        60
    }
}

fn tool_wait_candidate_ports(tool: &str, port: u16) -> &'static [u16] {
    if matches!(tool, "kafka" | "redpanda") && port == 9092 {
        &[29092]
    } else {
        &[]
    }
}

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
            | "laravel"
            | "symfony"
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
            | "android"
            | "solid"
            | "expo"
            | "electron"
            | "react-native"
            | "flutter"
            | "maui"
            | "swiftui"
    )
}

/// Whether a frontend framework runs on Vite (which binds 'localhost' and,
/// on modern Node, the IPv6 loopback only). The wizard's react/vue/svelte/
/// solid scaffolds all use Vite; Next/Nuxt own their own dev servers.
fn is_vite_family(fw: &str) -> bool {
    !matches!(
        fw,
        "nextjs" | "next" | "nuxt" | "nuxtjs" | "angular" | "expo" | "electron" | "react-native"
            | "flutter" | "maui" | "swiftui" | "android"
    )
}

/// Manifest files that prove a framework's code actually lives in a
/// directory. Used by the split-layout search below: a step is anchored to
/// the subdirectory that OWNS the required manifest, never to a guessed
/// folder name.
fn framework_manifests(fw: &str) -> &'static [&'static str] {
    match fw {
        // Rust backends / Tauri shells
        "axum" | "actix" | "actix-web" | "rocket" | "warp" | "tauri" => &["Cargo.toml"],
        // Go backends
        "gin" | "echo" | "fiber" | "chi" | "gorilla" => &["go.mod"],
        // Python backends
        "fastapi" | "flask" | "django" | "litestar" => {
            &["requirements.txt", "pyproject.toml", "Pipfile", "manage.py"]
        }
        // JVM backends
        "spring-boot" | "spring" | "ktor" => {
            &["pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle"]
        }
        // PHP backends
        "laravel" | "symfony" => &["composer.json", "artisan", "symfony.lock"],
        // Ruby backends
        "rails" => &["Gemfile", "config.ru"],
        // .NET backends
        "aspnet" | "aspnetcore" | "blazor" | "maui" => &["*.csproj", "*.fsproj", "*.sln"],
        // Everything Node-based (backends and frontends)
        _ => &["package.json"],
    }
}

/// Does `dir` contain any of the given manifest files? `*.csproj`-style
/// entries are matched against real filenames in the directory.
fn dir_has_manifest(dir: &Path, manifests: &[&str]) -> bool {
    manifests.iter().any(|m| {
        if let Some(glob) = m.strip_prefix('*') {
            let suffix = glob.to_ascii_lowercase();
            std::fs::read_dir(dir)
                .map(|entries| {
                    entries.flatten().any(|e| {
                        e.path()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.to_ascii_lowercase().ends_with(&suffix))
                    })
                })
                .unwrap_or(false)
        } else {
            dir.join(m).is_file()
        }
    })
}

/// Locate the working directory for a framework side in a SPLIT-structure
/// project: the project root when the manifest lives there, otherwise the
/// subdirectory that actually owns the framework's required manifest.
///
/// The wizard's own scaffold is explicit (`./backend` / `./frontend`), but
/// externally created projects routinely use other folder names (`server/`,
/// `api/`, `app/`...) or a single side without the other. Launching `cargo
/// run`/`npm run dev` from the project root then fails with the cryptic
/// "could not find Cargo.toml in <root> or any parent directory" вЂ” cargo
/// walks UP, never down. The search is deterministic and manifest-driven:
///
///   1. the root itself owns the manifest в†’ `None` (run in the root);
///   2. the canonical side folder (`backend`/`frontend`) owns it в†’ `./<side>`;
///   3. the first immediate subdirectory (sorted) that owns it в†’ `./<dir>`.
///
/// Returns a RELATIVE directory anchored to the profile's `project_root`,
/// or `None` when the project path is missing (nothing to anchor to) or the
/// side is not laid out in subdirectories.
fn locate_side_dir(root: &Path, side: &str, fw: &str) -> Option<String> {
    let manifests = framework_manifests(fw);
    if dir_has_manifest(root, manifests) {
        return None;
    }
    let preferred: &[&str] = if side == "frontend" {
        &["frontend", "client", "web", "ui"]
    } else {
        &["backend", "server", "api", "app"]
    };
    for name in preferred {
        let candidate = root.join(name);
        if candidate.is_dir() && dir_has_manifest(&candidate, manifests) {
            return Some(format!("./{}", name));
        }
    }
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    subdirs.sort();
    subdirs
        .iter()
        .find(|d| dir_has_manifest(d, manifests))
        .and_then(|d| d.strip_prefix(root).ok())
        .map(|rel| format!("./{}", rel.to_string_lossy().replace('\\', "/")))
}

/// Working directory for a framework side: the manifest-owned directory in
/// split-layout projects (see [`locate_side_dir`]); the project root (None)
/// when the framework's manifest lives there. The wizard itself creates
/// `backend/` / `frontend/` for split architectures, so the canonical names
/// are preferred вЂ” but the search never forces a folder that does not own
/// the side's required files.
///
/// When the project path is missing there is nothing to anchor a relative
/// dir to: a relative working directory would make every command run in the
/// app's own working directory ("The system cannot find the path
/// specified", cmd exit code 3). Emit `None` вЂ” the run-time preflight then
/// fails with the real reason.
fn side_dir(ctx: &WizardContext, side: &str, fw: &str) -> Option<String> {
    let Some(root) = ctx.project_path.as_ref() else {
        return None;
    };
    if !root.is_dir() {
        return None;
    }
    locate_side_dir(root, side, fw)
}

/// Pick the Go package containing the executable entry point.  Go projects
/// commonly keep binaries under `cmd/<name>/main.go`; `go run .` only works
/// when `main.go` is at the module root.
fn go_run_command(ctx: &WizardContext, working_dir: Option<&str>) -> String {
    let Some(root) = ctx.project_path.as_ref() else {
        return "go run .".to_string();
    };
    let dir = working_dir
        .map(|d| root.join(d))
        .unwrap_or_else(|| root.clone());
    if dir.join("main.go").is_file() {
        return "go run .".to_string();
    }
    let cmd_root = dir.join("cmd");
    if !cmd_root.is_dir() {
        return "go run .".to_string();
    }
    if cmd_root.join("main.go").is_file() {
        return "go run ./cmd".to_string();
    }
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&cmd_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("main.go").is_file() {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    candidates
        .first()
        .and_then(|p| p.strip_prefix(&dir).ok())
        .map(|p| format!("go run ./{}", p.to_string_lossy().replace('\\', "/")))
        .unwrap_or_else(|| "go run .".to_string())
}

/// Choose the package manager already used by a side of the project.  A
/// generated profile must respect pnpm/yarn/bun lockfiles; running `npm
/// install` against those projects can rewrite the lockfile or fail on
/// workspace-only manifests.
fn node_install_command(ctx: &WizardContext, working_dir: Option<&str>) -> String {
    let Some(root) = ctx.project_path.as_ref() else {
        return "npm install".to_string();
    };
    let dir = working_dir
        .map(|d| root.join(d))
        .unwrap_or_else(|| root.clone());
    if dir.join("pnpm-lock.yaml").is_file() {
        "pnpm install".to_string()
    } else if dir.join("yarn.lock").is_file() {
        "yarn install".to_string()
    } else if dir.join("bun.lockb").is_file() || dir.join("bun.lock").is_file() {
        "bun install".to_string()
    } else {
        "npm install".to_string()
    }
}

/// Commands emitted by the wizard must work in a freshly generated project
/// on every host.  Unix-style `./gradlew` is not executable by `cmd.exe`, and
/// Spring Initializr defaults to Maven even when Gradle is installed.
fn java_wrapper_command(ctx: &WizardContext, task: &str) -> String {
    let use_gradle =
        ctx.tools.iter().any(|t| t == "gradle") && !ctx.tools.iter().any(|t| t == "maven");
    if use_gradle {
        if cfg!(target_os = "windows") {
            format!("gradlew.bat {}", task)
        } else {
            format!("./gradlew {}", task)
        }
    } else if cfg!(target_os = "windows") {
        format!("mvnw.cmd {}", task)
    } else {
        format!("./mvnw {}", task)
    }
}

/// Use the project virtual environment when Project Creator created one.
/// Falling back to `python` keeps profiles for externally-created projects
/// usable, while generated Django projects never leak into system Python.
/// The relative interpreter path is resolved at spawn time by the
/// orchestrator, with an ancestor/system-python fallback when the venv is
/// missing.
fn python_command(_dir: Option<&str>, args: &str) -> String {
    python_venv_run(args)
}

/// Resolve the ASGI module path for a Python web framework backend.
///
/// Project Creator scaffolds FastAPI into `backend/src/main.py` and Flask
/// into `backend/src/app.py`, while hand-made projects keep the module at
/// the backend root (`main.py` / `app.py`). A framework launched from the
/// backend directory must reference the module that actually exists:
/// `uvicorn main:app` dies with "Could not import module main" when the
/// file lives in the `src` subpackage — the import path must be
/// `src.main:app` instead. The module is detected on disk so generated and
/// existing projects both start; unknown layouts fall back to the classic
/// root module.
///
/// `dir` is the framework's working directory as stored in the profile
/// (often a RELATIVE path like `./backend` — it resolves against the
/// project root, exactly like the run-time working-directory resolution).
fn python_framework_module(root: Option<&Path>, dir: Option<&str>) -> String {
    const CANDIDATES: [&str; 4] = ["src/main.py", "main.py", "src/app.py", "app.py"];
    let Some(root) = root else {
        return "main".to_string();
    };
    // `dir` is the framework's working directory as stored in the profile.
    // When it is None the backend IS the project root.
    let base = match dir {
        Some(rel) => {
            let p = Path::new(rel);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(rel)
            }
        }
        None => root.to_path_buf(),
    };
    for rel_file in CANDIDATES {
        if base.join(rel_file).is_file() {
            return rel_file.trim_end_matches(".py").replace('/', ".");
        }
    }
    "main".to_string()
}

fn python_install_command() -> String {
    python_ensure_venv_install("-m pip install -r requirements.txt")
}

/// Candidate ports a dev server may end up on when its configured port is
/// already taken: Vite, Next.js, Expo and the Angular CLI auto-increment by
/// 1 until they find a free port. The readiness wait accepts any of them, so
/// a busy primary port no longer times out the wait.
fn port_candidates(port: u16, spread: u16) -> Vec<u16> {
    let mut out = Vec::new();
    for p in (port as u32 + 1)..=(port as u32 + spread as u32) {
        if p <= u16::MAX as u32 {
            out.push(p as u16);
        }
    }
    out
}

/// Allocate the local dev-server port for a backend framework: the canonical
/// framework port (shared [`crate::ports`] table вЂ” the same one the wizard's
/// scaffolds and docker-compose use), bumped past ports already claimed by
/// the compose bootstrap or by other backends. Keeps every backend on a
/// distinct port and off the containers' host ports.
fn allocate_backend_port(
    fw: &str,
    compose_published_ports: &[u16],
    allocated: &mut Vec<u16>,
) -> u16 {
    let base = crate::ports::framework_default_port(fw).unwrap_or(3000);
    let mut reserved: Vec<u16> = compose_published_ports.to_vec();
    reserved.extend(allocated.iter().copied());
    let port = crate::ports::local_dev_port(base, &reserved);
    allocated.push(port);
    port
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
///
/// The IDE resolver is tried first; the generic application launcher
/// (well-known install dirs, Windows App Paths) covers the misses.
fn resolve_app(name: &str) -> Option<String> {
    if let Some(path) = crate::platform::ide::resolve_ide_executable(name) {
        return Some(path);
    }
    let launcher = crate::platform::app_launcher::resolve_application(name, None, None);
    if launcher.found && !launcher.is_flatpak {
        Some(launcher.program)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Builder options
// ---------------------------------------------------------------------------

/// User-configurable application choices used while building the graph.
/// `None`/empty values mean "auto-detect at build time".
#[derive(Debug, Clone, Default)]
pub struct ProfileBuildOptions {
    /// User-configured VS Code path/CLI (settings.vscode_path); falls back
    /// to "code" when empty.
    pub vscode: Option<String>,
    /// User-configured browser path/CLI (settings.browser_path); URL steps
    /// honor it at run time. Not used at build time.
#[allow(dead_code)]
    pub browser: Option<String>,
    /// User-configured database viewer path/CLI (settings.db_viewer_path);
    /// the first resolvable candidate wins when empty.
    pub db_viewer: Option<String>,
}

/// Ordered database viewer candidates: the user's configured choice first,
/// then platform defaults. `None` entries are skipped.
pub fn db_viewer_candidates(preferred: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = preferred {
        let p = p.trim();
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
    if cfg!(target_os = "windows") {
        out.extend(
            [
                "dbeaver",
                "datagrip",
                "heidisql",
                "tableplus",
                "sqlitebrowser",
                "pgadmin4",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    } else if cfg!(target_os = "macos") {
        out.extend(
            [
                "dbeaver",
                "datagrip",
                "tableplus",
                "sqlitebrowser",
                "pgadmin4",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    } else {
        out.extend(
            [
                "dbeaver-ce",
                "dbeaver",
                "datagrip",
                "sqlitebrowser",
                "pgadmin4",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    out
}

/// Display name of a database viewer CLI name (for step labels).
pub fn db_viewer_display_name(cli: &str) -> String {
    crate::platform::app_launcher::db_viewer_candidates()
        .iter()
        .find(|(id, _)| id == &cli)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| cli.to_string())
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a coherent V2 step graph from a wizard context.
///
/// The graph is structured like a real development session:
///   1. IDE/tool windows open first (independent roots).
///   2. Docker infrastructure: Docker Desktop в†’ wait for daemon в†’
///      compose up в†’ wait for database readiness.
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
    build_profile_v2_from_context_with_options(ctx, diagnostics, &ProfileBuildOptions::default())
}

/// Build a coherent V2 step graph from a wizard context, honoring the
/// user's application choices (VS Code path, database viewer).
///
/// The graph is structured like a real development session:
///   1. IDE/tool windows open first (independent roots).
///   2. Docker infrastructure: Docker Desktop в†’ wait for daemon в†’
///      compose up в†’ wait for database readiness.
///   3. Backend service starts (visible terminal), gated by the
///      infrastructure they need.
///   4. Readiness waits on backend ports, then API docs open.
///   5. Frontend dev servers (visible terminal) run in parallel.
///   6. A plain terminal is opened for ad-hoc commands.
///
/// Nothing is forced: IDE and tool steps are only emitted when the
/// application is resolvable on the host; install/migrate actions are
/// DISABLED by default (state-changing); the tools remain configurable.
pub fn build_profile_v2_from_context_with_options(
    ctx: &WizardContext,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
    opts: &ProfileBuildOptions,
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
    let effective_vscode = opts
        .vscode
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("code");
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
    // Host ports the compose step actually publishes. Local dev servers
    // (backend/frontend) allocate around them, so a generated profile never
    // makes two things listen on the same port (e.g. a Next.js frontend on
    // 3001 next to a Grafana container on 3001).
    let mut compose_published_ports: Vec<u16> = Vec::new();
    if docker {
        // Docker Desktop resolution is centralized in the platform Docker
        // service: Windows keeps its App Paths/Start Menu resolution, macOS
        // uses `open -a Docker`, Linux finds the GUI binary / CLI plugin /
        // systemd unit. Using the bare `docker` CLI here (the old Linux
        // branch) opened nothing — it only prints help — while the step was
        // reported as successful.
        if let Some((program, args)) =
            crate::platform::docker_service::DockerService::desktop_launcher()
        {
            open_app_step(&mut g, "Open Docker Desktop", &program, args, Vec::new());
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Low,
                "Docker Desktop not resolvable; daemon wait still applies".to_string(),
                None,
            ));
        }
        docker_wait = Some(wait_docker_step(&mut g));

        // The compose bootstrap must be anchored to a REAL configuration
        // file. The wizard writes docker-compose.yaml at the project root,
        // but generation can be skipped and users can move or delete the
        // file; `docker compose up` in a directory without one dies with
        // the cryptic "no configuration file provided: not found". Pin the
        // file with `-f` and omit the step (with a diagnostic) when it is
        // missing вЂ” the run then continues with the local install/start
        // steps instead of aborting on a file that does not exist.
        let compose_file = ctx
            .project_path
            .as_ref()
            .and_then(|root| DockerService::find_compose_file(root));
        if let Some(file) = compose_file {
            let compose_dir = file.parent().map(|p| p.to_string_lossy().into_owned());
            // The wizard's compose file includes the app service (`app:`,
            // build from the backend dir). The dev session runs the app
            // locally (visible terminal with hot reload); `docker compose up`
            // here bootstraps only the INFRASTRUCTURE services, so the app's
            // container never races the local dev server for the same port.
            // When no known infra service is found (foreign/edited compose
            // file) the whole file is started, preserving the old behavior.
            let parsed = parse_compose_services(&file);
            let infra_services: Vec<String> = parsed
                .services
                .iter()
                .filter(|s| crate::ports::tool_service_name(&s.name).is_some())
                .map(|s| s.name.clone())
                .collect();
            let full_up = infra_services.is_empty();
            let command = if full_up {
                format!("docker compose -f \"{}\" up -d", file.display())
            } else {
                format!(
                    "docker compose -f \"{}\" up -d {}",
                    file.display(),
                    infra_services.join(" ")
                )
            };
            // Only the services actually started by the command publish ports
            // during the run; those are the ports local dev servers must avoid.
            compose_published_ports = parsed
                .services
                .iter()
                .filter(|s| full_up || infra_services.contains(&s.name))
                .flat_map(|s| s.host_ports.iter().copied())
                .collect();
            compose_published_ports.sort_unstable();
            compose_published_ports.dedup();
            // The compose bootstrap runs in a VISIBLE terminal (the user
            // watches the build вЂ” hidden failures are the #1 support
            // question), but its completion is NOT "process started". A
            // `ProcessStarted` completion is a 6-second probe: a build that
            // is still running when the probe window expires reports
            // "success" even when it dies moments later (e.g. `npm ci`
            // failing on a missing package-lock.json). The DockerComposeUp
            // completion keeps polling вЂ” via a captured `docker compose ps`
            // (no extra terminal window) and the command's real exit code вЂ”
            // until containers are actually running. Readiness of specific
            // services is additionally verified by the port waits that
            // depend on this step.
            let mut metadata = HashMap::new();
            metadata.insert("kind".to_string(), "infrastructure".to_string());
            metadata.insert("visibility".to_string(), "visible_terminal".to_string());
            compose = Some(g.push(
                "Start Docker Compose",
                StepKind::RunCommand {
                    command,
                    command_spec: None,
                },
                vec![docker_wait
                            .clone()
                            .expect(
                                "docker_wait is Some: assigned at the top of the same `if docker` branch",
                            )],
                compose_dir,
                Some(Visibility::VisibleTerminal),
                Some(ExecutionMode::LongRunning),
                Some(CompletionPolicy::DockerComposeUp {
                    timeout_secs: COMPOSE_UP_TIMEOUT_SECS,
                }),
                None,
                true,
                Some(metadata),
            ));
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Warning,
                AnalysisConfidence::High,
                "Docker is enabled but the project has no docker compose configuration \
                 (compose.yaml, compose.yml, docker-compose.yaml, docker-compose.yml). \
                 The 'Start Docker Compose' step and container readiness waits were \
                 skipped; local install/start steps will run without containers."
                    .to_string(),
                None,
            ));
        }

        // Database readiness waits for compose-managed tools вЂ” only when the
        // compose bootstrap exists (a missing compose file means no services).
        let mut db_waits: Vec<String> = Vec::new();
        for tool in &ctx.tools {
            if !is_docker_tool(tool) {
                continue;
            }
            // Tools deployed locally are NOT in the compose file вЂ” no wait.
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
            // Without a compose bootstrap there are no containers to wait for.
            let Some(compose_id) = &compose else {
                continue;
            };
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
                tool_wait_candidate_ports(tool, port),
                tool_wait_timeout_secs(tool),
                compose_id.clone(),
                "low",
            );
            db_waits.push(id);
        }

        // The database viewer opens after the daemon is up. Only when the
        // project actually has database tools: a viewer step is added ONLY
        // if at least one supported database application is resolvable вЂ”
        // never a broken step for an unavailable app. The user-configured
        // viewer (settings.db_viewer_path) wins; otherwise the first
        // detected candidate is used.
        if !db_waits.is_empty() {
            let mut viewer: Option<(String, String)> = None;
            for cli in db_viewer_candidates(opts.db_viewer.as_deref()) {
                if let Some(resolved) = resolve_app(&cli) {
                    viewer = Some((resolved, db_viewer_display_name(&cli)));
                    break;
                }
            }
            match viewer {
                Some((resolved, display_name)) => {
                    open_app_step(
                        &mut g,
                        &format!("Open {}", display_name),
                        &resolved,
                        Vec::new(),
vec![docker_wait
                    .clone()
                    .expect("docker_wait is Some: assigned at the top of the same `if docker` branch")],
                    );
                }
                None => {
                    diagnostics.push(AnalysisDiagnostic::new(
                        DiagnosticSeverity::Info,
                        AnalysisConfidence::Low,
                        "No supported database viewer found; database client step omitted"
                            .to_string(),
                        None,
                    ));
                }
            }
        }
    }

    // --- 3. Backend services ---
    let mut backend_waits: Vec<String> = Vec::new();
    // Every backend's local dev port: canonical framework port, bumped past
    // compose-published ports and already-allocated backend ports.
    let mut allocated_backend_ports: Vec<u16> = Vec::new();
    for fw in &backend_frameworks(ctx) {
        let dir = side_dir(ctx, "backend", fw);
        let infra: Vec<String> = compose.clone().into_iter().collect();
        match fw.as_str() {
            "express" | "fastify" | "hono" | "nestjs" | "nest" => {
                let install = one_shot_step(
                    &mut g,
                    "Install backend dependencies",
                    &node_install_command(ctx, dir.as_deref()),
                    dir.as_deref(),
                    infra.clone(),
                    true,
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
                    {
                        let mut deps = infra.clone();
                        deps.push(install);
                        deps
                    },
                    "high",
                );
                let port = allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports);
                let wait = wait_port_step(
                    &mut g,
                    "Wait for backend port",
                    port,
                    &port_candidates(port, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
                    &python_install_command(),
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let cmd = if fw == "fastapi" {
                    let module = python_framework_module(ctx.project_path.as_deref(), dir.as_deref());
                    python_command(dir.as_deref(), &format!("-m uvicorn {}:app --reload", module))
                } else if fw == "flask" {
                    let module = python_framework_module(ctx.project_path.as_deref(), dir.as_deref());
                    python_command(dir.as_deref(), &format!("-m flask --app {} run --debug", module))
                } else {
                    python_command(dir.as_deref(), "-m litestar run --reload")
                };
                let start = service_step(
                    &mut g,
                    "Start Python backend",
                    &cmd,
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let port = allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports);
                let wait = wait_port_step(
                    &mut g,
                    "Wait for backend port",
                    port,
                    &port_candidates(port, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
                    &python_install_command(),
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let migrate = one_shot_step(
                    &mut g,
                    "Run Django database migrations (disabled by default)",
                    &python_command(dir.as_deref(), "manage.py migrate"),
                    dir.as_deref(),
                    vec![install.clone()],
                    false,
                );
                let start = service_step(
                    &mut g,
                    "Start Django server",
                    &python_command(dir.as_deref(), "manage.py runserver"),
                    dir.as_deref(),
                    // Wait for BOTH the install (which provisions the venv)
                    // and the migrate step. The migrate step is disabled, so
                    // it pre-completes instantly; depending on it ALONE would
                    // let the server start in parallel with the install and
                    // race the venv creation (the exact "Executable
                    // '.venv\Scripts\python.exe' not found" failure).
                    vec![migrate, install],
                    "high",
                );
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Django port",
                    allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports),
                    &[],
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
                    &go_run_command(ctx, dir.as_deref()),
                    dir.as_deref(),
                    infra,
                    "high",
                );
                let port = allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports);
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Go backend port",
                    port,
                    &port_candidates(port, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
            "symfony" | "laravel" => {
                // Composer installs are one-shot; the development server is
                // long-running and must be started from the PHP side's
                // directory. Symfony CLI is preferred when installed, while
                // the built-in PHP server keeps the profile usable on hosts
                // that only have PHP + Composer.
                let install = one_shot_step(
                    &mut g,
                    "Install PHP dependencies",
                    "composer install --no-interaction",
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let server = if fw == "symfony" && resolve_app("symfony").is_some() {
                    "symfony server:start --no-tls --allow-http --port=8000"
                } else if fw == "laravel" {
                    "php artisan serve --host=127.0.0.1 --port=8000"
                } else {
                    "php -S 127.0.0.1:8000 -t public"
                };
                let start = service_step(
                    &mut g,
                    if fw == "symfony" {
                        "Start Symfony backend"
                    } else {
                        "Start Laravel backend"
                    },
                    server,
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(
                    &mut g,
                    &format!(
                        "Wait for {} backend port",
                        if fw == "symfony" {
                            "Symfony"
                        } else {
                            "Laravel"
                        }
                    ),
                    allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports),
                    &[],
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "medium",
                );
                backend_waits.push(wait);
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
                    let port =
                        allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports);
                    let wait = wait_port_step(
                        &mut g,
                        "Wait for Rust backend port",
                        port,
                        &port_candidates(port, 2),
                        30,
                        start,
                        "low",
                    );
                    backend_waits.push(wait);
                }
            }
            "spring-boot" | "spring" => {
                let gradle = ctx.tools.iter().any(|t| t == "gradle")
                    && !ctx.tools.iter().any(|t| t == "maven");
                let install = one_shot_step(
                    &mut g,
                    "Install Java dependencies",
                    &java_wrapper_command(
                        ctx,
                        if gradle {
                            "build -x test"
                        } else {
                            "package -DskipTests"
                        },
                    ),
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let start = service_step(
                    &mut g,
                    "Start Spring Boot",
                    &java_wrapper_command(ctx, if gradle { "bootRun" } else { "spring-boot:run" }),
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let port = allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports);
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Spring Boot port",
                    port,
                    &port_candidates(port, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
                backend_waits.push(wait.clone());
                let docs = open_url_step(
                    &mut g,
                    "Open Spring Boot Swagger docs",
                    &format!("http://localhost:{}/swagger-ui/index.html", port),
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
                let wait = wait_port_step(
                    &mut g,
                    "Wait for .NET backend port",
                    allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports),
                    &port_candidates(5000, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
                backend_waits.push(wait);
            }
            "rails" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Ruby dependencies",
                    "bundle install",
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let start = service_step(
                    &mut g,
                    "Start Rails server",
                    "bin/rails server",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Rails port",
                    allocate_backend_port(fw, &compose_published_ports, &mut allocated_backend_ports),
                    &port_candidates(3000, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
                backend_waits.push(wait);
            }
            _ => {}
        }
    }

    // --- 4. Frontend services (parallel roots unless they need infra) ---
    // The wizard's Tauri scaffold pins the companion vite dev URL to 5173
    // (tauri.conf.json devUrl); shifting it would break `cargo tauri dev`,
    // so the frontend keeps its canonical port for Tauri stacks.
    let has_tauri = ctx.frameworks.iter().any(|f| f == "tauri");
    for fw in &frontend_frameworks(ctx) {
        let dir = side_dir(ctx, "frontend", fw);
        let infra: Vec<String> = compose.clone().into_iter().collect();
        match fw.as_str() {
            "nextjs" | "next" | "nuxt" | "nuxtjs" | "vite" | "vite-react" | "vite-vue"
            | "vite-svelte" | "react" | "vue" | "svelte" | "solid" => {
                // Canonical port first; nextjs/nuxt dev servers own 3000, the
                // vite family 5173. With a backend present the frontend moves
                // off the backend's port, then off every port the compose
                // bootstrap publishes (grafana 3001, airflow 8080, ...) so
                // the frontend never collides with a container.
                let base_port =
                    if matches!(fw.as_str(), "next" | "nextjs" | "nuxt" | "nuxtjs") {
                        3000
                    } else {
                        5173
                    };
                let frontend_port = if has_tauri {
                    base_port
                } else {
                    let mut reserved: Vec<u16> = compose_published_ports.clone();
                    reserved.extend(allocated_backend_ports.iter().copied());
                    let start = if has_backend { base_port + 1 } else { base_port };
                    crate::ports::local_dev_port(start, &reserved)
                };
                let frontend_cmd = if is_vite_family(fw) {
                    // Vite (and the wizard's react/vue/svelte/solid scaffolds
                    // which all run on Vite) binds 'localhost'. On modern
                    // Node (17+) 'localhost' resolves to ::1 FIRST, so the
                    // dev server listens on the IPv6 loopback ONLY — while
                    // Windows often refuses inbound ::1 connections
                    // (WSAEACCES, firewall/profile dependent), so the
                    // readiness wait never sees the port. Pinning the IPv4
                    // loopback makes the server reachable and the wait
                    // deterministic. Next/Nuxt bind 0.0.0.0 (all interfaces)
                    // and use different CLI flags — they stay unpinned.
                    if has_backend && !has_tauri {
                        format!(
                            "npm run dev -- --port {} --host 127.0.0.1",
                            frontend_port
                        )
                    } else {
                        "npm run dev -- --host 127.0.0.1".to_string()
                    }
                } else if has_backend && !has_tauri {
                    format!("npm run dev -- --port {}", frontend_port)
                } else {
                    "npm run dev".to_string()
                };
                let install = one_shot_step(
                    &mut g,
                    "Install frontend dependencies",
                    &node_install_command(ctx, dir.as_deref()),
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let start = service_step(
                    &mut g,
                    "Start frontend dev server",
                    &frontend_cmd,
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(
                    &mut g,
                    "Wait for frontend port",
                    frontend_port,
                    &port_candidates(frontend_port, 4),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
                let _ = wait;
            }
            "angular" => {
                let install = one_shot_step(
                    &mut g,
                    "Install frontend dependencies",
                    &node_install_command(ctx, dir.as_deref()),
                    dir.as_deref(),
                    infra.clone(),
                    true,
                );
                let start = service_step(
                    &mut g,
                    "Start Angular dev server",
                    "npm run start",
                    dir.as_deref(),
                    vec![install],
                    "high",
                );
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Angular port",
                    4200,
                    &port_candidates(4200, 4),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
                let wait = wait_port_step(
                    &mut g,
                    "Wait for Metro bundler",
                    8081,
                    &port_candidates(8081, 4),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
                let _ = wait;
            }
            "electron" => {
                let install = one_shot_step(
                    &mut g,
                    "Install Electron dependencies",
                    &node_install_command(ctx, dir.as_deref()),
                    dir.as_deref(),
                    infra.clone(),
                    true,
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
        let infra: Vec<String> = compose.clone().into_iter().collect();
        build_language_steps(ctx, &mut g, &infra);
    }

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

fn build_language_steps(ctx: &WizardContext, g: &mut GraphBuilder, infra: &[String]) {
    for lang in &ctx.languages {
        match lang.as_str() {
            "python" => {
                let install = one_shot_step(
                    g,
                    "Install Python dependencies",
                    &python_install_command(),
                    None,
                    infra.to_vec(),
                    true,
                );
                let start = service_step(
                    g,
                    "Run Python project",
                    &python_command(None, "main.py"),
                    None,
                    vec![install],
                    "medium",
                );
                let _ = start;
            }
            "go" => {
                let start = service_step(
                    g,
                    "Run Go project",
                    &go_run_command(ctx, None),
                    None,
                    Vec::new(),
                    "medium",
                );
                let wait = wait_port_step(
                    g,
                    "Wait for Go port",
                    8080,
                    &port_candidates(8080, 2),
                    WAIT_PORT_TIMEOUT_SECS,
                    start,
                    "low",
                );
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
                    &node_install_command(ctx, None),
                    None,
                    infra.to_vec(),
                    true,
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
                    &java_wrapper_command(ctx, "run"),
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
        parts.join(" В· ")
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
            readme_locale: None,
        }
    }

    fn build(c: &WizardContext) -> (LaunchProfileV2, Vec<AnalysisDiagnostic>) {
        let mut diagnostics = Vec::new();
        let profile = build_profile_v2_from_context(c, &mut diagnostics);
        (profile, diagnostics)
    }

    #[test]
    fn expo_plus_go_plus_docker_coherent_graph() {
        // Use a real temp directory as the project root: the validator
        // rejects non-existent roots (fail-fast on deleted projects).
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // The wizard writes docker-compose.yaml at the project root; the
        // compose bootstrap step is only emitted when the file exists.
        std::fs::write(
            dir.join("docker-compose.yaml"),
            "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5432:5432\"\n",
        )
        .unwrap();

        let mut c = ctx(
            &["typescript", "go"],
            &["expo", "gin"],
            &["postgresql"],
            true,
        );
        c.project_path = Some(dir.clone());
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
        // The auto-generated plain-terminal tool step was removed: profiles
        // are launch recipes, not suggestion lists.
        assert!(!has("Open a terminal"), "{:?}", labels);

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

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_bootstrap_runs_in_visible_terminal_with_start_probe() {
        // A captured one-shot `docker compose up -d` is fragile: the CLI
        // stays attached to the build, and when its stdout is a pipe the
        // daemon frequently cancels the build, so the step dies with
        // "Process exited with error code 1" while Docker itself was fine.
        // The bootstrap must run in a visible terminal and complete when the
        // command has started (readiness is verified by the port waits that
        // depend on it), exactly like the analyzer's compose step.
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_compose_vis_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("docker-compose.yaml"),
            "services:\n  db:\n    image: postgres:16\n    ports:\n      - \"5432:5432\"\n",
        )
        .unwrap();

        let mut c = ctx(&["typescript"], &["nestjs"], &["postgresql"], true);
        c.project_path = Some(dir.clone());
        let (profile, _diags) = build(&c);
        let compose = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Docker Compose"))
            .unwrap();
        assert_eq!(compose.visibility, Some(Visibility::VisibleTerminal));
        assert_eq!(compose.execution_mode, Some(ExecutionMode::LongRunning));
        assert!(
            matches!(
                compose.completion,
                Some(CompletionPolicy::DockerComposeUp { .. })
            ),
            "compose bootstrap must verify running containers, got {:?}",
            compose.completion
        );
        // The command pins the compose file so it never depends on the cwd.
        match &compose.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(command.contains("up -d"), "{}", command);
                assert!(
                    command.contains("-f"),
                    "compose bootstrap must pin its config file: {}",
                    command
                );
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
        // Readiness is still verified by the dependent port waits.
        assert!(profile
            .steps
            .iter()
            .any(|s| s.depends_on.contains(&compose.id)
                && matches!(s.kind, StepKind::WaitForPort { .. })));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kafka_readiness_wait_gets_generous_timeout_and_candidate_port() {
        // Kafka is the slowest compose tool to become reachable (image
        // pull + broker bootstrap routinely exceeds the generic 60s), so
        // its readiness wait must carry a longer timeout вЂ” otherwise the
        // wait dies with "Timeout: none of ports [9092] on 127.0.0.1 open
        // after 60s" while the broker is still starting.
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_kafka_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("docker-compose.yaml"),
            "services:\n  kafka:\n    image: confluentinc/cp-kafka:latest\n    ports:\n      - \"9092:9092\"\n",
        )
        .unwrap();

        let mut c = ctx(&["typescript"], &["nestjs"], &["kafka"], true);
        c.project_path = Some(dir.clone());
        let (profile, _diags) = build(&c);
        let kafka = profile
            .steps
            .iter()
            .find(|s| s.label.contains("kafka readiness"))
            .expect("kafka readiness wait must be generated");
        match &kafka.completion {
            Some(CompletionPolicy::PortOpen { timeout_secs, .. }) => {
                assert!(
                    *timeout_secs >= 180,
                    "kafka wait must carry a generous timeout, got {}s",
                    timeout_secs
                );
            }
            other => panic!("expected PortOpen completion, got {:?}", other),
        }
        match &kafka.kind {
            StepKind::WaitForPort {
                port,
                candidate_ports,
                ..
            } => {
                assert_eq!(*port, 9092);
                assert!(
                    candidate_ports.contains(&29092),
                    "kafka wait should accept 29092 (KRaft templates): {:?}",
                    candidate_ports
                );
            }
            other => panic!("expected WaitForPort, got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn docker_without_compose_file_skips_compose_step() {
        // A docker-enabled project whose compose file was never generated
        // (or was deleted) must NOT get a "Start Docker Compose" step:
        // `docker compose up` there dies with "no configuration file
        // provided: not found" and aborts the whole run.
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_nocompose_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut c = ctx(
            &["typescript", "go"],
            &["expo", "gin"],
            &["postgresql"],
            true,
        );
        c.project_path = Some(dir.clone());
        let (profile, diags) = build(&c);
        let labels: Vec<&str> = profile.steps.iter().map(|s| s.label.as_str()).collect();
        let has = |needle: &str| labels.iter().any(|l| l.contains(needle));

        // No compose bootstrap, no container readiness waits.
        assert!(!has("Start Docker Compose"), "{:?}", labels);
        assert!(!has("postgresql readiness"), "{:?}", labels);
        // The rest of the graph is intact.
        assert!(has("Go backend"), "{:?}", labels);
        assert!(has("Start Expo"), "{:?}", labels);
        // A clear warning explains why containers were skipped.
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("no docker compose configuration")),
            "{:?}",
            diags
        );

        let _ = std::fs::remove_dir_all(&dir);
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

    /// The Django chain must serialize the server behind the install step
    /// that provisions the venv: depending on the DISABLED migrate step alone
    /// let the server race the venv creation. The server and migrate steps
    /// must run with the venv interpreter.
    #[test]
    fn django_server_waits_for_install_and_runs_in_venv() {
        let c = ctx(&["python"], &["django"], &[], false);
        let (profile, _) = build(&c);

        let server = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Django server"))
            .expect("server step");
        let install = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Install Python dependencies"))
            .expect("install step");
        let migrate = profile
            .steps
            .iter()
            .find(|s| s.label.contains("migrations"))
            .expect("migrate step");
        assert!(!migrate.enabled);

        // Server waits for BOTH install and migrate, so it starts only after
        // the install finishes even when migrate is a disabled gate.
        assert!(
            server.depends_on.iter().any(|d| d == &install.id),
            "server depends on install: {:?}",
            server.depends_on
        );
        assert!(
            server.depends_on.iter().any(|d| d == &migrate.id),
            "server depends on migrate: {:?}",
            server.depends_on
        );

        match &server.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(command.ends_with("manage.py runserver"), "{command}");
                if cfg!(target_os = "windows") {
                    assert!(
                        command.starts_with(".venv\\Scripts\\python.exe "),
                        "{command}"
                    );
                } else {
                    assert!(command.starts_with("./.venv/bin/python "), "{command}");
                }
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
        match &install.kind {
            StepKind::RunCommand { command, .. } => {
                if cfg!(target_os = "windows") {
                    assert!(
                        command.contains("if not exist .venv\\Scripts\\python.exe"),
                        "{command}"
                    );
                } else {
                    assert!(command.contains("[ -x .venv/bin/python ]"), "{command}");
                }
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
    }

    /// FastAPI/Flask/Litestar backends must run the server module through the
    /// venv interpreter: `uvicorn`/`flask` installed into `.venv` are NOT on
    /// PATH, so the bare CLI names could never start after the venv install.
    #[test]
    fn python_backends_run_modules_through_venv_interpreter() {
        for fw in ["fastapi", "flask", "litestar"] {
            let c = ctx(&["python"], &[fw], &[], false);
            let (profile, _) = build(&c);
            let start = profile
                .steps
                .iter()
                .find(|s| s.label.contains("Start Python backend"))
                .expect("start step");
            match &start.kind {
                StepKind::RunCommand { command, .. } => {
                    if cfg!(target_os = "windows") {
                        assert!(
                            command.starts_with(".venv\\Scripts\\python.exe "),
                            "{command}"
                        );
                    } else {
                        assert!(command.starts_with("./.venv/bin/python "), "{command}");
                    }
                    assert!(command.contains("-m "), "{command}");
                }
                other => panic!("expected RunCommand, got {:?}", other),
            }
        }
    }

    /// FastAPI/Flask backends scaffolded by Project Creator live under
    /// `backend/src/` (src/main.py, src/app.py). The server command must
    /// reference the actual module: `uvicorn main:app` dies with "Could not
    /// import module main" when the file is inside the `src` subpackage.
    #[test]
    fn python_backends_in_src_layout_use_module_path() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_src_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("backend/src")).unwrap();
        std::fs::write(
            dir.join("backend/src/main.py"),
            "from fastapi import FastAPI\napp = FastAPI()\n",
        )
        .unwrap();
        std::fs::write(dir.join("backend/requirements.txt"), "fastapi\nuvicorn\n").unwrap();

        let mut c = ctx(&["python"], &["fastapi"], &[], false);
        c.project_path = Some(dir);
        let (profile, _) = build(&c);
        let start = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Python backend"))
            .expect("start step");
        match &start.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(
                    command.contains("-m uvicorn src.main:app --reload"),
                    "command must target the src subpackage: {command}"
                );
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
    }

    /// Vite dev servers must be pinned to the IPv4 loopback: on modern Node
    /// they bind ::1 only, and Windows often refuses inbound ::1
    /// connections — the port wait then never sees the server.
    #[test]
    fn vite_frontend_is_pinned_to_ipv4_loopback() {
        let c = ctx(&["typescript"], &["vite"], &[], false);
        let (profile, _) = build(&c);
        let start = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start frontend dev server"))
            .expect("start step");
        match &start.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(command.contains("--host 127.0.0.1"), "{command}");
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
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

    fn compose_dir(content: &str, tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_ports_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("docker-compose.yaml"), content).unwrap();
        dir
    }

    fn wait_port_of(profile: &LaunchProfileV2, label_part: &str) -> (u16, String) {
        let step = profile
            .steps
            .iter()
            .find(|s| s.label.contains(label_part))
            .unwrap_or_else(|| panic!("no step containing '{}'", label_part));
        match &step.kind {
            StepKind::WaitForPort { port, .. } => (*port, step.label.clone()),
            other => panic!("expected WaitForPort, got {:?}", other),
        }
    }

    #[test]
    fn compose_up_starts_only_infra_services_not_the_app() {
        // The wizard's compose file carries the app service (`app:` building
        // the backend). The dev session runs the app locally, so the compose
        // bootstrap must start only the infrastructure services вЂ” otherwise
        // the app container and the local dev server race for the same port.
        let dir = compose_dir(
            "services:\n\
             \x20 app:\n\x20\x20 build: .\n\x20\x20 ports:\n\x20\x20\x20 - \"3000:3000\"\n\
             \x20 postgres:\n\x20\x20 image: postgres:16\n\x20\x20 ports:\n\x20\x20\x20 - \"5432:5432\"\n",
            "infraonly",
        );
        let mut c = ctx(&["typescript"], &["nestjs"], &["postgresql"], true);
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);
        let compose = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Docker Compose"))
            .expect("compose step");
        match &compose.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(
                    command.contains("up -d postgres"),
                    "compose must start only infra services: {command}"
                );
                assert!(
                    !command.contains(" up -d app"),
                    "the app service must not be started by the dev session: {command}"
                );
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
        // The local backend keeps its canonical port 3000 (the app service is
        // not running, so nothing claims it).
        assert_eq!(wait_port_of(&profile, "Wait for backend port").0, 3000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frontend_avoids_grafana_and_backend_ports() {
        // Next.js + backend + Grafana: the frontend's default 3001 (shifted
        // off the backend's 3000) collides with Grafana's host port 3001 вЂ”
        // it must move to 3002 and pass `--port 3002` to the dev server.
        let dir = compose_dir(
            "services:\n\
             \x20 postgres:\n\x20\x20 image: postgres:16\n\x20\x20 ports:\n\x20\x20\x20 - \"5432:5432\"\n\
             \x20 grafana:\n\x20\x20 image: grafana/grafana\n\x20\x20 ports:\n\x20\x20\x20 - \"3001:3000\"\n",
            "grafana",
        );
        let mut c = ctx(
            &["typescript"],
            &["nestjs", "nextjs"],
            &["postgresql", "grafana"],
            true,
        );
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);
        assert_eq!(wait_port_of(&profile, "Wait for backend port").0, 3000);
        assert_eq!(wait_port_of(&profile, "Wait for frontend port").0, 3002);
        let frontend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start frontend dev server"))
            .expect("frontend step");
        match &frontend.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(command.contains("--port 3002"), "{command}");
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backend_avoids_compose_published_ports() {
        // A backend whose canonical port is claimed by a compose service
        // (Django 8000 taken by a custom compose service) moves off it; the
        // docs URL follows the allocated port.
        let dir = compose_dir(
            "services:\n\
             \x20 app:\n\x20\x20 build: .\n\x20\x20 ports:\n\x20\x20\x20 - \"8000:8000\"\n",
            "backend",
        );
        let mut c = ctx(&["python"], &["django"], &[], true);
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);
        assert_eq!(wait_port_of(&profile, "Wait for Django port").0, 8001);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tauri_vite_companion_keeps_5173() {
        // The Tauri scaffold pins vite's dev URL to 5173 (tauri.conf.json
        // devUrl); shifting the companion vite to 5174 would break
        // `cargo tauri dev`.
        let c = ctx(&["typescript"], &["tauri", "react"], &[], false);
        let (profile, _) = build(&c);
        assert_eq!(wait_port_of(&profile, "Wait for frontend port").0, 5173);
        let frontend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start frontend dev server"))
            .expect("frontend step");
        match &frontend.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(!command.contains("--port"), "{command}");
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
    }

    #[test]
    fn fastapi_wait_and_docs_follow_scaffold_port_8000() {
        let c = ctx(&["python"], &["fastapi"], &[], false);
        let (profile, _) = build(&c);
        let (port, _) = wait_port_of(&profile, "Wait for backend port");
        assert_eq!(port, 8000);
        let docs = profile
            .steps
            .iter()
            .find(|s| s.label.contains("FastAPI docs"))
            .expect("docs step");
        match &docs.kind {
            StepKind::OpenUrl { url } => assert_eq!(url, "http://localhost:8000/docs"),
            other => panic!("expected OpenUrl, got {:?}", other),
        }
    }

    /// A Rust backend whose manifest lives in `backend/` must be launched
    /// FROM that directory вЂ” never from the project root, where cargo fails
    /// with "could not find Cargo.toml ... or any parent directory".
    #[test]
    fn rust_backend_in_backend_dir_gets_manifest_owned_working_dir() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_split_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("backend")).unwrap();
        std::fs::create_dir_all(dir.join("frontend")).unwrap();
        std::fs::write(
            dir.join("backend/Cargo.toml"),
            "[package]\nname = \"api\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("frontend/package.json"),
            r#"{"name":"web","dependencies":{"vite":"^8"},"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();

        let mut c = ctx(&["rust", "typescript"], &["axum", "vue"], &[], false);
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);

        let rust = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Rust backend"))
            .expect("rust step");
        assert_eq!(
            rust.working_directory.as_deref(),
            Some("./backend"),
            "cargo run must run in the directory that owns Cargo.toml: {:?}",
            rust.working_directory
        );

        let frontend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start frontend dev server"))
            .expect("frontend step");
        assert_eq!(frontend.working_directory.as_deref(), Some("./frontend"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Split projects whose sides use NON-canonical folder names (`server/`,
    /// `client/`) are located by the manifest search, not by folder names.
    #[test]
    fn split_rust_search_finds_non_canonical_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_search_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("server")).unwrap();
        std::fs::create_dir_all(dir.join("client")).unwrap();
        std::fs::write(
            dir.join("server/Cargo.toml"),
            "[package]\nname = \"api\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("client/package.json"),
            r#"{"name":"web","dependencies":{"vue":"^3"},"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();

        let mut c = ctx(&["rust", "typescript"], &["axum", "vue"], &[], false);
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);

        let rust = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Rust backend"))
            .expect("rust step");
        assert_eq!(
            rust.working_directory.as_deref(),
            Some("./server"),
            "manifest search must find Cargo.toml in server/: {:?}",
            rust.working_directory
        );

        let frontend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start frontend dev server"))
            .expect("frontend step");
        assert_eq!(frontend.working_directory.as_deref(), Some("./client"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A backend whose manifest is at the project root stays at the root:
    /// the search must not invent a subdirectory for it.
    #[test]
    fn rust_backend_at_root_stays_at_root() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_pb_root_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"api\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("frontend")).unwrap();
        std::fs::write(
            dir.join("frontend/package.json"),
            r#"{"name":"web","dependencies":{"vue":"^3"},"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();

        let mut c = ctx(&["rust", "typescript"], &["axum", "vue"], &[], false);
        c.project_path = Some(dir.clone());
        let (profile, _) = build(&c);

        let rust = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Rust backend"))
            .expect("rust step");
        assert_eq!(
            rust.working_directory, None,
            "root-owned Cargo.toml keeps the step in the project root"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
