use crate::modules::devlauncher::models::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Confidence levels for inferred facts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisConfidence {
    /// Direct evidence (dependency present, lockfile, explicit config value).
    High,
    /// Structural evidence (entry file plus supporting files).
    Medium,
    /// Guess (default ports, implied services).
    Low,
}

impl AnalysisConfidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            AnalysisConfidence::High => "high",
            AnalysisConfidence::Medium => "medium",
            AnalysisConfidence::Low => "low",
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis diagnostics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisDiagnostic {
    pub severity: DiagnosticSeverity,
    pub confidence: AnalysisConfidence,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl AnalysisDiagnostic {
    pub(crate) fn new(
        severity: DiagnosticSeverity,
        confidence: AnalysisConfidence,
        message: impl Into<String>,
        file: Option<String>,
    ) -> Self {
        Self {
            severity,
            confidence,
            message: message.into(),
            file,
        }
    }
}

// ---------------------------------------------------------------------------
// Analyze options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    /// Maximum directory depth walked below the project root.
    pub max_depth: usize,
    /// User-configured VS Code CLI path (from settings).
    pub vscode_path: Option<String>,
    /// User-configured database viewer CLI path (from settings).
    pub db_viewer_path: Option<String>,
    /// Emit "open IDE" steps (only when the IDE is resolvable).
    pub include_ide_steps: bool,
    /// Emit helper/tool steps (Docker Desktop, database viewer, empty terminal).
    pub include_tool_steps: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            max_depth: 8,
            vscode_path: None,
            db_viewer_path: None,
            include_ide_steps: true,
            include_tool_steps: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Draft profile -- the analyzer output
// ---------------------------------------------------------------------------

/// A generated draft profile. The analyzer produces a *draft*, not a final
/// execution plan: every inference carries a confidence level and a
/// diagnostic, and the user is expected to review the steps before running.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftProfile {
    pub profile: LaunchProfileV2,
    pub diagnostics: Vec<AnalysisDiagnostic>,
}

// ---------------------------------------------------------------------------
// Project model -- detection pass first, step generation second
// ---------------------------------------------------------------------------

/// Directories that are never scanned: dependency trees, VCS metadata,
/// build outputs, virtual environments, caches and generated artifacts.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "target",
    "build",
    "dist",
    "out",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".expo",
    ".turbo",
    ".cache",
    ".idea",
    ".vscode",
    ".gradle",
    ".cargo",
    ".terraform",
    ".serverless",
    ".docusaurus",
    ".angular",
    ".parcel-cache",
    "coverage",
    "Pods",
    "DerivedData",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
    Unknown,
}

impl PackageManager {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
            PackageManager::Unknown => "unknown",
        }
    }
}

impl PackageManager {
    #[allow(dead_code)]
    pub fn is_known(&self) -> bool {
        !matches!(self, PackageManager::Unknown)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeFeatures {
    pub expo: bool,
    pub react_native: bool,
    pub vite: bool,
    pub next: bool,
    pub nuxt: bool,
    pub svelte: bool,
    pub vue: bool,
    pub angular: bool,
    pub electron: bool,
    pub api_framework: bool,
    pub swagger: bool,
    pub is_frontend_framework: bool,
    pub is_backend_framework: bool,
    /// Runnable script present but no recognizable framework.
    pub generic_node: bool,
}

#[derive(Debug, Clone)]
pub struct NodePackage {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub name: String,
    pub package_manager: PackageManager,
    pub is_workspace_root: bool,
    pub run_command: Option<String>,
    pub port: Option<u16>,
    pub port_confidence: AnalysisConfidence,
    pub port_source: Option<String>,
    pub features: NodeFeatures,
    pub has_lockfile: bool,
}

#[derive(Debug, Clone)]
pub struct CargoPackage {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub has_tauri: bool,
    pub is_workspace: bool,
}

#[derive(Debug, Clone)]
pub struct GoModule {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub module: String,
}

#[derive(Debug, Clone)]
pub struct PythonProject {
    pub dir: PathBuf,
    pub kind: PythonKind,
    pub has_fastapi: bool,
    pub has_flask: bool,
    pub has_django: bool,
    pub entry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonKind {
    Requirements,
    Pyproject,
    Pipfile,
    None_,
}

#[derive(Debug, Clone)]
pub struct GradleProject {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub is_spring: bool,
}

#[derive(Debug, Clone)]
pub struct MavenProject {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub is_spring: bool,
}

#[derive(Debug, Clone)]
pub struct DotnetProject {
    pub dir: PathBuf,
    pub manifest: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RubyProject {
    pub dir: PathBuf,
    pub manifest: PathBuf,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ComposerProject {
    pub dir: PathBuf,
    pub manifest: PathBuf,
    pub is_symfony: bool,
    pub is_laravel: bool,
}

/// A single service parsed from a docker-compose file. The compose file is
/// the source of truth for what runs in containers: ports are explicit
/// `ports:` mappings and build contexts tell us which local directories are
/// *governed* by the container (a local run there would conflict).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ComposeService {
    pub name: String,
    /// Build context directory, resolved relative to the compose file dir.
    pub build_context: Option<PathBuf>,
    pub image: Option<String>,
    /// Explicitly published host ports (`ports: - "8080:80"`).
    pub host_ports: Vec<u16>,
    /// Container ports (published or exposed).
    pub container_ports: Vec<u16>,
    pub depends_on: Vec<String>,
    /// Classified as a database service (name/image/port match).
    pub is_db: bool,
    /// Matched web-tool name when the service looks like a browser tool.
    pub web_tool: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComposeFile {
    pub path: PathBuf,
    pub dir: PathBuf,
    /// Host ports of services that look like databases, with the service
    /// name that hinted at them.
    pub db_ports: Vec<(u16, String)>,
    /// Host ports of services that look like web tools (grafana, airflow,
    /// ...), with the tool name that hinted at them.
    pub web_tools: Vec<(u16, String)>,
    /// Fully parsed services (ports, images, build contexts, depends_on).
    pub services: Vec<ComposeService>,
    pub parse_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DockerfileEntry {
    pub path: PathBuf,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutClass {
    #[default]
    RootOnly,
    Split,
    Monorepo,
    Nested,
}

/// The full picture of a project built by the detection pass. Step
/// generation reads this model; it never re-walks the filesystem.
#[derive(Debug, Clone, Default)]
pub struct ProjectModel {
    pub root: PathBuf,
    pub node_packages: Vec<NodePackage>,
    pub cargo_packages: Vec<CargoPackage>,
    pub go_modules: Vec<GoModule>,
    pub python_projects: Vec<PythonProject>,
    pub gradle_projects: Vec<GradleProject>,
    pub maven_projects: Vec<MavenProject>,
    pub dotnet_projects: Vec<DotnetProject>,
    pub ruby_projects: Vec<RubyProject>,
    pub composer_projects: Vec<ComposerProject>,
    pub compose_files: Vec<ComposeFile>,
    pub dockerfiles: Vec<DockerfileEntry>,
    pub makefiles: Vec<PathBuf>,
    pub solution_files: Vec<PathBuf>,
    pub has_git: bool,
    pub layout: LayoutClass,
}

impl ProjectModel {
    /// Build the project model by scanning the filesystem once.
    pub fn collect(root: &str, max_depth: usize) -> Result<Self, String> {
        let root = PathBuf::from(root);
        if !root.is_dir() {
            return Err(format!(
                "Project path '{}' is not a directory",
                root.display()
            ));
        }

        let mut model = ProjectModel {
            root,
            ..Default::default()
        };
        model.detect_root_markers();

        let walker = WalkDir::new(&model.root)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter();
        let mut seen_dirs: HashSet<PathBuf> = HashSet::new();

        for entry in walker.filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy().to_string();
                return !SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip));
            }
            true
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    eprintln!("[Analyzer] Access error: {}", err);
                    continue;
                }
            };

            if entry.file_type().is_dir() {
                continue;
            }

            let path = entry.path().to_path_buf();
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let dir = match path.parent() {
                Some(d) => d.to_path_buf(),
                None => continue,
            };

            match filename {
                "package.json" => {
                    if seen_dirs.insert(dir.clone()) {
                        match parse_node_package(&dir, &path) {
                            Ok(pkg) => model.node_packages.push(pkg),
                            Err(msg) => {
                                eprintln!("[Analyzer] Skipping {}: {}", path.display(), msg);
                            }
                        }
                    }
                }
                "Cargo.toml" => {
                    if seen_dirs.insert(dir.clone()) {
                        model.cargo_packages.push(parse_cargo_package(&dir, &path));
                    }
                }
                "go.mod" => {
                    if seen_dirs.insert(dir.clone()) {
                        model.go_modules.push(parse_go_module(&dir, &path));
                    }
                }
                "requirements.txt" | "pyproject.toml" | "Pipfile" => {
                    if seen_dirs.insert(dir.clone()) {
                        let kind = if filename == "requirements.txt" {
                            PythonKind::Requirements
                        } else if filename == "pyproject.toml" {
                            PythonKind::Pyproject
                        } else {
                            PythonKind::Pipfile
                        };
                        // Directory scan order is arbitrary: `manage.py` may be
                        // read before or after the manifest. Merge the manifest
                        // kind into a project already discovered via an entry
                        // point (e.g. manage.py → PythonKind::None_) instead of
                        // creating a duplicate project that would generate a
                        // second, manifest-less Django step graph.
                        if let Some(p) = model.python_projects.iter_mut().find(|p| p.dir == dir) {
                            p.kind = kind;
                        } else {
                            model.python_projects.push(parse_python_project(&dir, kind));
                        }
                    }
                }
                "manage.py" => {
                    // Django lives inside an existing python project (or its
                    // own directory) -- record it even without requirements.
                    if let Some(p) = model.python_projects.iter_mut().find(|p| p.dir == dir) {
                        p.has_django = true;
                        p.entry = Some("manage.py".to_string());
                    } else {
                        let mut p = parse_python_project(&dir, PythonKind::None_);
                        p.has_django = true;
                        p.entry = Some("manage.py".to_string());
                        model.python_projects.push(p);
                    }
                }
                "main.py" | "app.py" | "asgi.py" => {
                    if let Some(p) = model.python_projects.iter_mut().find(|p| p.dir == dir) {
                        if p.entry.is_none() {
                            p.entry = Some(filename.to_string());
                        }
                    }
                }
                "build.gradle" | "build.gradle.kts" => {
                    if seen_dirs.insert(dir.clone()) {
                        model
                            .gradle_projects
                            .push(parse_gradle_project(&dir, &path));
                    }
                }
                "pom.xml" => {
                    if seen_dirs.insert(dir.clone()) {
                        model.maven_projects.push(parse_maven_project(&dir, &path));
                    }
                }
                "Gemfile" => {
                    if seen_dirs.insert(dir.clone()) {
                        model.ruby_projects.push(RubyProject {
                            dir,
                            manifest: path,
                        });
                    }
                }
                "composer.json" => {
                    if seen_dirs.insert(dir.clone()) {
                        model
                            .composer_projects
                            .push(parse_composer_project(&dir, &path));
                    }
                }
                "Makefile" | "makefile" | "GNUmakefile" => {
                    model.makefiles.push(path);
                }
                "Dockerfile" | "dockerfile" | "Containerfile" => {
                    model.dockerfiles.push(DockerfileEntry {
                        path,
                        dir: dir.clone(),
                    });
                }
                "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
                    if seen_dirs.insert(dir.clone()) {
                        let services = parse_compose_services(&path);
                        model.compose_files.push(ComposeFile {
                            path,
                            dir,
                            db_ports: services.db_ports,
                            web_tools: services.web_tools,
                            services: services.services,
                            parse_warning: services.warning,
                        });
                    }
                }
                _ => {
                    if filename.ends_with(".csproj") && seen_dirs.insert(dir.clone()) {
                        model.dotnet_projects.push(DotnetProject {
                            dir,
                            manifest: path,
                        });
                    } else if filename.ends_with(".sln") {
                        model.solution_files.push(path);
                    }
                }
            }
        }

        model.layout = model.classify_layout();
        Ok(model)
    }

    fn detect_root_markers(&mut self) {
        if self.root.join(".git").is_dir() {
            self.has_git = true;
        }
    }

