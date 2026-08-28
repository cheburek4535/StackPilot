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
    /// Emit "open IDE" steps (only when the IDE is resolvable).
    pub include_ide_steps: bool,
    /// Emit helper/tool steps (Docker Desktop, DBeaver, empty terminal).
    pub include_tool_steps: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            max_depth: 8,
            vscode_path: None,
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
pub struct ComposeFile {
    pub path: PathBuf,
    pub dir: PathBuf,
    /// Host ports of services that look like databases, with the service
    /// name that hinted at them.
    pub db_ports: Vec<(u16, String)>,
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
                        model.python_projects.push(parse_python_project(&dir, kind));
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
                        let (db_ports, parse_warning) = parse_compose_ports(&path);
                        model.compose_files.push(ComposeFile {
                            path,
                            dir,
                            db_ports,
                            parse_warning,
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
        let re = regex::Regex::new(r"port\s*[:=]\s*(\d{2,5})").unwrap();
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
                let re = regex::Regex::new(r#""port"\s*:\s*(\d{2,5})"#).unwrap();
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
    let re = regex::Regex::new(r"(?m)^\s*PORT\s*=\s*(\d{2,5})\s*$").unwrap();
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

/// Lightweight compose port extraction. Full YAML parsing is out of scope
/// for a draft -- service names and `ports:` blocks are matched textually,
/// so the results are Low confidence and reported as diagnostics.
fn parse_compose_ports(path: &Path) -> (Vec<(u16, String)>, Option<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return (Vec::new(), Some(format!("Failed to read: {}", e))),
    };

    let mut db_ports: Vec<(u16, String)> = Vec::new();
    let mut current_service: Option<String> = None;
    let mut in_ports = false;
    let mut warning: Option<String> = None;

    let service_re = regex::Regex::new(r"^\s{2}([A-Za-z0-9][A-Za-z0-9_.-]*):\s*$").unwrap();
    let port_re = regex::Regex::new(r#"^\s*-\s*["']?(\d{2,5}):(\d{2,5})["']?\s*$"#).unwrap();
    let container_port_re = regex::Regex::new(r#"^\s*-\s*["']?(\d{2,5})["']?\s*$"#).unwrap();

    for line in content.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(caps) = service_re.captures(line) {
            current_service = Some(caps[1].to_string());
            in_ports = false;
            continue;
        }
        if line.trim() == "ports:" {
            in_ports = true;
            continue;
        }
        if !line.starts_with(' ') && line.ends_with(':') {
            in_ports = false;
        }
        if in_ports {
            if let Some(caps) = port_re.captures(line) {
                let host: u16 = caps[1].parse().unwrap_or(0);
                let container: u16 = caps[2].parse().unwrap_or(0);
                let service = current_service.clone().unwrap_or_default();
                let is_db = KNOWN_DB_SERVICES
                    .iter()
                    .any(|(name, _)| service.to_ascii_lowercase().contains(name))
                    || KNOWN_DB_SERVICES.iter().any(|(_, port)| *port == container);
                if is_db && host > 0 {
                    db_ports.push((host, service));
                }
            } else if let Some(caps) = container_port_re.captures(line) {
                // Container-only port: published on a random host port --
                // cannot be waited on. Recorded as a diagnostic hint only.
                let container: u16 = caps[1].parse().unwrap_or(0);
                if KNOWN_DB_SERVICES.iter().any(|(_, port)| *port == container) {
                    warning = Some(format!(
                        "Service '{}' publishes an anonymous host port; cannot infer a host port to wait on",
                        current_service.clone().unwrap_or_default()
                    ));
                }
            }
        }
    }

    db_ports.sort();
    db_ports.dedup();
    (db_ports, warning)
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

// ---------------------------------------------------------------------------
// IDE / application resolution
// ---------------------------------------------------------------------------

fn resolve_app(name: &str) -> Option<String> {
    crate::platform::ide::resolve_ide_executable(name)
}

/// Per-OS CLI names for auxiliary tools.
fn tool_cli_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["studio64", "Docker Desktop", "dbeaver"]
    } else if cfg!(target_os = "macos") {
        &["studio", "docker", "dbeaver"]
    } else {
        &["android-studio", "docker", "dbeaver-ce"]
    }
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
            metadata: None,
        }
    }

    fn wait_port(host: &str, port: u16, timeout: u64, depends_on: &str) -> Self {
        PendingStep {
            label: format!("Wait for port {}:{}", host, port),
            enabled: true,
            kind: StepKind::WaitForPort {
                host: host.to_string(),
                port,
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
            metadata: None,
        }
    }

    fn wait_docker(timeout: u64) -> Self {
        PendingStep {
            label: "Wait for Docker daemon".to_string(),
            enabled: true,
            kind: StepKind::WaitForDocker {},
            depends_on: Vec::new(),
            working_directory: None,
            visibility: None,
            execution_mode: None,
            completion: None,
            timeout: Some(timeout),
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
            let failure_policy = Some(StepKind::default_failure_policy(has_dependents));
            if !has_dependents && matches!(p.kind, StepKind::WaitForPort { .. }) {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::Low,
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
                retry_policy: None,
                metadata: p.metadata,
                extra: serde_json::Map::new(),
            }
        })
        .collect()
}

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
            if let Some(resolved) = resolve_app(tool_cli_names()[0]) {
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
        if let Some(resolved) = resolve_app(tool_cli_names()[1]) {
            steps.push(PendingStep::open_app(
                &resolved,
                Vec::new(),
                "Open Docker Desktop",
            ));
        } else {
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Low,
                "Docker Desktop not resolvable; the daemon wait will still apply".to_string(),
                None,
            ));
        }
        docker_wait_id = Some(format!("step_{:03}", steps.len() + 1));
        steps.push(PendingStep::wait_docker(30));

