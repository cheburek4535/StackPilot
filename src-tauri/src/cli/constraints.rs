use std::path::{Path, PathBuf};
use crate::modules::project_creator::models::{AlternativePolicy, FrameworkDef, ToolDef, WizardTreeData};

/// Information about a resolved project destination directory.
#[derive(Debug, Clone)]
pub struct ResolvedDestination {
    pub dest_path: PathBuf,
    pub project_name: String,
    pub is_current_dir: bool,
    pub is_new_dir: bool,
    pub is_empty: bool,
    pub display_preview: String,
}

/// Resolves the destination directory and project name cleanly.
///
/// Rules:
/// 1. If `raw_input` is "." or "./", the target is the current working directory.
///    The project name defaults to `explicit_name` if given, or the name of the current directory.
/// 2. If `raw_input` is a relative path (e.g. "./my-app" or "my-app"), it is resolved
///    relative to current working directory. The project name defaults to `explicit_name`
///    or the last path segment.
/// 3. If `raw_input` is None and `explicit_name` is Some("app"), the destination defaults
///    to `./app` in the current working directory.
/// 4. If neither is given, the destination defaults to `./my-project`.
pub fn resolve_destination(
    raw_input: Option<&str>,
    explicit_name: Option<&str>,
) -> ResolvedDestination {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let (dest_path, is_current_dir, derived_name) = match raw_input.map(str::trim).filter(|s| !s.is_empty()) {
        Some(".") | Some("./") => {
            let name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "my-project".to_string());
            (cwd.clone(), true, name)
        }
        Some(p) => {
            let path = PathBuf::from(p);
            let full_path = if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(&path)
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "my-project".to_string());
            (full_path, false, name)
        }
        None => {
            if let Some(name) = explicit_name.map(str::trim).filter(|s| !s.is_empty()) {
                (cwd.join(name), false, name.to_string())
            } else {
                (cwd.join("my-project"), false, "my-project".to_string())
            }
        }
    };

    let project_name = explicit_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or(derived_name);

    let is_new_dir = !dest_path.exists();
    let is_empty = if is_new_dir {
        true
    } else {
        match std::fs::read_dir(&dest_path) {
            Ok(mut entries) => entries.next().is_none(),
            Err(_) => false,
        }
    };

    let display_preview = if is_current_dir {
        format!("{} (current directory)", dest_path.display())
    } else if is_new_dir {
        format!("{} (new folder will be created)", dest_path.display())
    } else if is_empty {
        format!("{} (existing empty folder)", dest_path.display())
    } else {
        format!("{} (existing folder, NOT empty)", dest_path.display())
    };

    ResolvedDestination {
        dest_path,
        project_name,
        is_current_dir,
        is_new_dir,
        is_empty,
        display_preview,
    }
}

/// Checks whether a directory exists and is not empty.
pub fn is_dir_non_empty(path: &Path) -> bool {
    if !path.exists() || !path.is_dir() {
        return false;
    }
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => false,
    }
}

/// Checks if a framework provides its own built-in ORM.
pub fn framework_provides_orm(fw: &FrameworkDef) -> bool {
    fw.provides_capabilities.iter().any(|c| c == "orm")
        || fw.id == "django"
        || fw.id == "rails"
        || fw.id == "adonisjs"
}

/// Checks if a framework provides its own built-in database migrations.
pub fn framework_provides_migrations(fw: &FrameworkDef) -> bool {
    fw.provides_capabilities.iter().any(|c| c == "migrations")
        || fw.id == "django"
        || fw.id == "rails"
}

/// Checks if a tool serves as an ORM.
pub fn is_orm_tool(tool: &ToolDef) -> bool {
    tool.responsibility.as_deref() == Some("orm")
        || tool.id == "sqlalchemy"
        || tool.id == "prisma"
        || tool.id == "drizzle"
}

/// Checks if a tool serves as a database migration system.
pub fn is_migration_tool(tool: &ToolDef) -> bool {
    tool.responsibility.as_deref() == Some("migrations") || tool.id == "alembic"
}

/// Checks if a tool serves as a primary database engine.
pub fn is_database_tool(tool: &ToolDef) -> bool {
    tool.responsibility.as_deref() == Some("database")
        || matches!(
            tool.id.as_str(),
            "postgresql" | "mysql" | "sqlite" | "mongodb" | "clickhouse"
        )
}

/// Checks if a tool requires Docker to run.
pub fn tool_requires_docker(tree: &WizardTreeData, tool_id: &str) -> bool {
    tree.tools
        .iter()
        .find(|t| t.id == tool_id)
        .map(|t| t.requires_docker)
        .unwrap_or(false)
}