    /// Classify the project layout from the collected manifests.
    /// Root markers (workspace manifests, compose files, .git) take
    /// precedence; nothing is hardcoded to backend/frontend names -- the
    /// actual directories drive the classification.
    fn classify_layout(&self) -> LayoutClass {
        let root = &self.root;

        let root_pkg = self
            .node_packages
            .iter()
            .find(|p| p.dir == *root && p.is_workspace_root);
        let has_root_cargo_workspace = self
            .cargo_packages
            .iter()
            .find(|p| p.dir == *root && p.is_workspace);
        let has_root_compose = self.compose_files.iter().any(|c| c.dir == *root);
        let manifest_dirs: Vec<&PathBuf> = self
            .node_packages
            .iter()
            .map(|p| &p.dir)
            .chain(self.cargo_packages.iter().map(|p| &p.dir))
            .chain(self.go_modules.iter().map(|p| &p.dir))
            .chain(self.python_projects.iter().map(|p| &p.dir))
            .chain(self.gradle_projects.iter().map(|p| &p.dir))
            .chain(self.maven_projects.iter().map(|p| &p.dir))
            .chain(self.dotnet_projects.iter().map(|p| &p.dir))
            .chain(self.ruby_projects.iter().map(|p| &p.dir))
            .collect();

        if root_pkg.is_some() || has_root_cargo_workspace.is_some() || has_root_compose {
            return LayoutClass::Monorepo;
        }

        let root_depth = |p: &&PathBuf| {
            p.strip_prefix(root)
                .map(|r| r.components().count())
                .unwrap_or(0)
        };
        let mut dirs: Vec<usize> = manifest_dirs.iter().map(root_depth).collect();
        dirs.sort_unstable();
        dirs.dedup();

        match dirs.len() {
            0 => LayoutClass::RootOnly,
            1 => {
                if dirs[0] == 0 {
                    LayoutClass::RootOnly
                } else {
                    LayoutClass::Nested
                }
            }
            _ => {
                if dirs.contains(&0) || dirs.iter().all(|d| *d <= 2) {
                    LayoutClass::Split
                } else {
                    LayoutClass::Nested
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn rel_label(dir: &Path, root: &Path) -> String {
    let rel = dir
        .strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .into_owned();
    // Render with forward slashes on every platform: labels are shown in
    // the UI and stored in profiles, so they must not depend on the host
    // path separator.
    let rel = rel.replace('\\', "/");
    if rel.is_empty() || rel == "." {
        "root".to_string()
    } else {
        rel
    }
}

fn parse_node_package(dir: &Path, manifest: &Path) -> Result<NodePackage, String> {
    let content = fs::read_to_string(manifest)
        .map_err(|e| format!("Failed to read {}: {}", manifest.display(), e))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Malformed package.json at {}: {}", manifest.display(), e))?;

    let name = value
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unnamed")
        .to_string();

    let has_dep = |pkg: &str| {
        value.get("dependencies").and_then(|d| d.get(pkg)).is_some()
            || value
                .get("devDependencies")
                .and_then(|d| d.get(pkg))
                .is_some()
            || value
                .get("peerDependencies")
                .and_then(|d| d.get(pkg))
                .is_some()
    };

    let features = NodeFeatures {
        expo: has_dep("expo"),
        react_native: has_dep("react-native"),
        vite: has_dep("vite") || has_dep("@vitejs/plugin-react"),
        next: has_dep("next"),
        nuxt: has_dep("nuxt") || has_dep("nuxt3"),
        svelte: has_dep("svelte") || has_dep("@sveltejs/kit"),
        vue: has_dep("vue") || has_dep("vue-router"),
        angular: has_dep("@angular/core"),
        electron: has_dep("electron"),
        api_framework: [
            "express",
            "fastify",
            "hono",
            "@nestjs/core",
            "@fastify/cors",
        ]
        .iter()
        .any(|p| has_dep(p)),
        swagger: has_dep("swagger-ui-express")
            || has_dep("@nestjs/swagger")
            || has_dep("fastify-swagger")
            || has_dep("@fastify/swagger"),
        is_frontend_framework: false,
        is_backend_framework: false,
        generic_node: false,
    };

    let is_workspace_root = value
        .get("workspaces")
        .and_then(|w| w.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let package_manager = detect_package_manager(dir);
    let has_lockfile = package_manager != PackageManager::Unknown;

    // Choose the runnable dev command (never a script *value*).
    let scripts_value = value.get("scripts");
    let run_cmd: Option<String> = if features.expo {
        Some("npx expo start".to_string())
    } else if features.react_native {
        Some("npx react-native start".to_string())
    } else if features.next {
        pick_npm_run_script(scripts_value).map(|s| format!("npm run {}", s))
    } else if features.nuxt {
        pick_npm_run_script(scripts_value).map(|s| format!("npm run {}", s))
    } else if features.angular {
        pick_npm_run_script(scripts_value).map(|s| format!("npm run {}", s))
    } else if features.api_framework {
        pick_npm_run_script(scripts_value).map(|s| format!("npm run {}", s))
    } else if let Some(run_script) = pick_npm_run_script(scripts_value) {
        Some(format!("npm run {}", run_script))
    } else {
        value
            .get("main")
            .and_then(|m| m.as_str())
            .filter(|main| {
                let lower = main.to_ascii_lowercase();
                lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".cjs")
            })
            .map(|main| format!("node {}", main))
    };
    // Detect a likely dev port from scripts and config files.
    let (port, port_confidence, port_source) =
        detect_node_port(dir, &run_cmd, &features, scripts_value);

    let is_frontend_framework = features.vite
        || features.next
        || features.nuxt
        || features.svelte
        || features.vue
        || features.angular
        || features.expo
        || features.react_native
        || features.electron
        || has_dep("react")
        || has_dep("react-dom")
        || has_dep("svelte");
    let is_backend_framework =
        features.api_framework || has_dep("ts-node") || has_dep("@types/node");
    // A package with a runnable script but no recognizable framework is
    // still a runnable Node.js project: it must not silently disappear.
    let is_generic_node = run_cmd.is_some() && !is_frontend_framework && !is_backend_framework;

    let mut pkg = NodePackage {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        name,
        package_manager,
        is_workspace_root,
        run_command: run_cmd,
        port,
        port_confidence,
        port_source,
        features,
        has_lockfile,
    };
    pkg.features.is_frontend_framework = is_frontend_framework;
    pkg.features.is_backend_framework = is_backend_framework || is_generic_node;
    pkg.features.generic_node = is_generic_node;
    Ok(pkg)
}

fn detect_package_manager(dir: &Path) -> PackageManager {
    for (file, pm) in [
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("package-lock.json", PackageManager::Npm),
        ("bun.lockb", PackageManager::Bun),
        ("bun.lock", PackageManager::Bun),
    ] {
        if dir.join(file).is_file() {
            return pm;
        }
    }
    PackageManager::Unknown
}

/// Detect the likely dev-server port for a Node package.
///
/// Sources, strongest first:
/// 1. explicit port in framework config files (vite/nuxt/angular) -- High;
/// 2. `--port` / `PORT=` inside the chosen run command -- Medium;
/// 3. framework default -- Low.
fn detect_node_port(
    dir: &Path,
    run_cmd: &Option<String>,
    features: &NodeFeatures,
    scripts: Option<&serde_json::Value>,
) -> (Option<u16>, AnalysisConfidence, Option<String>) {
    if let Some(port) = config_file_port(dir, features) {
        return (
            Some(port),
            AnalysisConfidence::High,
            Some("config file".to_string()),
        );
    }
    if let Some(cmd) = run_cmd {
        if let Some(port) = detect_dev_port(cmd) {
            return (
                Some(port),
                AnalysisConfidence::Medium,
                Some("run command".to_string()),
            );
        }
    }
    if let Some(scripts) = scripts {
        for (name, cmd) in scripts
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.as_str(), v)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        {
            if ["dev", "start", "serve", "preview"].contains(&name) {
                if let Some(port) = detect_dev_port(cmd) {
                    return (
                        Some(port),
                        AnalysisConfidence::Medium,
                        Some(format!("{} script", name)),
                    );
                }
            }
        }
    }
    if let Some(port) = env_file_port(dir) {
        return (
            Some(port),
            AnalysisConfidence::Medium,
            Some(".env PORT".to_string()),
        );
    }

    let default_port = if features.expo || features.react_native {
        Some(8081)
    } else if features.next || features.nuxt {
        Some(3000)
    } else if features.vite || features.svelte || features.vue {
        Some(5173)
    } else if features.angular {
        Some(4200)
    } else if features.api_framework {
        Some(3000)
    } else {
        None
    };
    match default_port {
        Some(p) => (
            Some(p),
            AnalysisConfidence::Low,
            Some("framework default".to_string()),
        ),
        None => (None, AnalysisConfidence::Low, None),
    }
}

fn config_file_port(dir: &Path, features: &NodeFeatures) -> Option<u16> {
    for file in ["vite.config.ts", "vite.config.js", "vite.config.mjs"] {
        let path = dir.join(file);
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                for pat in [r"port\s*[:=]\s*(\d{2,5})", r"--port[= ](\d{2,5})"] {
                    if let Ok(re) = regex::Regex::new(pat) {
                        if let Some(caps) = re.captures(&content) {
                            if let Ok(p) = caps[1].parse::<u16>() {
                                if p > 0 {
                                    return Some(p);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if features.nuxt {
        let re = regex::Regex::new(r"port\s*[:=]\s*(\d{2,5})")
            .expect("hardcoded regex literal is valid");
        for file in ["nuxt.config.ts", "nuxt.config.js"] {
            let path = dir.join(file);
            if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Some(caps) = re.captures(&content) {
                        if let Ok(p) = caps[1].parse::<u16>() {
                            if p > 0 {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
    }
    if features.angular {
        let path = dir.join("angular.json");
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                let re = regex::Regex::new(r#""port"\s*:\s*(\d{2,5})"#)
                    .expect("hardcoded regex literal is valid");
                if let Some(caps) = re.captures(&content) {
                    if let Ok(p) = caps[1].parse::<u16>() {
                        if p > 0 {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

fn env_file_port(dir: &Path) -> Option<u16> {
    let re = regex::Regex::new(r"(?m)^\s*PORT\s*=\s*(\d{2,5})\s*$")
            .expect("hardcoded regex literal is valid");
    for file in [".env", ".env.local", ".env.development"] {
        let path = dir.join(file);
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(caps) = re.captures(&content) {
                    if let Ok(p) = caps[1].parse::<u16>() {
                        if p > 0 {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

fn parse_cargo_package(dir: &Path, manifest: &Path) -> CargoPackage {
    let content = fs::read_to_string(manifest).unwrap_or_default();
    let value: toml::Value =
        toml::from_str(&content).unwrap_or(toml::Value::Table(Default::default()));
    let has_tauri = value
        .get("dependencies")
        .and_then(|d| d.get("tauri"))
        .is_some();
    let is_workspace = value.get("workspace").is_some();
    CargoPackage {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        has_tauri,
        is_workspace,
    }
}

fn parse_go_module(dir: &Path, manifest: &Path) -> GoModule {
    let module = fs::read_to_string(manifest)
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("module "))
                .map(|l| l.trim_start_matches("module ").trim().to_string())
        })
        .unwrap_or_default();
    GoModule {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        module,
    }
}

fn parse_python_project(dir: &Path, kind: PythonKind) -> PythonProject {
    let mut p = PythonProject {
        dir: dir.to_path_buf(),
        kind,
        has_fastapi: false,
        has_flask: false,
        has_django: false,
        entry: None,
    };

    let text = if kind == PythonKind::Requirements {
        fs::read_to_string(dir.join("requirements.txt")).unwrap_or_default()
    } else if kind == PythonKind::Pyproject {
        fs::read_to_string(dir.join("pyproject.toml")).unwrap_or_default()
    } else if kind == PythonKind::Pipfile {
        fs::read_to_string(dir.join("Pipfile")).unwrap_or_default()
    } else {
        String::new()
    };

    let lower = text.to_ascii_lowercase();
    p.has_fastapi = lower.contains("fastapi");
    p.has_flask = lower.contains("flask");
    if dir.join("manage.py").is_file() {
        p.has_django = true;
        p.entry = Some("manage.py".to_string());
    }
    if p.entry.is_none() {
        for entry in ["main.py", "app.py", "asgi.py", "wsgi.py"] {
            if dir.join(entry).is_file() {
                p.entry = Some(entry.to_string());
                break;
            }
        }
    }
    p
}

fn parse_composer_project(dir: &Path, manifest: &Path) -> ComposerProject {
    let content = fs::read_to_string(manifest)
        .unwrap_or_default()
        .to_ascii_lowercase();
    ComposerProject {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        is_symfony: content.contains("symfony/") || content.contains("symfony/framework-bundle"),
        is_laravel: content.contains("laravel/") || content.contains("laravel/framework"),
    }
}

fn parse_gradle_project(dir: &Path, manifest: &Path) -> GradleProject {
    let content = fs::read_to_string(manifest).unwrap_or_default();
    let is_spring = content.contains("org.springframework.boot") || content.contains("spring-boot");
    GradleProject {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        is_spring,
    }
}

fn parse_maven_project(dir: &Path, manifest: &Path) -> MavenProject {
    let content = fs::read_to_string(manifest).unwrap_or_default();
    let is_spring = content.contains("spring-boot");
    MavenProject {
        dir: dir.to_path_buf(),
        manifest: manifest.to_path_buf(),
        is_spring,
    }
}

/// Parse a docker-compose file with a real YAML parser. Service names,
/// images, build contexts, `ports:` and `depends_on:` are extracted
/// structurally, so ports and service identities are direct evidence
/// (High confidence) rather than textual guesses.
#[derive(Debug, Clone, Default)]
pub(crate) struct ComposeServices {
    pub(crate) db_ports: Vec<(u16, String)>,
    pub(crate) web_tools: Vec<(u16, String)>,
    pub(crate) services: Vec<ComposeService>,
    pub(crate) warning: Option<String>,
}

/// Parse a compose file into structured services. Shared with the profile
/// builder so both paths agree on what the compose file actually publishes.
pub(crate) fn parse_compose_services(path: &Path) -> ComposeServices {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return ComposeServices {
                warning: Some(format!("Failed to read: {}", e)),
                ..ComposeServices::default()
            }
        }
    };

    let value: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return ComposeServices {
                warning: Some(format!("YAML parse failed: {}", e)),
                ..ComposeServices::default()
            }
        }
    };

    let mut result = ComposeServices::default();
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let services_map = match value.get("services").and_then(|s| s.as_mapping()) {
        Some(m) => m,
        None => {
            if !content.trim().is_empty() {
                result.warning = Some("No 'services' key found in compose file".to_string());
            }
            return result;
        }
    };

    for (name_val, spec_val) in services_map {
        let name = name_val.as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let Some(spec) = spec_val.as_mapping() else {
            result.warning = Some(format!("Service '{}' is not a mapping", name));
            continue;
        };

        let get_str = |key: &str| {
            spec.get(key)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };

        // build: <dir> | build: { context: <dir>, ... }
        let build_context = spec.get("build").and_then(|b| {
            if let Some(s) = b.as_str() {
                Some(s.to_string())
            } else if let Some(m) = b.as_mapping() {
                m.get("context").and_then(|c| c.as_str()).map(String::from)
            } else {
                None
            }
        });
        let image = get_str("image");

        // depends_on: [a, b] | depends_on: { a: { condition: ... } }
        let mut depends_on: Vec<String> = spec
            .get("depends_on")
            .and_then(|d| d.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if depends_on.is_empty() {
            depends_on = spec
                .get("depends_on")
                .and_then(|d| d.as_mapping())
                .map(|m| {
                    m.keys()
                        .filter_map(|k| k.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
        }

        let mut host_ports: Vec<u16> = Vec::new();
        let mut container_ports: Vec<u16> = Vec::new();
        if let Some(ports) = spec.get("ports").and_then(|p| p.as_sequence()) {
            for p in ports {
                let raw = match p {
                    serde_yaml::Value::String(s) => s.clone(),
                    serde_yaml::Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if let Some((host, container)) = parse_compose_port(&raw) {
                    if let Some(h) = host {
                        host_ports.push(h);
                    }
                    container_ports.push(container);
                }
            }
        }
        // expose: container-only ports (no host mapping).
        if let Some(exp) = spec.get("expose").and_then(|e| e.as_sequence()) {
            for p in exp {
                if let Some(s) = p.as_str() {
                    if let Ok(c) = s.parse::<u16>() {
                        container_ports.push(c);
                    }
                } else if let Some(n) = p.as_u64() {
                    container_ports.push(n as u16);
                }
            }
        }

        let lower_name = name.to_ascii_lowercase();
        let lower_image = image
            .as_deref()
            .map(|i| i.to_ascii_lowercase())
            .unwrap_or_default();

        let is_db = KNOWN_DB_SERVICES
            .iter()
            .any(|(n, _)| lower_name.contains(n))
            || KNOWN_DB_SERVICES
                .iter()
                .any(|(n, _)| lower_image.contains(n))
            || container_ports
                .iter()
                .any(|p| KNOWN_DB_SERVICES.iter().any(|(_, port)| port == p))
            || host_ports
                .iter()
                .any(|p| KNOWN_DB_SERVICES.iter().any(|(_, port)| port == p));
        let web_tool = KNOWN_WEB_TOOLS
            .iter()
            .find(|(n, _)| lower_name.contains(n) || lower_image.contains(n))
            .map(|(n, _)| n.to_string());

        let build_context = build_context.map(|ctx| {
            let p = PathBuf::from(&ctx);
            if p.is_absolute() {
                p
            } else {
                dir.join(p)
            }
        });

        result.services.push(ComposeService {
            name,
            build_context,
            image,
            host_ports,
            container_ports,
            depends_on,
            is_db,
            web_tool,
        });
    }

    for svc in &result.services {
        // A database without an explicit host port cannot be waited on
        // from the host; surface it instead of silently dropping the wait.
        if svc.is_db && svc.host_ports.is_empty() {
            result.warning = Some(format!(
                "Service '{}' (database) publishes no explicit host port; readiness wait omitted",
                svc.name
            ));
        }
        for p in &svc.host_ports {
            if svc.is_db {
                result.db_ports.push((*p, svc.name.clone()));
            }
            if let Some(tool) = &svc.web_tool {
                result.web_tools.push((*p, tool.clone()));
            }
        }
    }
    result.db_ports.sort();
    result.db_ports.dedup();
    result.web_tools.sort();
    result.web_tools.dedup();
    result
}

/// Parse a compose `ports:` entry into `(host_port, container_port)`.
/// Handles `"HOST:CONTAINER"`, `"IP:HOST:CONTAINER"`, bare `"CONTAINER"`
/// (anonymous host port), numeric values and `/proto` suffixes.
fn parse_compose_port(raw: &str) -> Option<(Option<u16>, u16)> {
    let raw = raw.split('/').next().unwrap_or(raw);
    let parts: Vec<&str> = raw.split(':').collect();
    match parts.len() {
        1 => {
            let c = parts[0].parse::<u16>().ok()?;
            if c == 0 {
                return None;
            }
            Some((None, c))
        }
        2 => {
            let h = parts[0].parse::<u16>().ok()?;
            let c = parts[1].parse::<u16>().ok()?;
            if h == 0 || c == 0 {
                return None;
            }
            Some((Some(h), c))
        }
        3 => {
            let h = parts[1].parse::<u16>().ok()?;
            let c = parts[2].parse::<u16>().ok()?;
            if h == 0 || c == 0 {
                return None;
            }
            Some((Some(h), c))
        }
        _ => None,
    }
}

const KNOWN_DB_SERVICES: &[(&str, u16)] = &[
    ("postgres", 5432),
    ("pg", 5432),
    ("mysql", 3306),
    ("mariadb", 3306),
    ("redis", 6379),
    ("mongo", 27017),
    ("mongodb", 27017),
    ("rabbitmq", 5672),
    ("elasticsearch", 9200),
    ("elastic", 9200),
    ("cassandra", 9042),
    ("mssql", 1433),
    ("sqlserver", 1433),
    ("clickhouse", 8123),
    ("kafka", 9092),
    ("redpanda", 9092),
    ("zookeeper", 2181),
];

/// Compose services that are *web tools* (browser UIs), matched by the
/// service name. The default URL path is "/" — dashboards are typically
/// served at the root.
const KNOWN_WEB_TOOLS: &[(&str, &str)] = &[
    ("grafana", "/"),
    ("airflow", "/"),
    ("prometheus", "/"),
    ("nginx", "/"),
    ("kafka-ui", "/"),
    ("kafkaui", "/"),
    ("portainer", "/"),
    ("sonarqube", "/"),
    ("jenkins", "/"),
    ("keycloak", "/"),
    ("superset", "/"),
    ("metabase", "/"),
    ("minio", "/"),
    ("pgadmin", "/"),
    ("phpmyadmin", "/"),
    ("mailhog", "/"),
    ("rabbitmq", "/"),
    ("redis-commander", "/"),
];

// ---------------------------------------------------------------------------
// IDE / application resolution
// ---------------------------------------------------------------------------

/// Resolve an application name to an executable path.
///
/// The IDE resolver is tried first (broader PATH / App Paths coverage for
/// developer tools). When it misses, the generic application launcher is
/// used (well-known install directories, Windows App Paths registry).
/// Flatpak invocations are not usable as a bare `OpenApplication` path,
/// so they are rejected here.
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

/// Per-OS CLI names for auxiliary tools.
fn android_studio_cli_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "studio64"
    } else if cfg!(target_os = "macos") {
        "studio"
    } else {
        "android-studio"
    }
}

/// Resolve the first available database viewer: the user-configured path
/// wins, otherwise the first supported candidate found on the host.
fn resolve_db_viewer(preferred: Option<&str>) -> Option<(String, String)> {
    for cli in super::profile_builder::db_viewer_candidates(preferred) {
        if let Some(resolved) = resolve_app(&cli) {
            return Some((
                resolved,
                super::profile_builder::db_viewer_display_name(&cli),
            ));
        }
    }
    None
}

/// Resolve the Docker Desktop application for the "Open Docker Desktop"
/// step. On macOS the canonical launcher is `open -a Docker`; elsewhere the
/// Desktop app is resolved through the platform resolvers (never the bare
/// docker CLI — that prints help instead of starting the daemon UI).
fn resolve_docker_desktop() -> Option<(String, Vec<String>)> {
    if cfg!(target_os = "macos") {
        return Some((
            "open".to_string(),
            vec!["-a".to_string(), "Docker".to_string()],
        ));
    }
    let name = if cfg!(target_os = "windows") {
        "Docker Desktop"
    } else {
        "docker-desktop"
    };
    if let Some(p) = resolve_app(name) {
        return Some((p, Vec::new()));
    }
    // Windows last resort: Start Menu shortcut. Covers Store/MSIX installs
    // that register neither App Paths nor a plain exe path; the structured
    // launcher carries the `shell:AppsFolder` alias arguments.
    #[cfg(target_os = "windows")]
    {
        if let Some(l) = crate::platform::app_launcher::start_menu_launcher("Docker Desktop") {
            return Some((l.program, l.args));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pick npm script (kept from the legacy analyzer -- script NAMES only)
// ---------------------------------------------------------------------------

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
    obj.keys().next().cloned()
}

/// Detect a port from a command string (`--port N`, `--port=N`, `-p N`,
/// `PORT=N`).
fn detect_dev_port(run_cmd: &str) -> Option<u16> {
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
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Python virtual-environment commands (shared with the profile builder)
// ---------------------------------------------------------------------------

/// Run `args` with the project's virtual-environment interpreter
/// (`.venv\Scripts\python.exe` on Windows, `.venv/bin/python` elsewhere).
///
/// The relative path is resolved by the orchestrator against the step's
/// working directory at spawn time, with a graceful fallback to an
/// ancestor `.venv` or the system interpreter when the venv is missing.
pub(crate) fn python_venv_run(args: &str) -> String {
    // Windows `cmd` does not accept forward-slash paths in a command token:
    // `.venv/Scripts/python.exe` fails with "'.venv' is not recognized as an
    // internal or external command". Use native separators.
    if cfg!(target_os = "windows") {
        ".venv\\Scripts\\python.exe ".to_string() + args
    } else {
        "./.venv/bin/python ".to_string() + args
    }
}

/// Install dependencies into the project venv, creating the venv first only
/// when it does not exist yet (idempotent). `install_args` is the venv-python
/// invocation to run once the venv is guaranteed (e.g. `-m pip install -r
/// requirements.txt`).
pub(crate) fn python_ensure_venv_install(install_args: &str) -> String {
    if cfg!(target_os = "windows") {
        format!(
            "cmd /C \"(if not exist .venv\\Scripts\\python.exe python -m venv .venv) && .venv\\Scripts\\python.exe {}\"",
            install_args
        )
    } else {
        format!(
            "sh -c \"([ -x .venv/bin/python ] || python3 -m venv .venv) && .venv/bin/python {}\"",
            install_args
        )
    }
}

/// The `pip`-style install target for a project's dependency manifest kind.
/// `None` means the project has no dependency manifest to install.
fn python_install_target(kind: PythonKind) -> Option<&'static str> {
    match kind {
        PythonKind::Requirements => Some("-m pip install -r requirements.txt"),
        PythonKind::Pyproject => Some("-m pip install -e ."),
        PythonKind::Pipfile => Some("-m pipenv install"),
        PythonKind::None_ => None,
    }
}

// ---------------------------------------------------------------------------
// Step graph generation
// ---------------------------------------------------------------------------

/// A step assembled during generation, before IDs are assigned.
struct PendingStep {
    label: String,
    enabled: bool,
    kind: StepKind,
    depends_on: Vec<String>,
    /// Explicit absolute working directory (or explicit project root).
    /// Never empty: every generated action anchors its paths to an
    /// explicit project root or an explicit absolute path.
    working_directory: Option<String>,
    visibility: Option<Visibility>,
    execution_mode: Option<ExecutionMode>,
    completion: Option<CompletionPolicy>,
    timeout: Option<u64>,
    retry_policy: Option<RetryPolicy>,
    metadata: Option<HashMap<String, String>>,
}

impl PendingStep {
    fn run(
        label: &str,
        command: &str,
        working_dir: Option<&Path>,
        project_root: &Path,
        visible: bool,
    ) -> Self {
        // Resolve to an explicit absolute path now -- a saved profile must
        // never depend on the globally active workspace to resolve paths.
        let working_directory = working_dir
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_else(|| project_root.to_string_lossy().into_owned());
        PendingStep {
            label: label.to_string(),
            enabled: true,
            kind: StepKind::RunCommand {
                command: command.to_string(),
                command_spec: None,
            },
            depends_on: Vec::new(),
            working_directory: Some(working_directory),
            visibility: Some(if visible {
                Visibility::VisibleTerminal
            } else {
                Visibility::Captured
            }),
            execution_mode: Some(if visible {
                ExecutionMode::LongRunning
            } else {
                ExecutionMode::OneShot
            }),
            completion: Some(if visible {
                CompletionPolicy::ProcessStarted
            } else {
                CompletionPolicy::ExitSuccess
            }),
            timeout: None,
            retry_policy: None,
            metadata: None,
        }
    }

    fn wait_port(host: &str, port: u16, timeout: u64, depends_on: &str) -> Self {
        PendingStep::wait_port_candidates(host, port, &[], timeout, depends_on)
    }

    fn wait_port_candidates(
        host: &str,
        port: u16,
        candidate_ports: &[u16],
        timeout: u64,
        depends_on: &str,
    ) -> Self {
        let mut candidates: Vec<u16> = candidate_ports.to_vec();
        candidates.retain(|p| *p != port);
        candidates.dedup();
        PendingStep {
            label: if candidates.is_empty() {
                format!("Wait for port {}:{}", host, port)
            } else {
                format!(
                    "Wait for port {}:{} (or {})",
                    host,
                    port,
                    candidates
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            enabled: true,
            kind: StepKind::WaitForPort {
                host: host.to_string(),
                port,
                candidate_ports: candidates.clone(),
            },
            depends_on: vec![depends_on.to_string()],
            working_directory: None,
            visibility: None,
            execution_mode: None,
            completion: Some(CompletionPolicy::PortOpen {
                host: host.to_string(),
                port,
                timeout_secs: timeout,
            }),
            timeout: Some(timeout),
            // Wait steps are retried with backoff: dev servers routinely
            // need a second chance on cold starts.
            retry_policy: Some(RetryPolicy {
                max_retries: 2,
                delay_ms: 2000,
                backoff_multiplier: Some(1.5),
            }),
            metadata: None,
        }
    }

    fn open_app(path: &str, args: Vec<String>, label: &str) -> Self {
        PendingStep {
            label: label.to_string(),
            enabled: true,
            kind: StepKind::OpenApplication {
                path: path.to_string(),
                args: if args.is_empty() { None } else { Some(args) },
            },
            depends_on: Vec::new(),
            working_directory: None,
            visibility: Some(Visibility::Detached),
            execution_mode: None,
            completion: Some(CompletionPolicy::ExternalLaunchAccepted),
            timeout: None,
            retry_policy: None,
            metadata: None,
        }
    }

    fn open_url(url: &str, label: &str) -> Self {
        PendingStep {
            label: label.to_string(),
            enabled: true,
            kind: StepKind::OpenUrl {
                url: url.to_string(),
            },
            depends_on: Vec::new(),
            working_directory: None,
            visibility: Some(Visibility::Detached),
            execution_mode: None,
            completion: Some(CompletionPolicy::ExternalLaunchAccepted),
            timeout: None,
            retry_policy: None,
            metadata: None,
        }
    }

    fn with_depends_on(mut self, dep: &str) -> Self {
        self.depends_on.push(dep.to_string());
        self
    }

    fn wait_docker(timeout: u64, depends_on: Option<&str>) -> Self {
        PendingStep {
            label: "Wait for Docker daemon".to_string(),
            enabled: true,
            kind: StepKind::WaitForDocker {},
            depends_on: depends_on.map(|d| vec![d.to_string()]).unwrap_or_default(),
            working_directory: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: Some(timeout),
            retry_policy: None,
            metadata: None,
        }
    }

    fn open_terminal() -> Self {
        PendingStep {
            label: "Open a terminal".to_string(),
            enabled: true,
            kind: StepKind::OpenTerminal {
                command: String::new(),
            },
            depends_on: Vec::new(),
            working_directory: None,
            visibility: Some(Visibility::VisibleTerminal),
            execution_mode: Some(ExecutionMode::LongRunning),
            completion: Some(CompletionPolicy::ProcessStarted),
            timeout: None,
            retry_policy: None,
            metadata: None,
        }
    }

    fn with_metadata(mut self, key: &str, value: &str) -> Self {
        let meta = self.metadata.get_or_insert_with(HashMap::new);
        meta.insert(key.to_string(), value.to_string());
        self
    }
}

/// Assign deterministic step IDs and resolve failure policies.
fn finalize_steps(
    pending: Vec<PendingStep>,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
) -> Vec<LaunchStep> {
    // Compute dependents for default failure policy.
    let mut dependents: HashMap<String, usize> = HashMap::new();
    for step in &pending {
        for dep in &step.depends_on {
            *dependents.entry(dep.clone()).or_insert(0) += 1;
        }
    }

    pending
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let id = format!("step_{:03}", i + 1);
            let has_dependents = dependents.get(&id).map(|n| *n > 0).unwrap_or(false);
            let failure_policy = Some(p.kind.default_failure_policy(has_dependents));
            if !has_dependents && matches!(p.kind, StepKind::WaitForPort { .. }) {
                // Computed from the generated graph — a deterministic fact,
                // not a guess.
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::High,
                    format!(
                        "Readiness wait '{}' has no dependents; a failure will not stop the run",
                        p.label
                    ),
                    None,
                ));
            }
            LaunchStep {
                id,
                label: p.label,
                enabled: p.enabled,
                kind: p.kind,
                depends_on: p.depends_on,
                working_directory: p.working_directory,
                environment: None,
                visibility: p.visibility,
                execution_mode: p.execution_mode,
                completion: p.completion,
                timeout: p.timeout,
                failure_policy,
                retry_policy: p.retry_policy.clone(),
                metadata: p.metadata,
                extra: serde_json::Map::new(),
            }
        })
        .collect()
}

/// Default timeout for the "Wait for Docker daemon" step. A cold Docker
/// Desktop first boot (especially with a WSL2 backend) routinely exceeds
/// 2 minutes, so the wait gets generous headroom while still failing
/// loudly. Individual daemon checks are bounded by the docker service so
/// this budget is spent on polling, not on one stuck `docker version`.
const WAIT_DOCKER_TIMEOUT_SECS: u64 = 180;

/// Default timeout for the "Start Docker Compose" verification. Image
/// builds (npm ci, pip install, ...) routinely take minutes on cold
/// starts; the step still fails fast when the compose command itself
/// exits non-zero (the startup probe reports the real exit code).
const COMPOSE_UP_TIMEOUT_SECS: u64 = 600;

/// Default timeout for generated port-readiness waits. Dev servers
/// (Metro, Go, Django, ...) need headroom for cold starts; combined with
/// the retry policy this yields ~90s + two backoff re-arms.
const WAIT_PORT_TIMEOUT_SECS: u64 = 90;

/// Generate the draft step graph from the project model.
fn generate_steps(
    model: &ProjectModel,
    options: &AnalyzeOptions,
    diagnostics: &mut Vec<AnalysisDiagnostic>,
) -> Vec<LaunchStep> {
    let root = &model.root;
    let mut steps: Vec<PendingStep> = Vec::new();
    let mut known: HashSet<(String, String)> = HashSet::new();

    // --- 1. IDE steps (roots, only when resolvable) ---
    if options.include_ide_steps {
        let ide_name = options.vscode_path.as_deref().unwrap_or("code");
        if let Some(resolved) = resolve_app(ide_name) {
            steps.push(PendingStep::open_app(
                &resolved,
                vec![".".to_string()],
                "Open VS Code",
            ));
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Medium,
                "VS Code not found; 'Open VS Code' step omitted".to_string(),
                None,
            ));
        }

        let has_mobile = model
            .node_packages
            .iter()
            .any(|p| p.features.expo || p.features.react_native);
        if has_mobile {
            if let Some(resolved) = resolve_app(android_studio_cli_name()) {
                steps.push(PendingStep::open_app(
                    &resolved,
                    vec![".".to_string()],
                    "Open Android Studio",
                ));
            } else {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::Medium,
                    "Android Studio not found; mobile IDE step omitted".to_string(),
                    None,
                ));
            }
        }
    }

    // --- 2. Docker infrastructure (tool steps, conditional) ---
    let docker_present = !model.compose_files.is_empty() || !model.dockerfiles.is_empty();
    let mut docker_wait_id: Option<String> = None;
    if options.include_tool_steps && docker_present {
        // "Open Docker Desktop" is always emitted so the launch intent is
        // visible in the profile. When the app cannot be resolved the step
        // is DISABLED (with a warning) instead of silently omitted; the
        // daemon wait below depends on it either way, so a disabled open
        // step does not block the wait.
        match resolve_docker_desktop() {
            Some((resolved, args)) => {
                steps.push(PendingStep::open_app(
                    &resolved,
                    args,
                    "Open Docker Desktop",
                ));
            }
            None => {
                let mut disabled =
                    PendingStep::open_app("docker-desktop", Vec::new(), "Open Docker Desktop");
                disabled.enabled = false;
                steps.push(disabled);
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    AnalysisConfidence::Medium,
                    "Docker Desktop not resolvable; 'Open Docker Desktop' is disabled \
                     and the daemon wait will not auto-launch it"
                        .to_string(),
                    None,
                ));
            }
        }
        docker_wait_id = Some(format!("step_{:03}", steps.len() + 1));
        // The daemon check is self-healing (it can launch Docker Desktop), so
        // it must not be blocked by the best-effort GUI launch above.  Store
        // and portable installs often report a transient launch error even
        // though the daemon becomes ready a few seconds later.
        steps.push(PendingStep::wait_docker(WAIT_DOCKER_TIMEOUT_SECS, None));

        let has_db = model.compose_files.iter().any(|c| !c.db_ports.is_empty());
        if has_db {
            if let Some((resolved, display_name)) =
                resolve_db_viewer(options.db_viewer_path.as_deref())
            {
                steps.push(PendingStep {
                    label: format!("Open {}", display_name),
                    enabled: true,
                    kind: StepKind::OpenApplication {
                        path: resolved,
                        args: None,
                    },
                    depends_on: vec![docker_wait_id
                        .clone()
                        .expect(
                            "docker_wait_id is Some: set at the top of this branch (include_tool_steps && docker_present)",
                        )],
                    working_directory: None,
                    visibility: Some(Visibility::Detached),
                    execution_mode: None,
                    completion: Some(CompletionPolicy::ExternalLaunchAccepted),
                    timeout: None,
                    retry_policy: None,
                    metadata: None,
                });
            } else {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::Medium,
                    "No supported database viewer found; database client step omitted".to_string(),
                    None,
                ));
            }
        }
    }

    // --- 3. Docker compose + database readiness (unconditional) ---
    let mut compose_ids: Vec<String> = Vec::new();
    // Directories that a compose service builds from: the container provides
    // the service, so a local run there would conflict (port clash). Maps
    // dir -> (compose step id, governing service names).
    let mut compose_governed: HashMap<PathBuf, (String, Vec<String>)> = HashMap::new();
    for compose in &model.compose_files {
        let label = format!("Start Docker Compose ({})", rel_label(&compose.dir, root));
        if !known.insert(("compose".to_string(), compose.dir.display().to_string())) {
            continue;
        }
        // Visible terminal: the user sees the image build and container
        // startup output — hidden failures (exit code 1) are the #1
        // support question. Readiness is still verified by the port waits.
        let mut step = PendingStep::run(
            &label,
            "docker compose up -d",
            Some(&compose.dir),
            root,
            true,
        );
        // The compose bootstrap must NOT complete on "process started": a
        // build that fails minutes later (broken Dockerfile, missing
        // package-lock.json for `npm ci`) would falsely report success. The
        // DockerComposeUp completion keeps polling — via a captured
        // `docker compose ps`, never an extra terminal window — and the
        // command's real exit code until containers are actually running.
        step.completion = Some(CompletionPolicy::DockerComposeUp {
            timeout_secs: COMPOSE_UP_TIMEOUT_SECS,
        });
        if let Some(w) = &docker_wait_id {
            step.depends_on.push(w.clone());
        }
        let compose_id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        compose_ids.push(compose_id.clone());

        for svc in &compose.services {
            if let Some(ctx) = &svc.build_context {
                let ctx = ctx.clone();
                compose_governed
                    .entry(ctx)
                    .and_modify(|(_, names)| {
                        if !names.contains(&svc.name) {
                            names.push(svc.name.clone());
                        }
                    })
                    .or_insert_with(|| (compose_id.clone(), vec![svc.name.clone()]));
            }
        }

        for (port, service) in &compose.db_ports {
            let key = format!("{}:{}", compose.dir.display(), port);
            if !known.insert(("db_wait".to_string(), key)) {
                continue;
            }
            // Kafka (and Redpanda) brokers are the slowest containers to
            // become reachable — image pull + broker bootstrap routinely
            // exceeds the generic 90s on cold starts, so they get extra
            // headroom.
            let is_kafka = matches!(
                service.to_ascii_lowercase().as_str(),
                "kafka" | "redpanda"
            );
            let timeout = if is_kafka {
                180
            } else {
                WAIT_PORT_TIMEOUT_SECS
            };
            // Many Kafka/KRaft compose templates expose the host-facing
            // listener on 29092 instead of 9092; accept it as an alternative
            // so the wait does not time out against the published port.
            let mut wait = if is_kafka && *port == 9092 {
                PendingStep::wait_port_candidates(
                    "127.0.0.1",
                    *port,
                    &[29092],
                    timeout,
                    &compose_id,
                )
            } else {
                PendingStep::wait_port("127.0.0.1", *port, timeout, &compose_id)
            };
            wait.label = format!(
                "Wait for {} ({})",
                if service.is_empty() {
                    "database".to_string()
                } else {
                    service.clone()
                },
                port
            );
            wait = wait
                .with_metadata("confidence", "high")
                .with_metadata("source", "docker-compose ports");
            steps.push(wait);
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::High,
                format!(
                    "Database port {} for compose service '{}' (explicit host port in {}); readiness wait added",
                    port,
                    service,
                    compose.path.display()
                ),
                Some(compose.path.display().to_string()),
            ));
        }

        // Web tools (grafana, airflow, ...): wait for the port, then open
        // the dashboard in the browser when it is up.
        for (port, tool) in &compose.web_tools {
            let key = format!("{}:{}", compose.dir.display(), port);
            if !known.insert(("web_tool".to_string(), key)) {
                continue;
            }
            let mut wait =
                PendingStep::wait_port("127.0.0.1", *port, WAIT_PORT_TIMEOUT_SECS, &compose_id);
            wait.label = format!("Wait for {} ({})", tool, port);
            wait = wait
                .with_metadata("confidence", "high")
                .with_metadata("source", "docker-compose web tool");
            let wait_id = format!("step_{:03}", steps.len() + 1);
            steps.push(wait);

            let url = format!("http://localhost:{}", port);
            let open = PendingStep::open_url(&url, &format!("Open {} ({})", tool, url))
                .with_depends_on(&wait_id);
            steps.push(open);
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::High,
                format!(
                    "Web tool '{}' from compose service (host port {}); browser step added",
                    tool, port
                ),
                Some(compose.path.display().to_string()),
            ));
        }
    }
    let compose_id: Option<String> = compose_ids.first().cloned();

    // --- 4. Backend services ---
    for go in &model.go_modules {
        let dir_label = rel_label(&go.dir, root);
        let label = if dir_label == "root" {
            "Run Go backend".to_string()
        } else {
            format!("Run Go backend ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let go_command = go_run_command(&go.dir);
        let mut step = PendingStep::run(&label, &go_command, Some(&go.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        // Go ports are rarely declared in the manifest; the hint is
        // source-aware (High for .env PORT / ListenAndServe, Medium for an
        // address literal, Low for the 8080 fallback).
        let (port, port_confidence, port_source) =
            go_port_hint(&go.dir).unwrap_or((8080, AnalysisConfidence::Low, "default".to_string()));
        let mut wait = PendingStep::wait_port("127.0.0.1", port, WAIT_PORT_TIMEOUT_SECS, &id);
        wait.label = format!("Wait for Go backend port {}", port);
        wait = wait
            .with_metadata("confidence", port_confidence.as_str())
            .with_metadata("source", &port_source);
        steps.push(wait);
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            port_confidence,
            format!(
                "Go backend port {} from {} (confidence {})",
                port,
                port_source,
                port_confidence.as_str()
            ),
            Some(go.manifest.display().to_string()),
        ));
    }

    for cargo in &model.cargo_packages {
        if cargo.is_workspace {
            continue;
        }
        let dir_label = rel_label(&cargo.dir, root);
        let (cmd, port, wait_label) = if cargo.has_tauri {
            ("cargo tauri dev", Some(1420), "Wait for Tauri port")
        } else {
            ("cargo run", Some(3000), "Wait for Rust port")
        };
        let label = if dir_label == "root" {
            "Run Rust project".to_string()
        } else {
            format!("Run Rust project ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, cmd, Some(&cargo.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        if let Some(port) = port {
            let mut wait = PendingStep::wait_port("127.0.0.1", port, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = wait_label.to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
        }
    }

    for py in &model.python_projects {
        let dir_label = rel_label(&py.dir, root);
        if py.has_django {
            let label = if dir_label == "root" {
                "Run Django server".to_string()
            } else {
                format!("Run Django server ({})", dir_label)
            };
            // The install step creates the venv (only when missing) and
            // installs dependencies, then finishes. It is a CAPTURED one-shot
            // that waits for real process exit, so the server step that
            // depends on it can never race ahead of the venv bootstrap. The
            // command never depends on the state of `.venv` at analysis time.
            let install_id = python_install_target(py.kind).map(|target| {
                let install = PendingStep::run(
                    "Install Django dependencies",
                    &python_ensure_venv_install(target),
                    Some(&py.dir),
                    root,
                    false,
                );
                let id = format!("step_{:03}", steps.len() + 1);
                steps.push(install);
                id
            });
            let python_cmd = python_venv_run("manage.py runserver");
            let mut step = PendingStep::run(&label, &python_cmd, Some(&py.dir), root, true);
            if let Some(iid) = &install_id {
                step.depends_on.push(iid.clone());
            }
            if let Some(cid) = &compose_id {
                step.depends_on.push(cid.clone());
            }
            let id = format!("step_{:03}", steps.len() + 1);
            steps.push(step);
            let mut wait = PendingStep::wait_port("127.0.0.1", 8000, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = "Wait for Django port 8000".to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
            // DB migration is state-changing -- never enabled by default. It
            // runs with the SAME interpreter as the server (the project venv)
            // so migrations apply to the environment that actually runs the
            // server, never the system python.
            let mut migrate = PendingStep::run(
                "Run Django migrate (disabled by default)",
                &python_venv_run("manage.py migrate"),
                Some(&py.dir),
                root,
                false,
            );
            migrate.enabled = false;
            if let Some(iid) = &install_id {
                migrate.depends_on.push(iid.clone());
            }
            migrate = migrate.with_metadata("policy", "manual-enable");
            steps.push(migrate);
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Warning,
                AnalysisConfidence::High,
                "Django migrate step generated but DISABLED (state-changing action; enable manually)"
                    .to_string(),
                Some(py.dir.join("manage.py").display().to_string()),
            ));
            continue;
        }

        if py.has_fastapi || py.has_flask {
            let is_fastapi = py.has_fastapi;
            let entry = py.entry.as_deref().unwrap_or("main:app");
            let cmd = if is_fastapi {
                let module = entry.strip_suffix(".py").unwrap_or(entry).replace('/', ".");
                let target = if module.contains(':') {
                    module
                } else {
                    format!("{}:app", module)
                };
                format!("uvicorn {} --reload", target)
            } else {
                "flask run --debug".to_string()
            };
            let port = if is_fastapi { 8000 } else { 5000 };
            let framework = if is_fastapi { "FastAPI" } else { "Flask" };
            let label = if dir_label == "root" {
                format!("Run {} backend", framework)
            } else {
                format!("Run {} backend ({})", framework, dir_label)
            };
            let mut step = PendingStep::run(&label, &cmd, Some(&py.dir), root, true);
            if let Some(cid) = &compose_id {
                step.depends_on.push(cid.clone());
            }
            let id = format!("step_{:03}", steps.len() + 1);
            steps.push(step);
            let mut wait = PendingStep::wait_port("127.0.0.1", port, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = format!("Wait for {} port {}", framework, port);
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);

            // API docs: inferred from the framework, enabled.
            if is_fastapi {
                let url = format!("http://localhost:{}/docs", port);
                let mut docs = PendingStep::open_url(&url, "Open FastAPI docs");
                docs.depends_on.push(id.clone());
                docs = docs.with_metadata("source", "fastapi default /docs");
                steps.push(docs);
            }
        }
    }

    for gradle in &model.gradle_projects {
        let dir_label = rel_label(&gradle.dir, root);
        let cmd = if gradle.is_spring {
            if cfg!(target_os = "windows") {
                "gradlew.bat bootRun"
            } else {
                "./gradlew bootRun"
            }
        } else {
            if cfg!(target_os = "windows") {
                "gradlew.bat run"
            } else {
                "./gradlew run"
            }
        };
        let label = if dir_label == "root" {
            "Run Gradle project".to_string()
        } else {
            format!("Run Gradle project ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, cmd, Some(&gradle.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        if gradle.is_spring {
            let mut wait = PendingStep::wait_port("127.0.0.1", 8080, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = "Wait for Spring Boot port 8080".to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
        }
    }

    for maven in &model.maven_projects {
        let dir_label = rel_label(&maven.dir, root);
        let cmd = if maven.is_spring {
            if cfg!(target_os = "windows") {
                "mvnw.cmd spring-boot:run"
            } else {
                "./mvnw spring-boot:run"
            }
        } else {
            if cfg!(target_os = "windows") {
                "mvnw.cmd exec:java"
            } else {
                "./mvnw exec:java"
            }
        };
        let label = if dir_label == "root" {
            "Run Maven project".to_string()
        } else {
            format!("Run Maven project ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, cmd, Some(&maven.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        if maven.is_spring {
            let mut wait = PendingStep::wait_port("127.0.0.1", 8080, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = "Wait for Spring Boot port 8080".to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
        }
    }

    for dotnet in &model.dotnet_projects {
        let dir_label = rel_label(&dotnet.dir, root);
        let label = if dir_label == "root" {
            "Run .NET project".to_string()
        } else {
            format!("Run .NET project ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, "dotnet run", Some(&dotnet.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        let mut wait = PendingStep::wait_port("127.0.0.1", 5000, WAIT_PORT_TIMEOUT_SECS, &id);
        wait.label = "Wait for .NET port 5000".to_string();
        wait = wait.with_metadata("confidence", "low");
        steps.push(wait);
    }

    for ruby in &model.ruby_projects {
        let dir_label = rel_label(&ruby.dir, root);
        let label = if dir_label == "root" {
            "Run Rails server".to_string()
        } else {
            format!("Run Rails server ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(
            &label,
            "bundle exec rails server",
            Some(&ruby.dir),
            root,
            true,
        );
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        let mut wait = PendingStep::wait_port("127.0.0.1", 3000, WAIT_PORT_TIMEOUT_SECS, &id);
        wait.label = "Wait for Rails port 3000".to_string();
        wait = wait.with_metadata("confidence", "low");
        steps.push(wait);
    }

    // --- 5. PHP Composer applications (Symfony/Laravel) ---
    for php in &model.composer_projects {
        if !php.is_symfony && !php.is_laravel {
            continue;
        }
        let framework = if php.is_symfony { "Symfony" } else { "Laravel" };
        let dir_label = rel_label(&php.dir, root);
        let label = if dir_label == "root" {
            format!("Start {} backend", framework)
        } else {
            format!("Start {} backend ({})", framework, dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }

        let install_id = format!("step_{:03}", steps.len() + 1);
        steps.push(PendingStep::run(
            &format!("Install {} dependencies", framework),
            "composer install --no-interaction",
            Some(&php.dir),
            root,
            false,
        ));

        let command = if php.is_symfony && resolve_app("symfony").is_some() {
            "symfony server:start --no-tls --allow-http --port=8000"
        } else if php.is_laravel {
            "php artisan serve --host=127.0.0.1 --port=8000"
        } else {
            "php -S 127.0.0.1:8000 -t public"
        };
        let mut start = PendingStep::run(&label, command, Some(&php.dir), root, true);
        start.depends_on.push(install_id);
        if let Some(cid) = &compose_id {
            start.depends_on.push(cid.clone());
        }
        let start_id = format!("step_{:03}", steps.len() + 1);
        steps.push(start);
        let mut wait = PendingStep::wait_port("127.0.0.1", 8000, WAIT_PORT_TIMEOUT_SECS, &start_id);
        wait.label = format!("Wait for {} backend port 8000", framework);
        wait = wait.with_metadata("confidence", "medium");
        steps.push(wait);
    }

    // --- 6. Node packages: backend first, then frontend, deduplicated ---
    let mut node_backends: Vec<&NodePackage> = model
        .node_packages
        .iter()
        .filter(|p| p.features.is_backend_framework && !p.features.is_frontend_framework)
        .collect();
    node_backends.sort_by(|a, b| a.dir.cmp(&b.dir));
    for pkg in &node_backends {
        let Some(run) = &pkg.run_command else {
            continue;
        };
        let dir_label = rel_label(&pkg.dir, root);
        let kind_label = if pkg.features.generic_node {
            "Node.js project"
        } else {
            "Node.js backend"
        };
        let label = if dir_label == "root" {
            format!("Run {}", kind_label)
        } else {
            format!("Run {} ({})", kind_label, dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, run, Some(&pkg.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);

        if let Some(port) = pkg.port {
            let mut wait = PendingStep::wait_port("127.0.0.1", port, WAIT_PORT_TIMEOUT_SECS, &id);
            wait.label = format!("Wait for backend port {}", port);
            wait = wait.with_metadata("confidence", pkg.port_confidence.as_str());
            steps.push(wait);

            // API docs when a Swagger/OpenAPI tool is present.
            if pkg.features.swagger {
                let url = docs_url_for_node(pkg);
                let mut docs = PendingStep::open_url(&url, "Open API docs");
                docs.depends_on.push(id.clone());
                docs = docs.with_metadata("source", "swagger dependency");
                steps.push(docs);
            }
        }
    }

    let mut node_frontends: Vec<&NodePackage> = model
        .node_packages
        .iter()
        .filter(|p| p.features.is_frontend_framework)
        .collect();
    node_frontends.sort_by(|a, b| a.dir.cmp(&b.dir));
    for pkg in &node_frontends {
        let Some(run) = &pkg.run_command else {
            continue;
        };
        let dir_label = rel_label(&pkg.dir, root);
        let tool = if pkg.features.expo {
            "Expo"
        } else if pkg.features.react_native {
            "React Native"
        } else if pkg.features.next {
            "Next.js"
        } else if pkg.features.nuxt {
            "Nuxt"
        } else if pkg.features.angular {
            "Angular"
        } else if pkg.features.vite || pkg.features.svelte || pkg.features.vue {
            "Vite"
        } else if pkg.features.electron {
            "Electron"
        } else {
            "frontend"
        };
        let label = if dir_label == "root" {
            format!("Start {} dev server", tool)
        } else {
            format!("Start {} dev server ({})", tool, dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        // A Next.js/Nuxt frontend defaults to 3000; when a node backend also
        // wants 3000 — or a compose service publishes 3000/3001 (e.g. the
        // wizard's Grafana on host 3001) — the frontend is re-pinned to the
        // first free port starting at 3001. Ports published by compose and
        // backend ports are reserved, so the shift never lands on a port a
        // container (or another server) already owns.
        let mut reserved: Vec<u16> = model
            .compose_files
            .iter()
            .flat_map(|c| c.services.iter().flat_map(|s| s.host_ports.iter().copied()))
            .collect();
        reserved.extend(node_backends.iter().filter_map(|b| b.port));
        reserved.sort_unstable();
        reserved.dedup();
        let backend_uses_3000 = node_backends.iter().any(|b| b.port == Some(3000));
        let frontend_port_override = (backend_uses_3000 || reserved.contains(&3000))
            && (pkg.features.next || pkg.features.nuxt)
            && pkg.port.unwrap_or(3000) == 3000;
        let override_port = if frontend_port_override {
            crate::ports::local_dev_port(3001, &reserved)
        } else {
            pkg.port.unwrap_or(3000)
        };
        let run_command = if frontend_port_override {
            format!("{} -- --port {}", run, override_port)
        } else {
            run.clone()
        };
        let step = PendingStep::run(&label, &run_command, Some(&pkg.dir), root, true);
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);

        if let Some(port) = pkg
            .port
            .map(|p| if frontend_port_override { override_port } else { p })
        {
            // Dev servers (Vite, Next.js, Expo) auto-increment their port when
            // the configured one is already taken. Accept the detected port
            // plus the next few so a busy primary port does not time out the
            // wait.
            let candidates: Vec<u16> = (port + 1..=port + 4).collect();
            let mut wait = PendingStep::wait_port_candidates(
                "127.0.0.1",
                port,
                &candidates,
                WAIT_PORT_TIMEOUT_SECS,
                &id,
            );
            wait.label = format!("Wait for {} port {}", tool, port);
            wait = wait.with_metadata("confidence", pkg.port_confidence.as_str());
            steps.push(wait);
        }
    }

    // --- 6. Makefile (best-effort default target; not a service) ---
    for makefile in &model.makefiles {
        let dir = makefile.parent().unwrap_or(root);
        let dir_label = rel_label(dir, root);
        let label = if dir_label == "root" {
            "Run Makefile default target".to_string()
        } else {
            format!("Run Makefile default target ({})", dir_label)
        };
        if !known.insert(("run".to_string(), label.clone())) {
            continue;
        }
        let mut step = PendingStep::run(&label, "make", Some(dir), root, false);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        steps.push(step);
    }

    // --- 7. Dockerfiles: never assumed to be runnable services ---
    for df in &model.dockerfiles {
        // Governed when a compose file in the same dir (or the root) covers
        // it, or when any compose service builds from the Dockerfile's dir.
        let governed_by_compose = model
            .compose_files
            .iter()
            .any(|c| c.dir == df.dir || c.dir == *root)
            || compose_governed.contains_key(&df.dir);
        if governed_by_compose {
            continue;
        }
        let dir_label = rel_label(&df.dir, root);
        let label = if dir_label == "root" {
            "Build Docker image (disabled by default)".to_string()
        } else {
            format!("Build Docker image ({}) (disabled by default)", dir_label)
        };
        let mut step =
            PendingStep::run(&label, "docker build -t app .", Some(&df.dir), root, false);
        step.enabled = false;
        step = step.with_metadata("policy", "manual-enable");
        steps.push(step);
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Warning,
            AnalysisConfidence::Medium,
            "Dockerfile found but no runnable service assumed; build step DISABLED by default"
                .to_string(),
            Some(df.path.display().to_string()),
        ));
    }

    // --- 8. Open a plain terminal at the end (tool step) ---
    if options.include_tool_steps {
        // Anchor the terminal to the project root: a bare terminal must
        // never open in the app's own working directory.
        let mut term = PendingStep::open_terminal();
        term.working_directory = Some(root.to_string_lossy().into_owned());
        steps.push(term);
    }

    // --- 9. Docker compose governance post-pass ---
    // When a compose service builds from a local project directory, the
    // container IS that service: the generated local run step is disabled
    // (the user can still enable it to develop locally) and its readiness
    // wait is re-pointed at the compose step so it never races `docker
    // compose up` while the local run is skipped.
    let mut governed_run_ids: HashMap<String, String> = HashMap::new();
    for (i, p) in steps.iter_mut().enumerate() {
        let run_id = format!("step_{:03}", i + 1);
        // The compose step itself is never "governed" — it IS the container
        // orchestration (a service building from the compose dir must not
        // disable the very step that starts it).
        if compose_ids.contains(&run_id) {
            continue;
        }
        let Some(wd) = &p.working_directory else {
            continue;
        };
        let wd_path = PathBuf::from(wd);
        let Some((compose_step_id, service_names)) = compose_governed.get(&wd_path) else {
            continue;
        };
        if !matches!(p.kind, StepKind::RunCommand { .. }) {
            continue;
        }
        governed_run_ids.insert(run_id.clone(), compose_step_id.clone());
        p.enabled = false;
        p.metadata
            .get_or_insert_with(HashMap::new)
            .insert("policy".to_string(), "docker-compose-governs".to_string());
        if !p.depends_on.iter().any(|d| d == compose_step_id) {
            p.depends_on.push(compose_step_id.clone());
        }
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Warning,
            AnalysisConfidence::High,
            format!(
                "Local run '{}' DISABLED: docker compose service(s) '{}' build from this \
                 directory; the container provides the service (enable manually to run locally)",
                p.label,
                service_names.join(", ")
            ),
            None,
        ));
    }
    // Re-point every dependent of a governed run at the compose step: port
    // waits must not race `docker compose up`, and URL steps (API docs)
    // must not open before the containerized service is reachable.
    for p in steps.iter_mut() {
        for dep in &mut p.depends_on {
            if let Some(compose_step_id) = governed_run_ids.get(dep) {
                *dep = compose_step_id.clone();
            }
        }
    }

    // Structural diagnostic about the layout.
    let layout_note = match model.layout {
        LayoutClass::RootOnly => "single-root project",
        LayoutClass::Split => "split layout (multiple sibling manifests)",
        LayoutClass::Monorepo => "monorepo (workspace root + packages)",
        LayoutClass::Nested => "nested packages",
    };
    let manifest_count = model.node_packages.len()
        + model.cargo_packages.len()
        + model.go_modules.len()
        + model.python_projects.len()
        + model.gradle_projects.len()
        + model.maven_projects.len()
        + model.dotnet_projects.len()
        + model.ruby_projects.len()
        + model.composer_projects.len();
    diagnostics.push(AnalysisDiagnostic::new(
        DiagnosticSeverity::Info,
        AnalysisConfidence::High,
        format!(
            "Project layout classified as {}; {} manifest(s) found",
            layout_note, manifest_count
        ),
        None,
    ));

    finalize_steps(steps, diagnostics)
}

/// Select a runnable Go package inside a module.  A common layout keeps the
/// executable under `cmd/<name>/main.go`; `go run .` in the module root then
/// fails with "no Go files".  Prefer the root package, then the conventional
/// `cmd` tree, while retaining the root command as a fallback for projects
/// whose entry point is generated at runtime.
fn go_run_command(module_dir: &Path) -> String {
    if module_dir.join("main.go").is_file() {
        return "go run .".to_string();
    }

    let cmd_root = module_dir.join("cmd");
    if !cmd_root.is_dir() {
        return "go run .".to_string();
    }

    if cmd_root.join("main.go").is_file() {
        return "go run ./cmd".to_string();
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&cmd_root)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("main.go")
        {
            if let Some(parent) = entry.path().parent() {
                candidates.push(parent.to_path_buf());
            }
        }
    }
    candidates.sort();
    if let Some(entry_dir) = candidates.first() {
        if let Ok(relative) = entry_dir.strip_prefix(module_dir) {
            let rel = relative.to_string_lossy().replace('\\', "/");
            return format!("go run ./{}", rel.trim_start_matches("./"));
        }
    }
    "go run .".to_string()
}

fn go_port_hint(dir: &Path) -> Option<(u16, AnalysisConfidence, String)> {
    // Highest evidence: an explicit PORT in the environment file.
    let env_re = regex::Regex::new(r"(?m)^\s*PORT\s*=\s*(\d{2,5})")
        .expect("hardcoded regex literal is valid");
    let env_path = dir.join(".env");
    if env_path.is_file() {
        if let Ok(content) = fs::read_to_string(&env_path) {
            if let Some(caps) = env_re.captures(&content) {
                if let Ok(p) = caps[1].parse::<u16>() {
                    if p > 0 {
                        return Some((p, AnalysisConfidence::High, ".env PORT".to_string()));
                    }
                }
            }
        }
    }
    // Direct listen call: `http.ListenAndServe(":8080", ...)`.
    let listen_re = regex::Regex::new(r#"ListenAndServe(TLS)?\s*\(\s*":(\d{2,5})"#)
        .expect("hardcoded regex literal is valid");
    // Fallback: any `:port` token in the entry source.
    let addr_re = regex::Regex::new(r":(\d{2,5})\b").expect("hardcoded regex literal is valid");
    for file in ["main.go", "cmd/main.go"] {
        let path = dir.join(file);
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(caps) = listen_re.captures(&content) {
                    if let Ok(p) = caps[2].parse::<u16>() {
                        if p > 0 {
                            return Some((
                                p,
                                AnalysisConfidence::High,
                                format!("{} ListenAndServe", file),
                            ));
                        }
                    }
                }
                for caps in addr_re.captures_iter(&content) {
                    if let Ok(p) = caps[1].parse::<u16>() {
                        if p > 0 && p != 80 && p != 443 {
                            return Some((
                                p,
                                AnalysisConfidence::Medium,
                                format!("{} address literal", file),
                            ));
                        }
                    }
                }
            }
        }
    }
    None
}

fn docs_url_for_node(pkg: &NodePackage) -> String {
    let port = pkg.port.unwrap_or(3000);
    if pkg.features.next {
        return format!("http://localhost:{}/api", port);
    }
    if pkg.features.api_framework {
        // nestjs / fastify default to /docs; swagger-ui-express to /api-docs
        return format!("http://localhost:{}/docs", port);
    }
    format!("http://localhost:{}/docs", port)
}

// ---------------------------------------------------------------------------
// Detection diagnostics вЂ” every inference is recorded with a confidence
// ---------------------------------------------------------------------------

/// Emit an informational diagnostic per detected manifest, so the draft
/// profile carries the evidence behind each inferred action (rule 8:
/// "record confidence and diagnostics for each inferred action").
fn emit_detection_diagnostics(model: &ProjectModel, diagnostics: &mut Vec<AnalysisDiagnostic>) {
    for pkg in &model.node_packages {
        let layout = if pkg.is_workspace_root {
            "workspace root"
        } else if pkg.dir == model.root {
            "project root"
        } else {
            "package"
        };
        let manager = pkg.package_manager.as_str();
        let mut evidence = format!(
            "Detected Node.js {} '{}' in '{}' (package manager: {}{})",
            layout,
            pkg.name,
            rel_label(&pkg.dir, &model.root),
            manager,
            if pkg.has_lockfile {
                ""
            } else {
                "; no lockfile found"
            }
        );
        if let Some(cmd) = &pkg.run_command {
            evidence.push_str(&format!("; run: {}", cmd));
        }
        if let Some(port) = pkg.port {
            if let Some(source) = &pkg.port_source {
                evidence.push_str(&format!(
                    "; port {} guessed from {} (confidence {})",
                    port,
                    source,
                    pkg.port_confidence.as_str()
                ));
            }
        }
        // The manifest file itself is direct evidence of the package; a
        // lockfile additionally pins the package manager. The port guess
        // (if any) is reported in the message text with its own confidence
        // — it must not drag the detection confidence down.
        let detection_confidence = if pkg.has_lockfile {
            AnalysisConfidence::High
        } else {
            AnalysisConfidence::Medium
        };
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            detection_confidence,
            evidence,
            Some(pkg.manifest.display().to_string()),
        ));
    }

    for cargo in &model.cargo_packages {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected Cargo package in '{}'{}",
                rel_label(&cargo.dir, &model.root),
                if cargo.has_tauri { " (Tauri)" } else { "" }
            ),
            Some(cargo.manifest.display().to_string()),
        ));
    }

    for go in &model.go_modules {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected Go module '{}' in '{}'",
                go.module,
                rel_label(&go.dir, &model.root)
            ),
            Some(go.manifest.display().to_string()),
        ));
    }

    for py in &model.python_projects {
        let kind = match py.kind {
            PythonKind::Requirements => "requirements.txt",
            PythonKind::Pyproject => "pyproject.toml",
            PythonKind::Pipfile => "Pipfile",
            PythonKind::None_ => "entry files only",
        };
        let mut evidence = format!(
            "Detected Python project in '{}' ({})",
            rel_label(&py.dir, &model.root),
            kind
        );
        if py.has_fastapi {
            evidence.push_str("; FastAPI");
        }
        if py.has_flask {
            evidence.push_str("; Flask");
        }
        if py.has_django {
            evidence.push_str("; Django");
        }
        if let Some(entry) = &py.entry {
            evidence.push_str(&format!("; entry {}", entry));
        }
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            evidence,
            None,
        ));
    }

    for gradle in &model.gradle_projects {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected Gradle project in '{}'{}",
                rel_label(&gradle.dir, &model.root),
                if gradle.is_spring {
                    " (Spring Boot)"
                } else {
                    ""
                }
            ),
            Some(gradle.manifest.display().to_string()),
        ));
    }

    for maven in &model.maven_projects {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected Maven project in '{}'{}",
                rel_label(&maven.dir, &model.root),
                if maven.is_spring {
                    " (Spring Boot)"
                } else {
                    ""
                }
            ),
            Some(maven.manifest.display().to_string()),
        ));
    }

    for dotnet in &model.dotnet_projects {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected .NET project in '{}'",
                rel_label(&dotnet.dir, &model.root)
            ),
            Some(dotnet.manifest.display().to_string()),
        ));
    }

    for ruby in &model.ruby_projects {
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            format!(
                "Detected Ruby/Rails project in '{}'",
                rel_label(&ruby.dir, &model.root)
            ),
            Some(ruby.manifest.display().to_string()),
        ));
    }

    for compose in &model.compose_files {
        let db_hint = if compose.db_ports.is_empty() {
            "no database services inferred".to_string()
        } else {
            format!(
                "{} database port(s) inferred: {}",
                compose.db_ports.len(),
                compose
                    .db_ports
                    .iter()
                    .map(|(p, s)| format!("{}:{}", s, p))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let mut message = format!(
            "Detected compose file in '{}' ({})",
            rel_label(&compose.dir, &model.root),
            db_hint
        );
        if !compose.web_tools.is_empty() {
            message.push_str(&format!(
                "; {} web tool(s) inferred: {}",
                compose.web_tools.len(),
                compose
                    .web_tools
                    .iter()
                    .map(|(p, t)| format!("{}:{}", t, p))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(w) = &compose.parse_warning {
            message.push_str(&format!("; warning: {}", w));
        }
        // A compose file parsed by a real YAML parser is direct evidence:
        // service names and host ports literally come from the file. The
        // only inference is "this service is a database/web tool", which is
        // a canonical-list match — high confidence, not a guess.
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::High,
            message,
            Some(compose.path.display().to_string()),
        ));
    }
}

// ---------------------------------------------------------------------------
// Analyzer traits
// ---------------------------------------------------------------------------

pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(
        &self,
        project_path: &str,
        vscode_path: Option<&str>,
    ) -> Result<LaunchProfile, String>;
}

pub trait ProjectAnalyzerV2: Send + Sync {
    fn analyze_draft(
        &self,
        project_path: &str,
        options: &AnalyzeOptions,
    ) -> Result<DraftProfile, String>;
}

pub struct FsProjectAnalyzer;

impl ProjectAnalyzerV2 for FsProjectAnalyzer {
    fn analyze_draft(
        &self,
        project_path: &str,
        options: &AnalyzeOptions,
    ) -> Result<DraftProfile, String> {
        let model = ProjectModel::collect(project_path, options.max_depth)?;
        let mut diagnostics: Vec<AnalysisDiagnostic> = Vec::new();
        emit_detection_diagnostics(&model, &mut diagnostics);
        let steps = generate_steps(&model, options, &mut diagnostics);

        if model.node_packages.is_empty()
            && model.cargo_packages.is_empty()
            && model.go_modules.is_empty()
            && model.python_projects.is_empty()
            && model.compose_files.is_empty()
        {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Warning,
                AnalysisConfidence::Medium,
                "No recognizable project manifests found; the draft contains only tool/IDE steps"
                    .to_string(),
                None,
            ));
        }

        let project_name = Path::new(project_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Project")
            .to_string();

        let profile = LaunchProfileV2 {
            schema_version: PROFILE_SCHEMA_VERSION.to_string(),
            id: generate_stable_id(),
            name: project_name,
            description: format!("Auto-detected draft profile for {}", project_path),
            project_root: Some(project_path.to_string()),
            steps,
            environment_binding_id: None,
            preferred_ide: None,
            default_execution_mode: None,
            extra: serde_json::Map::new(),
        };

        Ok(DraftProfile {
            profile,
            diagnostics,
        })
    }
}