        let has_db = model.compose_files.iter().any(|c| !c.db_ports.is_empty());
        if has_db {
            if let Some(resolved) = resolve_app(tool_cli_names()[2]) {
                steps.push(PendingStep {
                    label: "Open DBeaver".to_string(),
                    enabled: true,
                    kind: StepKind::OpenApplication {
                        path: resolved,
                        args: None,
                    },
                    depends_on: vec![docker_wait_id.clone().unwrap()],
                    working_directory: None,
                    visibility: Some(Visibility::Detached),
                    execution_mode: None,
                    completion: Some(CompletionPolicy::ExternalLaunchAccepted),
                    timeout: None,
                    metadata: None,
                });
            } else {
                diagnostics.push(AnalysisDiagnostic::new(
                    DiagnosticSeverity::Info,
                    AnalysisConfidence::Low,
                    "DBeaver not found; database client step omitted".to_string(),
                    None,
                ));
            }
        }
    }

    // --- 3. Docker compose + database readiness (unconditional) ---
    let mut compose_ids: Vec<String> = Vec::new();
    for compose in &model.compose_files {
        let label = format!("Start Docker Compose ({})", rel_label(&compose.dir, root));
        if !known.insert(("compose".to_string(), compose.dir.display().to_string())) {
            continue;
        }
        let mut step = PendingStep::run(
            &label,
            "docker compose up -d",
            Some(&compose.dir),
            root,
            false,
        );
        if let Some(w) = &docker_wait_id {
            step.depends_on.push(w.clone());
        }
        let compose_id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        compose_ids.push(compose_id.clone());

        for (port, service) in &compose.db_ports {
            let key = format!("{}:{}", compose.dir.display(), port);
            if !known.insert(("db_wait".to_string(), key)) {
                continue;
            }
            let mut wait = PendingStep::wait_port("127.0.0.1", *port, 60, &compose_id);
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
                .with_metadata("confidence", "low")
                .with_metadata("source", "docker-compose ports");
            steps.push(wait);
            diagnostics.push(AnalysisDiagnostic::new(
                DiagnosticSeverity::Info,
                AnalysisConfidence::Low,
                format!(
                    "Guessed database port {} from compose service '{}'",
                    port, service
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
        let mut step = PendingStep::run(&label, "go run .", Some(&go.dir), root, true);
        if let Some(cid) = &compose_id {
            step.depends_on.push(cid.clone());
        }
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);
        // Go ports are almost never declared in the manifest -- low confidence.
        let port = go_port_hint(&go.dir).unwrap_or(8080);
        let mut wait = PendingStep::wait_port("127.0.0.1", port, 60, &id);
        wait.label = format!("Wait for Go backend port {}", port);
        wait = wait
            .with_metadata("confidence", "low")
            .with_metadata("source", "default 8080");
        steps.push(wait);
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            AnalysisConfidence::Low,
            format!(
                "Go backend port {} is guessed (no explicit configuration found)",
                port
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
            let mut wait = PendingStep::wait_port("127.0.0.1", port, 60, &id);
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
            let mut step = PendingStep::run(
                &label,
                "python manage.py runserver",
                Some(&py.dir),
                root,
                true,
            );
            if let Some(cid) = &compose_id {
                step.depends_on.push(cid.clone());
            }
            let id = format!("step_{:03}", steps.len() + 1);
            steps.push(step);
            let mut wait = PendingStep::wait_port("127.0.0.1", 8000, 60, &id);
            wait.label = "Wait for Django port 8000".to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
            // DB migration is state-changing -- never enabled by default.
            let mut migrate = PendingStep::run(
                "Run Django migrate (disabled by default)",
                "python manage.py migrate",
                Some(&py.dir),
                root,
                false,
            );
            migrate.enabled = false;
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
            let mut wait = PendingStep::wait_port("127.0.0.1", port, 60, &id);
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
            "./gradlew bootRun"
        } else {
            "./gradlew run"
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
            let mut wait = PendingStep::wait_port("127.0.0.1", 8080, 60, &id);
            wait.label = "Wait for Spring Boot port 8080".to_string();
            wait = wait.with_metadata("confidence", "low");
            steps.push(wait);
        }
    }

    for maven in &model.maven_projects {
        let dir_label = rel_label(&maven.dir, root);
        let cmd = if maven.is_spring {
            "mvn spring-boot:run"
        } else {
            "mvn exec:java"
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
            let mut wait = PendingStep::wait_port("127.0.0.1", 8080, 60, &id);
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
        steps.push(step);
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
        let mut wait = PendingStep::wait_port("127.0.0.1", 3000, 60, &id);
        wait.label = "Wait for Rails port 3000".to_string();
        wait = wait.with_metadata("confidence", "low");
        steps.push(wait);
    }

    // --- 5. Node packages: backend first, then frontend, deduplicated ---
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
            let mut wait = PendingStep::wait_port("127.0.0.1", port, 60, &id);
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
        let step = PendingStep::run(&label, run, Some(&pkg.dir), root, true);
        let id = format!("step_{:03}", steps.len() + 1);
        steps.push(step);

        if let Some(port) = pkg.port {
            let mut wait = PendingStep::wait_port("127.0.0.1", port, 60, &id);
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
        let governed_by_compose = model
            .compose_files
            .iter()
            .any(|c| c.dir == df.dir || c.dir == *root);
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
        steps.push(PendingStep::open_terminal());
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
        + model.ruby_projects.len();
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

fn go_port_hint(dir: &Path) -> Option<u16> {
    let env_re = regex::Regex::new(r"(?m)\bPORT\s*=\s*(\d{2,5})").unwrap();
    let addr_re = regex::Regex::new(r":(\d{2,5})\b").unwrap();
    for file in [".env", "main.go", "cmd/main.go"] {
        let path = dir.join(file);
        if path.is_file() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(caps) = env_re.captures(&content) {
                    if let Ok(p) = caps[1].parse::<u16>() {
                        if p > 0 {
                            return Some(p);
                        }
                    }
                }
                for caps in addr_re.captures_iter(&content) {
                    if let Ok(p) = caps[1].parse::<u16>() {
                        if p > 0 && p != 80 && p != 443 {
                            return Some(p);
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
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            pkg.port_confidence,
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
        if let Some(w) = &compose.parse_warning {
            message.push_str(&format!("; warning: {}", w));
        }
        diagnostics.push(AnalysisDiagnostic::new(
            DiagnosticSeverity::Info,
            if compose.db_ports.is_empty() {
                AnalysisConfidence::Medium
            } else {
                AnalysisConfidence::Low
            },
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
        assert!(has("database"), "labels: {:?}", labels);
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
            .find(|s| s.label.contains("database"))
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
        // No duplicate servers for the same project.
        let expo_count = labels.iter().filter(|l| l.contains("Expo")).count();
        assert_eq!(expo_count, 1);
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
    fn compose_port_extraction_finds_db_ports() {
        let dir = temp_dir("compose_parse");
        let compose = dir.join("compose.yml");
        fs::write(
            &compose,
            "services:\n  postgres:\n    image: postgres:16\n    ports:\n      - \"5433:5432\"\n  redis:\n    image: redis\n    ports:\n      - \"6379:6379\"\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n",
        )
        .unwrap();
        let (ports, warning) = parse_compose_ports(&compose);
        assert!(warning.is_none());
        assert_eq!(
            ports,
            vec![(5433, "postgres".to_string()), (6379, "redis".to_string())]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_port_hint_detects_port_from_env() {
        let dir = temp_dir("goenv");
        fs::write(dir.join(".env"), "PORT=9090\n").unwrap();
        assert_eq!(go_port_hint(&dir), Some(9090));
        let _ = fs::remove_dir_all(&dir);
    }
}