/// Filters available frameworks for a frontend or backend selection step,
/// ensuring that platform, project type, and mutual conflicts are respected.
pub fn filter_available_frameworks<'a>(
    tree: &'a WizardTreeData,
    side: &str, // "backend" or "frontend"
    project_type: Option<&str>,
    os: &str,
    already_selected_fws: &[String],
) -> Vec<&'a FrameworkDef> {
    tree.frameworks
        .iter()
        .filter(|fw| {
            // Side matching
            let side_match = if side == "backend" {
                fw.side == "backend" || fw.side == "either"
            } else {
                fw.side == "frontend" || fw.side == "either"
            };
            if !side_match {
                return false;
            }

            // Platform check
            if !fw.platforms.is_empty() && !fw.platforms.iter().any(|p| p == os) {
                return false;
            }

            // Project type check
            if let Some(pt) = project_type {
                if !fw.project_types.is_empty() && !fw.project_types.iter().any(|t| t == pt) {
                    return false;
                }
            }

            // Mutual conflicts with already selected frameworks
            for sel_id in already_selected_fws {
                if let Some(sel) = tree.frameworks.iter().find(|f| &f.id == sel_id) {
                    if fw.conflicts.contains(sel_id) || sel.conflicts.contains(&fw.id) {
                        return false;
                    }
                    // Prevent two "either" frameworks or multiple incompatible fullstack frameworks
                    if fw.side == "either" && sel.side == "either" && fw.id != sel.id {
                        return false;
                    }
                }
            }

            true
        })
        .collect()
}

/// Filters available tools for interactive selection based on the current stack:
/// - Languages must match (tools for Python only shown if Python is selected, etc.)
/// - Project type must match (if tool restricts project types)
/// - Frameworks must match (if tool specifies `for_frameworks`)
/// - Framework conflicts: if framework defines `tool_conflicts`, tool is excluded
/// - Dynamic exclusions:
///     * If framework has built-in ORM (e.g. Django, Rails), external ORMs (SQLAlchemy, Prisma, Drizzle) are excluded
///     * If framework has built-in migrations (e.g. Django, Rails), external migrations (Alembic) are excluded
/// - Tool internal exclusions: "npm" is hidden (managed automatically)
pub fn filter_compatible_tools<'a>(
    tree: &'a WizardTreeData,
    selected_frameworks: &[String],
    selected_languages: &[String],
    project_type: Option<&str>,
    os: &str,
) -> Vec<&'a ToolDef> {
    let fws: Vec<&FrameworkDef> = selected_frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();

    let has_builtin_orm = fws.iter().any(|f| framework_provides_orm(f));
    let has_builtin_migrations = fws.iter().any(|f| framework_provides_migrations(f));

    tree.tools
        .iter()
        .filter(|t| {
            // Hide npm from manual selection (handled via toolchain)
            if t.id == "npm" {
                return false;
            }

            // Platform check
            if !t.platforms.is_empty() && !t.platforms.iter().any(|p| p == os) {
                return false;
            }

            // Language compatibility
            if !t.for_languages.is_empty() {
                let matches_lang = t.for_languages.iter().any(|tl| selected_languages.contains(tl));
                if !matches_lang {
                    return false;
                }
            }

            // Project type compatibility
            if let Some(pt) = project_type {
                if !t.for_project_types.is_empty() && !t.for_project_types.iter().any(|tpt| tpt == pt) {
                    return false;
                }
            }

            // Framework restrictions (tool.for_frameworks)
            if !t.for_frameworks.is_empty() {
                let matches_fw = selected_frameworks.iter().any(|sf| t.for_frameworks.contains(sf));
                if !matches_fw {
                    return false;
                }
            }

            // Framework hard tool_conflicts
            for fw in &fws {
                if fw.tool_conflicts.contains(&t.id) {
                    return false;
                }
            }

            // Dynamic exclusion: Framework provides built-in ORM -> hide other ORM tools
            if has_builtin_orm && is_orm_tool(t) {
                return false;
            }

            // Dynamic exclusion: Framework provides built-in migrations -> hide migration tools
            if has_builtin_migrations && is_migration_tool(t) {
                return false;
            }

            true
        })
        .collect()
}

/// Result of auto-resolving tool dependencies and Docker requirements.
#[derive(Debug, Clone)]
pub struct ToolResolutionResult {
    pub resolved_tools: Vec<String>,
    pub added_dependencies: Vec<(String, String)>, // (added_tool_label, required_by_label)
    pub docker_enabled: bool,
    pub docker_reasons: Vec<String>, // names of tools that required Docker
}