impl ProjectAnalyzer for FsProjectAnalyzer {
    fn analyze(
        &self,
        project_path: &str,
        vscode_path: Option<&str>,
    ) -> Result<LaunchProfile, String> {
        let options = AnalyzeOptions {
            vscode_path: vscode_path.map(String::from),
            ..AnalyzeOptions::default()
        };
        let draft = self.analyze_draft(project_path, &options)?;
        Ok(LaunchProfile::from(draft.profile))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_dl_analyzer_{}_{}_{}",
            tag,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_tree(base: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = base.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    fn analyze(path: &Path) -> DraftProfile {
        FsProjectAnalyzer
            .analyze_draft(
                &path.to_string_lossy(),
                &AnalyzeOptions {
                    include_ide_steps: false,
                    include_tool_steps: false,
                    ..AnalyzeOptions::default()
                },
            )
            .unwrap()
    }

    const PKG_JSON: &str = r#"{
        "name": "app",
        "dependencies": { "express": "^4" },
        "scripts": { "dev": "node server.js" }
    }"#;

    #[test]
    fn root_only_project_generates_backend_step() {
        let dir = temp_dir("root_only");
        write_tree(
            &dir,
            &[
                ("package.json", PKG_JSON),
                ("server.js", "console.log('hi')"),
                (".git/HEAD", "ref: refs/heads/main"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        assert_eq!(
            profile.project_root.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        assert!(!profile.steps.is_empty());
        let run = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Node.js backend"))
            .expect("backend step");
        assert_eq!(
            run.working_directory.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        assert!(run.visibility == Some(Visibility::VisibleTerminal));
        // Wait step depends on the run step.
        let wait = profile
            .steps
            .iter()
            .find(|s| s.label.starts_with("Wait for backend port"))
            .expect("wait step");
        assert_eq!(wait.depends_on, vec![run.id.clone()]);
        // Wait steps carry a low-confidence metadata marker.
        assert_eq!(
            wait.metadata.as_ref().unwrap().get("confidence").unwrap(),
            "low"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn frontend_backend_monorepo_generates_two_servers() {
        let dir = temp_dir("split");
        write_tree(
            &dir,
            &[
                (
                    "backend/package.json",
                    r#"{"name":"api","dependencies":{"express":"^4"},"scripts":{"dev":"node index.js"}}"#,
                ),
                ("backend/index.js", "1"),
                (
                    "frontend/package.json",
                    r#"{"name":"web","dependencies":{"vite":"^5"},"scripts":{"dev":"vite --port 5173"}}"#,
                ),
                ("frontend/index.html", "<html></html>"),
                (
                    "frontend/vite.config.ts",
                    "export default { server: { port: 5173 } }",
                ),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let backend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Node.js backend"))
            .expect("backend step");
        assert_eq!(
            backend.working_directory.as_deref(),
            Some(dir.join("backend").to_string_lossy().as_ref())
        );
        let frontend = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Vite dev server"))
            .expect("frontend step");
        assert_eq!(
            frontend.working_directory.as_deref(),
            Some(dir.join("frontend").to_string_lossy().as_ref())
        );
        // The frontend wait uses the config-file port at high confidence.
        let fw = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Wait for Vite port"))
            .expect("frontend wait");
        match fw.kind {
            StepKind::WaitForPort { port, .. } => assert_eq!(port, 5173),
            _ => panic!("expected WaitForPort"),
        }
        assert_eq!(
            fw.metadata.as_ref().unwrap().get("confidence").unwrap(),
            "high"
        );
        // Distinct working directories -> no duplicate suppression.
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn expo_and_go_backend_generates_coherent_graph() {
        let dir = temp_dir("expo_go");
        write_tree(
            &dir,
            &[
                (
                    "mobile/package.json",
                    r#"{"name":"mobile","dependencies":{"expo":"^51"},"scripts":{"start":"expo start"}}"#,
                ),
                ("mobile/App.tsx", "1"),
                ("backend/go.mod", "module example.com/api\n\ngo 1.22\n"),
                ("backend/main.go", "package main\nfunc main() {}\n"),
                ("docker-compose.yml", "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5432:5432\"\n"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let labels: Vec<&str> = profile.steps.iter().map(|s| s.label.as_str()).collect();
        let has = |needle: &str| labels.iter().any(|l| l.contains(needle));
        assert!(has("Expo"), "labels: {:?}", labels);
        assert!(has("Go backend"), "labels: {:?}", labels);
        assert!(has("Start Docker Compose"), "labels: {:?}", labels);
        // The DB wait is labelled with the compose service name.
        assert!(has("postgres"), "labels: {:?}", labels);
        assert!(has("Wait for Go backend port"), "labels: {:?}", labels);

        // Dependency graph: compose -> db wait; backend after compose.
        let compose = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Docker Compose"))
            .unwrap();
        let db_wait = profile
            .steps
            .iter()
            .find(|s| s.label.contains("postgres"))
            .unwrap();
        assert!(db_wait.depends_on.contains(&compose.id));
        let go = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Go backend"))
            .unwrap();
        assert!(go.depends_on.contains(&compose.id));
        let go_wait = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Wait for Go backend port"))
            .unwrap();
        assert!(go_wait.depends_on.contains(&go.id));
        // The DB port was inferred from compose (5432).
        match db_wait.kind {
            StepKind::WaitForPort { port, .. } => assert_eq!(port, 5432),
            _ => panic!("expected WaitForPort"),
        }
        // No duplicate servers for the same project: exactly one Expo *run*
        // step (the readiness wait also mentions Expo in its label).
        let expo_runs = labels
            .iter()
            .filter(|l| l.contains("Expo") && !l.contains("port"))
            .count();
        assert_eq!(expo_runs, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_package_jsons_deduplicated_per_dir() {
        let dir = temp_dir("multi_pkg");
        write_tree(
            &dir,
            &[
                (
                    "services/api/package.json",
                    r#"{"name":"api","scripts":{"dev":"node index.js"}}"#,
                ),
                ("services/api/index.js", "1"),
                (
                    "services/worker/package.json",
                    r#"{"name":"worker","scripts":{"dev":"node worker.js"}}"#,
                ),
                ("services/worker/worker.js", "1"),
                // Same command as api but in a nested copy -- must be skipped
                // as a duplicate label (same dir is impossible; identical
                // command in a different dir is legitimate).
                (
                    "apps/web/package.json",
                    r#"{"name":"web","scripts":{"dev":"node index.js"}}"#,
                ),
                ("apps/web/index.js", "1"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let labels: Vec<&str> = profile.steps.iter().map(|s| s.label.as_str()).collect();
        let api = labels.iter().filter(|l| l.contains("Node.js")).count();
        assert_eq!(api, 3, "one step per distinct directory: {:?}", labels);
        // Working dirs are all distinct.
        let dirs: Vec<&str> = profile
            .steps
            .iter()
            .filter_map(|s| s.working_directory.as_deref())
            .collect();
        assert_eq!(dirs.len(), profile.steps.len());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_package_json_does_not_abort_analysis() {
        let dir = temp_dir("malformed");
        write_tree(
            &dir,
            &[
                ("package.json", "{ not json !!"),
                ("main.go", "package main\nfunc main() {}\n"),
                ("go.mod", "module x\n"),
            ],
        );
        let draft = analyze(&dir);
        assert!(!draft.profile.steps.is_empty());
        // Go backend still detected even though package.json is broken.
        assert!(draft
            .profile
            .steps
            .iter()
            .any(|s| s.label.contains("Go backend")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_lockfiles_detects_package_manager_unknown() {
        let dir = temp_dir("nolock");
        write_tree(&dir, &[("package.json", PKG_JSON)]);
        let model = ProjectModel::collect(&dir.to_string_lossy(), 8).unwrap();
        assert_eq!(model.node_packages.len(), 1);
        assert_eq!(
            model.node_packages[0].package_manager,
            PackageManager::Unknown
        );
        assert!(!model.node_packages[0].has_lockfile);
        let _ = fs::remove_dir_all(&dir);

        let dir2 = temp_dir("lock");
        write_tree(
            &dir2,
            &[
                ("package.json", PKG_JSON),
                ("pnpm-lock.yaml", "lockfileVersion: '9.0'"),
            ],
        );
        let model = ProjectModel::collect(&dir2.to_string_lossy(), 8).unwrap();
        assert_eq!(model.node_packages[0].package_manager, PackageManager::Pnpm);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn duplicate_detection_across_manifest_kinds() {
        let dir = temp_dir("dup");
        // Dockerfile + compose in the same dir: compose wins, no duplicate
        // compose step, and the Dockerfile step is disabled.
        write_tree(
            &dir,
            &[
                ("Dockerfile", "FROM node:20"),
                (
                    "docker-compose.yml",
                    "services:\n  web:\n    build: .\n    ports:\n      - \"3000:3000\"\n",
                ),
                ("package.json", PKG_JSON),
                ("server.js", "1"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let compose_count = profile
            .steps
            .iter()
            .filter(|s| s.label.contains("Start Docker Compose"))
            .count();
        assert_eq!(compose_count, 1);
        let docker_build = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Build Docker image"));
        // Governed by compose -> the build step is not generated at all.
        assert!(
            docker_build.is_none(),
            "docker build must be skipped when compose governs"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dockerfile_without_compose_yields_disabled_build_step() {
        let dir = temp_dir("dfonly");
        write_tree(
            &dir,
            &[
                ("Dockerfile", "FROM python:3.12"),
                ("requirements.txt", "fastapi\n"),
                ("main.py", "def app():\n    pass\n"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let build = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Build Docker image"))
            .expect("disabled build step");
        assert!(!build.enabled);
        assert_eq!(
            build.metadata.as_ref().unwrap().get("policy").unwrap(),
            "manual-enable"
        );
        // FastAPI server still generated and enabled.
        let fastapi = profile
            .steps
            .iter()
            .find(|s| s.label.contains("FastAPI"))
            .expect("fastapi step");
        assert!(fastapi.enabled);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fastapi_generates_api_docs_step_enabled() {
        let dir = temp_dir("fastapi");
        write_tree(
            &dir,
            &[
                ("requirements.txt", "fastapi\nuvicorn\n"),
                ("main.py", "from fastapi import FastAPI\napp = FastAPI()\n"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let docs = profile
            .steps
            .iter()
            .find(|s| s.label.contains("FastAPI docs"))
            .expect("docs step");
        assert!(
            docs.enabled,
            "API docs must not default to disabled when inferable"
        );
        match &docs.kind {
            StepKind::OpenUrl { url } => assert_eq!(url, "http://localhost:8000/docs"),
            other => panic!("expected OpenUrl, got {:?}", other),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nested_packages_in_custom_directories() {
        let dir = temp_dir("custom");
        write_tree(
            &dir,
            &[
                (
                    "apps/mobile/package.json",
                    r#"{"name":"m","dependencies":{"react-native":"^0.74"},"scripts":{"start":"react-native start"}}"#,
                ),
                ("apps/mobile/index.js", "1"),
                (
                    "services/api/package.json",
                    r#"{"name":"a","scripts":{"dev":"node api.js"}}"#,
                ),
                ("services/api/api.js", "1"),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        let labels: Vec<&str> = profile.steps.iter().map(|s| s.label.as_str()).collect();
        assert!(labels
            .iter()
            .any(|l| l.contains("React Native") && l.contains("apps/mobile")));
        assert!(labels
            .iter()
            .any(|l| l.contains("Node.js") && l.contains("services/api")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dependency_graph_has_no_cycles_and_wait_ordering() {
        let dir = temp_dir("graph");
        write_tree(
            &dir,
            &[
                ("package.json", PKG_JSON),
                ("server.js", "1"),
                ("go.mod", "module g\n"),
                ("main.go", "package main\n"),
                (
                    "docker-compose.yml",
                    "services:\n  redis:\n    image: redis\n    ports:\n      - \"6379:6379\"\n",
                ),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;
        // Validate the generated graph.
        let result = crate::modules::devlauncher::validation::validate_profile_v2(&profile);
        assert!(
            result.valid,
            "generated graph must validate: {:?}",
            result.diagnostics
        );
        // Wait steps never precede their server step.
        for wait in profile
            .steps
            .iter()
            .filter(|s| matches!(s.kind, StepKind::WaitForPort { .. }))
        {
            for dep in &wait.depends_on {
                let dep_step = profile.steps.iter().find(|s| &s.id == dep).unwrap();
                assert!(
                    !matches!(dep_step.kind, StepKind::WaitForPort { .. }),
                    "wait step {} depends on another wait: {}",
                    wait.id,
                    dep
                );
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_generated_step_has_explicit_working_directory_or_none_for_waits() {
        let dir = temp_dir("wd");
        write_tree(
            &dir,
            &[
                ("package.json", PKG_JSON),
                ("server.js", "1"),
                ("main.go", "package main\n"),
                ("go.mod", "module g\n"),
            ],
        );
        let draft = analyze(&dir);
        for step in &draft.profile.steps {
            match step.kind {
                // Every generated command anchors its paths to an explicit
                // project root or an explicit absolute working directory.
                StepKind::RunCommand { .. } => {
                    assert!(
                        step.working_directory.is_some(),
                        "step '{}' must have an explicit working directory",
                        step.label
                    );
                }
                _ => {}
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn node_scripts_never_misread_values() {
        let s = serde_json::json!({
            "dev": "node --watch src/index.js",
            "start": "node src/index.js"
        });
        let picked = pick_npm_run_script(Some(&s)).unwrap();
        assert!(picked == "dev" || picked == "start");

        let s = serde_json::json!({
            "build": "nuxt build",
            "generate": "nuxt generate",
            "postinstall": "nuxt prepare"
        });
        assert_eq!(pick_npm_run_script(Some(&s)), Some("build".into()));
    }

    #[test]
    fn port_detection_variants() {
        assert_eq!(detect_dev_port("npm run dev -- --port 5173"), Some(5173));
        assert_eq!(detect_dev_port("npm run dev -- --port=8080"), Some(8080));
        assert_eq!(detect_dev_port("vite -p 4000"), Some(4000));
        assert_eq!(
            detect_dev_port("cross-env PORT=5000 node index.js"),
            Some(5000)
        );
        assert_eq!(detect_dev_port("npm run dev"), None);
    }

    #[test]
    fn rel_label_uses_actual_directories() {
        let root = Path::new("/proj");
        assert_eq!(rel_label(Path::new("/proj"), root), "root");
        assert_eq!(
            rel_label(Path::new("/proj/apps/mobile"), root),
            "apps/mobile"
        );
        assert_eq!(rel_label(Path::new("/elsewhere"), root), "/elsewhere");
    }

    #[test]
    fn compose_ports_extract_db_and_web_tools() {
        let dir = temp_dir("compose_parse");
        let compose = dir.join("compose.yml");
        fs::write(
            &compose,
            "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5433:5432\"\n  redis:\n    image: redis\n    ports:\n      - \"6379:6379\"\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n",
        )
        .unwrap();
        let result = parse_compose_services(&compose);
        assert!(result.warning.is_none());
        assert_eq!(
            result.db_ports,
            vec![(5433, "postgres".to_string()), (6379, "redis".to_string())]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A compose file with dashboards (grafana, airflow) must produce
    /// browser-open steps wired to the compose start.
    #[test]
    fn compose_web_tools_generate_browser_steps() {
        let dir = temp_dir("compose_web");
        fs::write(
            dir.join("compose.yml"),
            "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5433:5432\"\n  grafana:\n    image: grafana/grafana\n    ports:\n      - \"3000:3000\"\n  airflow:\n    image: apache/airflow\n    ports:\n      - \"8080:8080\"\n",
        )
        .unwrap();
        let draft = FsProjectAnalyzer
            .analyze_draft(
                &dir.to_string_lossy(),
                &AnalyzeOptions {
                    include_tool_steps: false,
                    ..AnalyzeOptions::default()
                },
            )
            .unwrap();
        let urls: Vec<&str> = draft
            .profile
            .steps
            .iter()
            .filter_map(|s| match &s.kind {
                StepKind::OpenUrl { url } => Some(url.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            urls.iter().any(|u| u.ends_with(":3000")),
            "grafana URL missing: {:?}",
            urls
        );
        assert!(
            urls.iter().any(|u| u.ends_with(":8080")),
            "airflow URL missing: {:?}",
            urls
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_governed_services_disable_local_runs() {
        let dir = temp_dir("governed");
        write_tree(
            &dir,
            &[
                (
                    "frontend/package.json",
                    r#"{"name":"mobile","dependencies":{"expo":"^51"},"scripts":{"start":"expo start"}}"#,
                ),
                ("frontend/App.tsx", "1"),
                ("backend/go.mod", "module example.com/api\n\ngo 1.22\n"),
                (
                    "backend/main.go",
                    "package main\nfunc main() { http.ListenAndServe(\":3000\", nil) }\n",
                ),
                (
                    "docker-compose.yaml",
                    "services:\n\
                     \x20 postgres:\n\x20\x20 image: postgres:16\n\x20\x20 ports:\n\x20\x20\x20 - \"5432:5432\"\n\
                     \x20 backend:\n\x20\x20 build: ./backend\n\x20\x20 ports:\n\x20\x20\x20 - \"3000:3000\"\n\
                     \x20 grafana:\n\x20\x20 image: grafana/grafana\n\x20\x20 ports:\n\x20\x20\x20 - \"3001:3000\"\n",
                ),
            ],
        );
        let draft = analyze(&dir);
        let profile = &draft.profile;

        let go = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Go backend"))
            .expect("go backend step generated (disabled)");
        assert!(
            !go.enabled,
            "go run must be disabled when compose builds it"
        );
        assert_eq!(
            go.metadata.as_ref().unwrap().get("policy").unwrap(),
            "docker-compose-governs"
        );

        // The readiness wait must be chained to the compose step, never to
        // the (skipped) local run — it must not race `docker compose up`.
        let go_wait = profile
            .steps
            .iter()
            .find(|s| s.label.starts_with("Wait for Go backend port"))
            .expect("go wait");
        assert!(!go_wait.depends_on.contains(&go.id));
        let compose = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Start Docker Compose"))
            .unwrap();
        assert!(
            go_wait.depends_on.contains(&compose.id),
            "go wait must depend on compose, got {:?}",
            go_wait.depends_on
        );

        // The Expo frontend is not governed and stays enabled.
        let expo = profile
            .steps
            .iter()
            .find(|s| s.label.contains("Expo") && !s.label.contains("port"))
            .expect("expo run step");
        assert!(expo.enabled);

        // Web tools from compose produce browser steps at high confidence.
        let compose_diag = draft
            .diagnostics
            .iter()
            .find(|d| d.message.contains("Detected compose file"))
            .expect("compose diagnostic");
        assert_eq!(compose_diag.confidence, AnalysisConfidence::High);
        assert!(draft
            .diagnostics
            .iter()
            .any(|d| d.message.contains("Web tool 'grafana'")
                && d.confidence == AnalysisConfidence::High));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_kafka_wait_accepts_29092_candidate() {
        let dir = temp_dir("kafka29092");
        write_tree(
            &dir,
            &[
                (
                    "docker-compose.yaml",
                    "services:\n  kafka:\n    image: confluentinc/cp-kafka\n    ports:\n      - \"9092:9092\"\n",
                ),
            ],
        );
        let draft = analyze(&dir);
        let wait = draft
            .profile
            .steps
            .iter()
            .find(|s| s.label.contains("kafka"))
            .expect("kafka wait");
        match &wait.kind {
            StepKind::WaitForPort {
                port,
                candidate_ports,
                ..
            } => {
                assert_eq!(*port, 9092);
                assert!(
                    candidate_ports.contains(&29092),
                    "kafka wait must accept 29092 (KRaft templates): {:?}",
                    candidate_ports
                );
            }
            other => panic!("expected WaitForPort, got {:?}", other),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nextjs_frontend_shift_avoids_compose_grafana_port() {
        // Next.js frontend + Node backend + Grafana on host 3001: the
        // frontend is re-pinned off the backend's 3000, and the shift must
        // skip Grafana's 3001 and land on 3002.
        let dir = temp_dir("nextgraf");
        write_tree(
            &dir,
            &[
                (
                    "backend/package.json",
                    r#"{"name":"api","dependencies":{"express":"^4"},"scripts":{"dev":"node index.js"}}"#,
                ),
                ("backend/index.js", "1"),
                (
                    "frontend/package.json",
                    r#"{"name":"web","dependencies":{"next":"^14"},"scripts":{"dev":"next dev"}}"#,
                ),
                (
                    "docker-compose.yaml",
                    "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5432:5432\"\n  grafana:\n    image: grafana/grafana\n    ports:\n      - \"3001:3000\"\n",
                ),
            ],
        );
        let draft = analyze(&dir);
        let frontend = draft
            .profile
            .steps
            .iter()
            .find(|s| s.label.contains("Next.js dev server"))
            .expect("nextjs run step");
        match &frontend.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(
                    command.contains("--port 3002"),
                    "frontend must land on 3002 (3000 taken by backend, 3001 by grafana): {command}"
                );
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }
        let wait = draft
            .profile
            .steps
            .iter()
            .find(|s| s.label.contains("Wait for Next.js port"))
            .expect("nextjs wait");
        match &wait.kind {
            StepKind::WaitForPort { port, .. } => assert_eq!(*port, 3002),
            other => panic!("expected WaitForPort, got {:?}", other),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_port_hint_detects_port_from_env() {
        let dir = temp_dir("goenv");
        fs::write(dir.join(".env"), "PORT=9090\n").unwrap();
        let hint = go_port_hint(&dir).expect("hint");
        assert_eq!(hint.0, 9090);
        assert_eq!(hint.1, AnalysisConfidence::High);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_port_hint_detects_listen_and_serve() {
        let dir = temp_dir("golisten");
        fs::write(
            dir.join("main.go"),
            "package main\nimport \"net/http\"\nfunc main() { http.ListenAndServe(\":7070\", nil) }\n",
        )
        .unwrap();
        let hint = go_port_hint(&dir).expect("hint");
        assert_eq!(hint.0, 7070);
        assert_eq!(hint.1, AnalysisConfidence::High);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The daemon wait must be linked to the "Open Docker Desktop" step
    /// (so it never races the app launch) and carry the generous default
    /// timeout instead of the old 30s race.
    #[test]
    fn wait_docker_is_linked_to_docker_open_with_generous_timeout() {
        let wait = PendingStep::wait_docker(WAIT_DOCKER_TIMEOUT_SECS, Some("step_002"));
        assert_eq!(wait.timeout, Some(180));
        assert_eq!(wait.depends_on, vec!["step_002".to_string()]);
        assert!(matches!(wait.kind, StepKind::WaitForDocker {}));
        assert!(wait.enabled);

        let standalone = PendingStep::wait_docker(WAIT_DOCKER_TIMEOUT_SECS, None);
        assert!(standalone.depends_on.is_empty());
    }

    /// Port waits must carry the generous timeout AND a retry policy with
    /// backoff, so a dev server that needs a second chance on cold start
    /// is not reported as failed on the first miss.
    #[test]
    fn wait_port_carries_generous_timeout_and_retry_policy() {
        let wait = PendingStep::wait_port("127.0.0.1", 8081, WAIT_PORT_TIMEOUT_SECS, "step_001");
        assert_eq!(wait.timeout, Some(90));
        assert_eq!(wait.depends_on, vec!["step_001".to_string()]);
        let retry = wait
            .retry_policy
            .expect("wait port must carry a retry policy");
        assert_eq!(retry.max_retries, 2);
        assert_eq!(retry.delay_ms, 2000);
        assert_eq!(retry.backoff_multiplier, Some(1.5));
    }

    /// The Django chain must be venv-first and serialized: a captured install
    /// step creates the venv (only when missing) and finishes before the
    /// server starts, and the server/migrate steps always run with the venv
    /// interpreter — never a snapshot of `.venv` state taken at analysis time.
    #[test]
    fn django_steps_are_venv_first_and_serialized() {
        let dir = temp_dir("django");
        write_tree(
            &dir,
            &[
                ("manage.py", "# django"),
                ("requirements.txt", "Django>=4\n"),
                (".git/HEAD", "ref: refs/heads/main"),
            ],
        );
        let draft = analyze(&dir);
        let steps = &draft.profile.steps;

        let install = steps
            .iter()
            .find(|s| s.label.contains("Install Django dependencies"))
            .expect("install step");
        // Captured one-shot: waits for real process exit so the server below
        // can never race the venv creation.
        assert_eq!(install.visibility, Some(Visibility::Captured));
        assert_eq!(install.execution_mode, Some(ExecutionMode::OneShot));
        match &install.kind {
            StepKind::RunCommand { command, .. } => {
                if cfg!(target_os = "windows") {
                    assert!(
                        command.contains("if not exist .venv\\Scripts\\python.exe"),
                        "{command}"
                    );
                    assert!(
                        command.contains(".venv\\Scripts\\python.exe -m pip install"),
                        "{command}"
                    );
                } else {
                    assert!(command.contains("[ -x .venv/bin/python ]"), "{command}");
                    assert!(
                        command.contains(".venv/bin/python -m pip install"),
                        "{command}"
                    );
                }
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }

        let server = steps
            .iter()
            .find(|s| s.label.contains("Run Django server"))
            .expect("server step");
        // Server depends on the install step, so it starts only after the
        // venv exists.
        let install_idx = steps
            .iter()
            .position(|s| s.label.contains("Install Django dependencies"))
            .unwrap();
        let install_id = format!("step_{:03}", install_idx + 1);
        assert!(
            server.depends_on.iter().any(|d| d == &install_id),
            "server depends on install: {:?}",
            server.depends_on
        );
        match &server.kind {
            StepKind::RunCommand { command, .. } => {
                if cfg!(target_os = "windows") {
                    assert!(
                        command.starts_with(".venv\\Scripts\\python.exe "),
                        "{command}"
                    );
                } else {
                    assert!(command.starts_with("./.venv/bin/python "), "{command}");
                }
                assert!(command.ends_with("manage.py runserver"), "{command}");
            }
            other => panic!("expected RunCommand, got {:?}", other),
        }

        let migrate = steps
            .iter()
            .find(|s| s.label.contains("migrate"))
            .expect("migrate step");
        assert!(!migrate.enabled);
        match &migrate.kind {
            StepKind::RunCommand { command, .. } => {
                assert!(command.ends_with("manage.py migrate"), "{command}");
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

        let _ = fs::remove_dir_all(&dir);
    }

    /// Django with no dependency manifest must not generate an install step
    /// that would fail on a missing requirements.txt — the server step relies
    /// on the existing environment instead.
    #[test]
    fn django_without_manifest_skips_install_step() {
        let dir = temp_dir("django_nomanifest");
        write_tree(
            &dir,
            &[
                ("manage.py", "# django"),
                (".git/HEAD", "ref: refs/heads/main"),
            ],
        );
        let draft = analyze(&dir);
        assert!(!draft
            .profile
            .steps
            .iter()
            .any(|s| s.label.contains("Install Django dependencies")));
        assert!(draft
            .profile
            .steps
            .iter()
            .any(|s| s.label.contains("Run Django server")));
        let _ = fs::remove_dir_all(&dir);
    }
}
