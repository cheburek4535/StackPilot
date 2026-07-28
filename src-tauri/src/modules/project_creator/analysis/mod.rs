use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use crate::modules::project_creator::models::*;

const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "target", ".venv", "__pycache__",
    ".next", "dist", "build", ".idea", ".vscode", "vendor",
];

pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(&self, path: &Path) -> Result<AnalysisReport, String>;
}

pub struct DefaultProjectAnalyzer;

impl DefaultProjectAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl ProjectAnalyzer for DefaultProjectAnalyzer {
    fn analyze(&self, path: &Path) -> Result<AnalysisReport, String> {
        if !path.exists() {
            return Err(format!("Path does not exist: {}", path.display()));
        }
        if !path.is_dir() {
            return Err(format!("Not a directory: {}", path.display()));
        }

        let mut techs: Vec<DetectedTechnology> = Vec::new();
        let mut configs: Vec<String> = Vec::new();
        let mut has_docker = false;
        let mut has_git = false;
        let mut has_ci = false;
        let mut has_tests = false;
        let mut has_readme = false;
        let mut has_license = false;
        let mut project_type_hints: Vec<String> = Vec::new();

        let mut package_json_content: Option<String> = None;
        let mut cargo_toml_content: Option<String> = None;
        let mut pyproject_toml_content: Option<String> = None;

        let walker = WalkDir::new(path)
            .follow_links(false)
            .max_depth(4)
            .into_iter();

        for entry_result in walker {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let p = entry.path();
            let filename = p.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if entry.file_type().is_dir() {
                if SKIP_DIRS.contains(&filename) {
                    continue;
                }
                let rel = p.strip_prefix(path).unwrap_or(p);
                let rel_str = rel.to_string_lossy();

                if filename == ".github" {
                    has_ci = true;
                    configs.push(rel_str.to_string());
                }
                if filename == ".vscode" {
                    configs.push(rel_str.to_string());
                }
                if filename == "tests" || filename == "test" || filename == "spec" {
                    has_tests = true;
                }
                continue;
            }

            let rel = p.strip_prefix(path).unwrap_or(p);
            let rel_str = rel.to_string_lossy().to_string();
            configs.push(rel_str.clone());

            match filename {
                "package.json" => {
                    package_json_content = fs::read_to_string(p).ok();
                }
                "Cargo.toml" => {
                    cargo_toml_content = fs::read_to_string(p).ok();
                }
                "pyproject.toml" => {
                    pyproject_toml_content = fs::read_to_string(p).ok();
                }
                "Dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => {
                    has_docker = true;
                }
                ".gitignore" | ".gitattributes" | "HEAD" if rel_str.starts_with(".git") => {
                    has_git = true;
                }
                _ => {}
            }

            // CI config files
            if matches!(filename, ".github" | ".gitlab-ci.yml" | "Jenkinsfile") {
                has_ci = true;
            }
        }