/// Automatically resolves tool dependencies (e.g. Alembic -> SQLAlchemy)
/// and detects Docker requirements (e.g. PostgreSQL, Redis -> Docker).
pub fn resolve_tool_stack(
    tree: &WizardTreeData,
    selected_tools: &[String],
    explicit_docker: bool,
) -> ToolResolutionResult {
    let mut resolved_tools = selected_tools.to_vec();
    let mut added_dependencies = Vec::new();
    let mut docker_reasons = Vec::new();

    // 1. Resolve missing dependencies (tool.requires)
    for tool_id in selected_tools {
        if let Some(tool) = tree.tools.iter().find(|t| &t.id == tool_id) {
            for req_id in &tool.requires {
                if !resolved_tools.contains(req_id) {
                    resolved_tools.push(req_id.clone());
                    let req_label = tree
                        .tools
                        .iter()
                        .find(|t| &t.id == req_id)
                        .map(|t| t.label.clone())
                        .unwrap_or_else(|| req_id.clone());
                    added_dependencies.push((req_label, tool.label.clone()));
                }
            }
        }
    }

    // 2. Check docker requirement
    for tool_id in &resolved_tools {
        if let Some(tool) = tree.tools.iter().find(|t| &t.id == tool_id) {
            if tool.requires_docker {
                docker_reasons.push(tool.label.clone());
            }
        }
    }

    let docker_enabled = explicit_docker || !docker_reasons.is_empty() || resolved_tools.contains(&"docker".to_string());

    if docker_enabled && !resolved_tools.contains(&"docker".to_string()) {
        resolved_tools.push("docker".to_string());
    }

    ToolResolutionResult {
        resolved_tools,
        added_dependencies,
        docker_enabled,
        docker_reasons,
    }
}

/// Detects mutual exclusions between tools in the given selection.
/// Returns a list of conflicting pairs (tool_a_id, tool_b_id, reason).
pub fn detect_tool_conflicts(
    tree: &WizardTreeData,
    tools: &[String],
) -> Vec<(String, String, String)> {
    let mut conflicts = Vec::new();
    let selected_defs: Vec<&ToolDef> = tools
        .iter()
        .filter_map(|id| tree.tools.iter().find(|t| &t.id == id))
        .collect();

    // 1. Check direct tool.conflicts
    for (i, a) in selected_defs.iter().enumerate() {
        for b in selected_defs.iter().skip(i + 1) {
            if a.conflicts.contains(&b.id) || b.conflicts.contains(&a.id) {
                conflicts.push((
                    a.id.clone(),
                    b.id.clone(),
                    format!("«{}» and «{}» are directly incompatible.", a.label, b.label),
                ));
            }
        }
    }

    // 2. Check exclusive responsibility overlap (e.g. postgresql vs mysql, prisma vs drizzle, gradle vs maven)
    let mut by_resp: std::collections::HashMap<&str, Vec<&ToolDef>> = std::collections::HashMap::new();
    for tool in &selected_defs {
        if let Some(resp) = &tool.responsibility {
            by_resp.entry(resp.as_str()).or_default().push(tool);
        }
    }

    for (resp, group) in by_resp {
        if group.len() <= 1 {
            continue;
        }
        for (i, a) in group.iter().enumerate() {
            for b in group.iter().skip(i + 1) {
                let is_exclusive = a.alternative_policy == AlternativePolicy::Exclusive
                    || b.alternative_policy == AlternativePolicy::Exclusive;
                if is_exclusive {
                    conflicts.push((
                        a.id.clone(),
                        b.id.clone(),
                        format!(
                            "«{}» and «{}» are mutually exclusive (both serve as '{}').",
                            a.label, b.label, resp
                        ),
                    ));
                }
            }
        }
    }

    conflicts
}

/// Strict validation of technology stack for CLI execution.
/// Catches mutual exclusions, framework overlaps, and missing prerequisites.
pub fn validate_strict_stack(
    tree: &WizardTreeData,
    frameworks: &[String],
    tools: &[String],
    languages: &[String],
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    let fws: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();

    // 1. Framework built-in capabilities vs tools
    for fw in &fws {
        if framework_provides_orm(fw) {
            for tool_id in tools {
                if let Some(tool) = tree.tools.iter().find(|t| &t.id == tool_id) {
                    if is_orm_tool(tool) {
                        errors.push(format!(
                            "Tool '{}' cannot be used with '{}': {} already includes a built-in ORM.",
                            tool.label, fw.label, fw.label
                        ));
                    }
                }
            }
        }
        if framework_provides_migrations(fw) {
            for tool_id in tools {
                if let Some(tool) = tree.tools.iter().find(|t| &t.id == tool_id) {
                    if is_migration_tool(tool) {
                        errors.push(format!(
                            "Tool '{}' cannot be used with '{}': {} already includes built-in migrations.",
                            tool.label, fw.label, fw.label
                        ));
                    }
                }
            }
        }

        // Framework hard tool conflicts
        for tool_id in tools {
            if fw.tool_conflicts.contains(tool_id) {
                let tool_label = tree
                    .tools
                    .iter()
                    .find(|t| &t.id == tool_id)
                    .map(|t| t.label.as_str())
                    .unwrap_or(tool_id);
                errors.push(format!(
                    "Framework '{}' is incompatible with tool '{}'.",
                    fw.label, tool_label
                ));
            }
        }
    }

    // 2. Tool mutual exclusions
    for (_, _, reason) in detect_tool_conflicts(tree, tools) {
        errors.push(reason);
    }

    // 3. Language compatibility
    for tool_id in tools {
        if let Some(tool) = tree.tools.iter().find(|t| &t.id == tool_id) {
            if !tool.for_languages.is_empty() {
                let ok = tool.for_languages.iter().any(|tl| languages.contains(tl));
                if !ok {
                    errors.push(format!(
                        "Tool '{}' requires one of [{}], but the project uses [{}].",
                        tool.label,
                        tool.for_languages.join(", "),
                        languages.join(", ")
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::project_creator::wizard::WizardEngine;

    #[test]
    fn test_destination_resolution_current_dir() {
        let dest = resolve_destination(Some("."), Some("my-app"));
        assert!(dest.is_current_dir);
        assert_eq!(dest.project_name, "my-app");
    }

    #[test]
    fn test_destination_resolution_subfolder() {
        let dest = resolve_destination(Some("my-app"), None);
        assert!(!dest.is_current_dir);
        assert_eq!(dest.project_name, "my-app");
        assert!(dest.dest_path.ends_with("my-app"));
    }

    #[test]
    fn test_destination_resolution_explicit_name_only() {
        let dest = resolve_destination(None, Some("custom-project"));
        assert!(!dest.is_current_dir);
        assert_eq!(dest.project_name, "custom-project");
        assert!(dest.dest_path.ends_with("custom-project"));
    }

    #[test]
    fn test_django_excludes_sqlalchemy_and_alembic() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let compatible = filter_compatible_tools(
            tree,
            &["django".into()],
            &["python".into()],
            Some("web-app"),
            "windows",
        );

        let ids: Vec<&str> = compatible.iter().map(|t| t.id.as_str()).collect();
        assert!(!ids.contains(&"sqlalchemy"), "Django must not offer SQLAlchemy");
        assert!(!ids.contains(&"alembic"), "Django must not offer Alembic");
        assert!(!ids.contains(&"prisma"), "Django must not offer Prisma");
        assert!(ids.contains(&"postgresql"), "Django must offer PostgreSQL");
        assert!(ids.contains(&"redis"), "Django must offer Redis");
        assert!(ids.contains(&"ruff"), "Django must offer Ruff");
    }

    #[test]
    fn test_fastapi_offers_sqlalchemy_and_alembic() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let compatible = filter_compatible_tools(
            tree,
            &["fastapi".into()],
            &["python".into()],
            Some("rest-api"),
            "windows",
        );

        let ids: Vec<&str> = compatible.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"sqlalchemy"), "FastAPI should offer SQLAlchemy");
        assert!(ids.contains(&"alembic"), "FastAPI should offer Alembic");
        assert!(ids.contains(&"postgresql"), "FastAPI should offer PostgreSQL");
        assert!(!ids.contains(&"gradle"), "FastAPI must not offer Gradle (Java)");
    }

    #[test]
    fn test_strict_validation_rejects_django_with_sqlalchemy() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let res = validate_strict_stack(
            tree,
            &["django".into()],
            &["sqlalchemy".into()],
            &["python".into()],
        );
        assert!(res.is_err());
        let errs = res.unwrap_err();
        assert!(errs.iter().any(|e| e.contains("built-in ORM")));
    }

    #[test]
    fn test_strict_validation_rejects_multiple_databases() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let res = validate_strict_stack(
            tree,
            &["fastapi".into()],
            &["postgresql".into(), "mysql".into()],
            &["python".into()],
        );
        assert!(res.is_err());
        let errs = res.unwrap_err();
        assert!(errs.iter().any(|e| e.contains("mutually exclusive")));
    }

    #[test]
    fn test_resolve_tool_stack_auto_docker() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let res = resolve_tool_stack(tree, &["postgresql".into(), "redis".into()], false);
        assert!(res.docker_enabled, "Docker must be automatically enabled");
        assert!(res.resolved_tools.contains(&"docker".into()));
        assert!(!res.docker_reasons.is_empty());
    }

    #[test]
    fn test_resolve_tool_stack_auto_adds_sqlalchemy_for_alembic() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let res = resolve_tool_stack(tree, &["alembic".into()], false);
        assert!(res.resolved_tools.contains(&"sqlalchemy".into()));
        assert!(res.added_dependencies.iter().any(|(dep, _)| dep == "SQLAlchemy"));
    }
}