        // --- Language detection by extension ---
        let mut ext_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for entry_result in WalkDir::new(path).follow_links(false).max_depth(5).into_iter() {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if matches!(
                    ext_lower.as_str(),
                    "rs" | "py" | "ts" | "tsx" | "js" | "jsx"
                        | "go" | "java" | "kt" | "swift" | "rb"
                        | "c" | "h" | "cpp" | "hpp" | "cs" | "zig"
                        | "svelte" | "vue"
                ) {
                    *ext_counts.entry(ext_lower).or_default() += 1;
                }
            }
        }

        let lang_map: Vec<(&str, &str)> = vec![
            ("rs", "Rust"),
            ("py", "Python"),
            ("ts", "TypeScript"),
            ("tsx", "TypeScript"),
            ("js", "JavaScript"),
            ("jsx", "JavaScript"),
            ("go", "Go"),
            ("java", "Java"),
            ("kt", "Kotlin"),
            ("swift", "Swift"),
            ("rb", "Ruby"),
            ("cs", "C#"),
            ("c", "C"),
            ("h", "C"),
            ("cpp", "C++"),
            ("hpp", "C++"),
            ("zig", "Zig"),
            ("svelte", "Svelte"),
            ("vue", "Vue"),
        ];

        let mut detected_langs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (ext, lang) in &lang_map {
            if let Some(&count) = ext_counts.get(*ext) {
                if count > 0 {
                    detected_langs.insert(lang.to_string());
                    techs.push(DetectedTechnology {
                        name: lang.to_string(),
                        version: None,
                        confidence: if *ext == "rs" || *ext == "go" { DetectionConfidence::Certain }
                            else { DetectionConfidence::Likely },
                        evidence: vec![format!("Found {} .{} files", count, ext)],
                    });
                }
            }
        }

        // --- Framework detection ---
        if let Some(content) = &package_json_content {
            let val: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
            let deps = val.get("dependencies");
            let dev_deps = val.get("devDependencies");
            let has_dep = |pkg: &str| {
                deps.and_then(|d| d.get(pkg)).is_some()
                    || dev_deps.and_then(|d| d.get(pkg)).is_some()
            };

            let framework_map: Vec<(&str, &[&str])> = vec![
                ("React", &["react"]),
                ("Next.js", &["next"]),
                ("Vue", &["vue"]),
                ("Nuxt", &["nuxt"]),
                ("Svelte", &["svelte"]),
                ("SvelteKit", &["@sveltejs/kit"]),
                ("Angular", &["@angular/core"]),
                ("NestJS", &["@nestjs/core"]),
                ("Express", &["express"]),
                ("Fastify", &["fastify"]),
                ("Astro", &["astro"]),
                ("Solid.js", &["solid-js"]),
                ("Qwik", &["@builder.io/qwik"]),
                ("Gatsby", &["gatsby"]),
                ("Remix", &["@remix-run/react"]),
            ];

            for (fw, packages) in &framework_map {
                if packages.iter().any(|p| has_dep(p)) {
                    let pkg_ver = packages.iter().find_map(|p| {
                        deps.and_then(|d| d.get(p))
                            .or_else(|| dev_deps.and_then(|d| d.get(p)))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    });
                    techs.push(DetectedTechnology {
                        name: fw.to_string(),
                        version: pkg_ver,
                        confidence: DetectionConfidence::Certain,
                        evidence: vec![format!("Found in package.json: {}", packages.iter().find(|p| has_dep(p)).unwrap())],
                    });
                    project_type_hints.push(fw.to_string());
                }
            }

            if has_dep("expo") {
                project_type_hints.push("Mobile (Expo)".to_string());
            }
        }

        if let Some(content) = &cargo_toml_content {
            let val = match toml::from_str::<toml::Value>(content) {
                Ok(v) => v,
                Err(_) => return Err("Failed to parse Cargo.toml".to_string()),
            };
            let deps = val.get("dependencies");
            let build_deps = val.get("build-dependencies");

            let has_crate = |name: &str| {
                deps.and_then(|d| d.get(name)).is_some()
                    || build_deps.and_then(|d| d.get(name)).is_some()
            };

            let crate_map: Vec<(&str, &[&str])> = vec![
                ("Tauri", &["tauri"]),
                ("Actix Web", &["actix-web"]),
                ("Axum", &["axum"]),
                ("Rocket", &["rocket"]),
                ("Warp", &["warp"]),
                ("Yew", &["yew"]),
                ("Dioxus", &["dioxus"]),
                ("Leptos", &["leptos"]),
                ("Tokio", &["tokio"]),
                ("Serde", &["serde"]),
            ];

            for (name, crates) in &crate_map {
                if crates.iter().any(|c| has_crate(c)) {
                    let ver = crates.iter().find_map(|c| {
                        deps.and_then(|d| d.get(c))
                            .or_else(|| build_deps.and_then(|d| d.get(c)))
                            .and_then(|v| {
                                if let Some(table) = v.as_table() {
                                    table.get("version").and_then(|v2| v2.as_str())
                                } else {
                                    v.as_str()
                                }
                            })
                            .map(|s| s.to_string())
                    });
                    techs.push(DetectedTechnology {
                        name: name.to_string(),
                        version: ver,
                        confidence: DetectionConfidence::Certain,
                        evidence: vec![format!("Found in Cargo.toml: {}", crates.iter().find(|c| has_crate(c)).unwrap())],
                    });
                    project_type_hints.push(name.to_string());
                }
            }
        }

        if let Some(content) = &pyproject_toml_content {
            if let Ok(val) = toml::from_str::<toml::Value>(content) {
                let proj_deps = val.get("project").and_then(|p| p.get("dependencies"));
                let dep_strings: Vec<String> = proj_deps
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| {
                                s.split(|c: char| c == '>' || c == '<' || c == '=' || c == '!' || c == '~')
                                    .next()
                                    .unwrap_or(s)
                                    .trim()
                                    .to_lowercase()
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let py_fw: Vec<(&str, Vec<&str>)> = vec![
                    ("FastAPI", vec!["fastapi"]),
                    ("Django", vec!["django"]),
                    ("Flask", vec!["flask"]),
                    ("SQLAlchemy", vec!["sqlalchemy"]),
                    ("Pydantic", vec!["pydantic"]),
                    ("Alembic", vec!["alembic"]),
                    ("Celery", vec!["celery"]),
                ];

                for (name, pkgs) in &py_fw {
                    if pkgs.iter().any(|p| dep_strings.contains(&p.to_string())) {
                        techs.push(DetectedTechnology {
                            name: name.to_string(),
                            version: None,
                            confidence: DetectionConfidence::Certain,
                            evidence: vec![format!("Found in pyproject.toml dependencies")],
                        });
                        project_type_hints.push(name.to_string());
                    }
                }
            }
        }

        // --- Generic docker/git/tests/readme/license detection ---
        let root = path;
        if root.join(".git").exists() {
            has_git = true;
        }
        if root.join("Dockerfile").exists() || root.join("docker-compose.yml").exists() {
            has_docker = true;
        }
        if root.join("README.md").exists() || root.join("README").exists() {
            has_readme = true;
        }
        if root.join("LICENSE").exists() || root.join("LICENSE.md").exists() {
            has_license = true;
        }
        if root.join(".github").join("workflows").exists()
            || root.join(".gitlab-ci.yml").exists()
            || root.join("Jenkinsfile").exists()
        {
            has_ci = true;
        }

        // Derive project type hints from language dominance
        if detected_langs.contains("Rust") {
            project_type_hints.push("Rust".to_string());
        }
        if detected_langs.contains("Go") {
            project_type_hints.push("Go".to_string());
        }
        if detected_langs.contains("Python") {
            project_type_hints.push("Python".to_string());
        }
        if detected_langs.contains("TypeScript") || detected_langs.contains("JavaScript") {
            if !project_type_hints.iter().any(|h| h.contains("React") || h.contains("Vue") || h.contains("Svelte") || h.contains("Angular")) {
                project_type_hints.push("Node.js".to_string());
            }
        }

        // Deduplicate
        project_type_hints.sort();
        project_type_hints.dedup();

        let summary = if techs.is_empty() {
            format!(
                "No known technologies detected in {}",
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            )
        } else {
            let langs: Vec<&str> = detected_langs.iter().map(|s| s.as_str()).collect();
            let frameworks: Vec<&str> = techs.iter()
                .filter(|t| !detected_langs.contains(&t.name))
                .map(|t| t.name.as_str())
                .collect();
            format!(
                "Detected {} in {} with {}",
                langs.join(", "),
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                if frameworks.is_empty() { "no framework detected".to_string() } else { frameworks.join(", ") }
            )
        };

        Ok(AnalysisReport {
            project_path: path.to_path_buf(),
            detected_technologies: techs,
            existing_configs: configs,
            missing_configs: Vec::new(),
            has_docker,
            has_git,
            has_ci,
            has_tests,
            has_readme,
            has_license,
            project_type_hints,
            summary,
        })
    }
}
