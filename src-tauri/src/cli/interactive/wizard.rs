use colored::Colorize;
use inquire::{Confirm, MultiSelect, Select, Text};
use std::fmt;
use std::path::PathBuf;

use crate::cli::constraints::{
    detect_tool_conflicts, filter_available_frameworks, filter_compatible_tools,
    is_dir_non_empty, resolve_destination, resolve_tool_stack,
};
use crate::modules::project_creator::models::{WizardContext, WizardTreeData};
use crate::modules::project_creator::validate::current_os;

#[derive(Clone)]
struct ProjectTypeChoice {
    id: String,
    label: String,
    desc: String,
}

impl fmt::Display for ProjectTypeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:<22} {}", self.label.bold().bright_white(), self.desc.dimmed())
    }
}

#[derive(Clone)]
struct FrameworkChoice {
    id: String,
    label: String,
    language: String,
    desc: String,
}

impl fmt::Display for FrameworkChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.id == "none" {
            write!(f, "{}", self.label.dimmed())
        } else {
            write!(
                f,
                "{:<18} {:<14} {}",
                self.label.bold().bright_white(),
                format!("[{}]", self.language).cyan(),
                self.desc.dimmed()
            )
        }
    }
}

#[derive(Clone)]
struct ToolChoice {
    id: String,
    label: String,
    category: String,
    requires_docker: bool,
    desc: String,
}

impl fmt::Display for ToolChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let docker_tag = if self.requires_docker {
            " (🐳 Docker)".yellow().to_string()
        } else {
            String::new()
        };
        write!(
            f,
            "{:<18} {:<14} {}{}",
            self.label.bold().bright_white(),
            format!("[{}]", self.category).blue(),
            self.desc.dimmed(),
            docker_tag
        )
    }
}

pub struct WizardResult {
    pub context: WizardContext,
    pub dest_path: PathBuf,
    pub open_ide: Option<String>,
}

pub fn run_interactive_wizard(
    tree: &WizardTreeData,
    initial_path: Option<PathBuf>,
) -> Result<WizardResult, String> {
    println!("{}", "\n⚡ StackPilot Interactive Project Creator".bold().cyan());
    println!(
        "{}\n",
        "  Build and verify your development stack in 5 quick steps."
            .dimmed()
    );

    let os = current_os();

    // ─────────────────────────────────────────────────────────
    // STEP 1: Project Type
    // ─────────────────────────────────────────────────────────
    println!("{}", "Step 1/5 • Select Project Type".bold().bright_blue());
    let type_choices: Vec<ProjectTypeChoice> = tree
        .project_types
        .iter()
        .map(|pt| ProjectTypeChoice {
            id: pt.id.clone(),
            label: crate::cli::i18n::tr(&pt.label),
            desc: crate::cli::i18n::tr(&pt.description),
        })
        .collect();

    let selected_type = Select::new("Choose project type:", type_choices)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    let pt_def = tree
        .project_types
        .iter()
        .find(|pt| pt.id == selected_type.id);
    let has_backend = pt_def.map(|p| p.has_backend).unwrap_or(true);

    println!(
        "  {} {}\n",
        "Selected type:".dimmed(),
        selected_type.label.green().bold()
    );

    // ─────────────────────────────────────────────────────────
    // STEP 2: Backend Framework
    // ─────────────────────────────────────────────────────────
    let mut chosen_frameworks: Vec<String> = Vec::new();
    let mut chosen_backend_languages: Vec<String> = Vec::new();
    let mut chosen_frontend_languages: Vec<String> = Vec::new();

    if has_backend {
        println!("{}", "Step 2/5 • Backend Framework".bold().bright_blue());
        let available_be = filter_available_frameworks(
            tree,
            "backend",
            Some(&selected_type.id),
            os,
            &chosen_frameworks,
        );

        let be_choices: Vec<FrameworkChoice> = std::iter::once(FrameworkChoice {
            id: "none".into(),
            label: "None (Skip Backend)".into(),
            language: "-".into(),
            desc: "Do not add a backend framework".into(),
        })
        .chain(available_be.into_iter().map(|fw| FrameworkChoice {
            id: fw.id.clone(),
            label: fw.label.clone(),
            language: fw.recommended_language.clone(),
            desc: crate::cli::i18n::tr(&fw.description),
        }))
        .collect();

        let selected_be = Select::new("Choose backend framework:", be_choices)
            .prompt()
            .map_err(|e| format!("Cancelled: {e}"))?;

        if selected_be.id != "none" {
            chosen_frameworks.push(selected_be.id.clone());
            if let Some(fw) = tree.frameworks.iter().find(|f| f.id == selected_be.id) {
                if !fw.recommended_language.is_empty() {
                    chosen_backend_languages.push(fw.recommended_language.clone());
                }
            }
            println!(
                "  {} {}\n",
                "Selected backend:".dimmed(),
                selected_be.label.green().bold()
            );
        } else {
            println!("  {}\n", "No backend framework selected.".dimmed());
        }
    } else {
        println!("  {}\n", "Step 2/5 • Backend not required for this project type.".dimmed());
    }

    // ─────────────────────────────────────────────────────────
    // STEP 3: Frontend Framework
    // ─────────────────────────────────────────────────────────
    println!("{}", "Step 3/5 • Frontend Framework".bold().bright_blue());
    let available_fe = filter_available_frameworks(
        tree,
        "frontend",
        Some(&selected_type.id),
        os,
        &chosen_frameworks,
    );

    let fe_choices: Vec<FrameworkChoice> = std::iter::once(FrameworkChoice {
        id: "none".into(),
        label: "None (Skip Frontend)".into(),
        language: "-".into(),
        desc: "Do not add a frontend framework".into(),
    })
    .chain(available_fe.into_iter().map(|fw| FrameworkChoice {
        id: fw.id.clone(),
        label: fw.label.clone(),
        language: fw.recommended_language.clone(),
        desc: crate::cli::i18n::tr(&fw.description),
    }))
    .collect();

    let selected_fe = Select::new("Choose frontend framework:", fe_choices)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    if selected_fe.id != "none" {
        chosen_frameworks.push(selected_fe.id.clone());
        if let Some(fw) = tree.frameworks.iter().find(|f| f.id == selected_fe.id) {
            if !fw.recommended_language.is_empty() {
                chosen_frontend_languages.push(fw.recommended_language.clone());
            }
        }
        println!(
            "  {} {}\n",
            "Selected frontend:".dimmed(),
            selected_fe.label.green().bold()
        );
    } else {
        println!("  {}\n", "No frontend framework selected.".dimmed());
    }

    // Determine current project languages
    let mut project_languages = chosen_backend_languages.clone();
    for l in &chosen_frontend_languages {
        if !project_languages.contains(l) {
            project_languages.push(l.clone());
        }
    }

    // ─────────────────────────────────────────────────────────
    // STEP 4: Tools & Databases (Dynamic Constraints & Exclusions)
    // ─────────────────────────────────────────────────────────
    println!("{}", "Step 4/5 • Tools, Databases & Services".bold().bright_blue());

    // Inform user if certain tools were excluded due to framework capabilities
    let has_django = chosen_frameworks.iter().any(|f| f == "django");
    let has_rails = chosen_frameworks.iter().any(|f| f == "rails");
    if has_django {
        println!(
            "  {} {}",
            "ℹ".cyan(),
            "Django includes built-in ORM & migrations — overlapping ORMs (SQLAlchemy, Alembic) excluded."
                .dimmed()
        );
    } else if has_rails {
        println!(
            "  {} {}",
            "ℹ".cyan(),
            "Ruby on Rails includes Active Record — external ORMs excluded.".dimmed()
        );
    }

    let compatible_tool_defs = filter_compatible_tools(
        tree,
        &chosen_frameworks,
        &project_languages,
        Some(&selected_type.id),
        os,
    );

    let tool_choices: Vec<ToolChoice> = compatible_tool_defs
        .iter()
        .map(|t| ToolChoice {
            id: t.id.clone(),
            label: t.label.clone(),
            category: t.category.clone(),
            requires_docker: t.requires_docker,
            desc: crate::cli::i18n::tr(&t.description),
        })
        .collect();

    let mut selected_tool_ids = if !tool_choices.is_empty() {
        let multi = MultiSelect::new(
            "Select tools and services (Space toggles, Enter confirms, empty for none):",
            tool_choices,
        )
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

        multi.into_iter().map(|t| t.id).collect::<Vec<String>>()
    } else {
        println!("  {}", "No additional tools available for this stack.".dimmed());
        Vec::new()
    };

    // Check for mutual exclusions between selected tools and interactively resolve
    let conflicts = detect_tool_conflicts(tree, &selected_tool_ids);
    for (tool_a_id, tool_b_id, reason) in conflicts {
        if selected_tool_ids.contains(&tool_a_id) && selected_tool_ids.contains(&tool_b_id) {
            let label_a = tree.tools.iter().find(|t| t.id == tool_a_id).map(|t| t.label.clone()).unwrap_or(tool_a_id.clone());
            let label_b = tree.tools.iter().find(|t| t.id == tool_b_id).map(|t| t.label.clone()).unwrap_or(tool_b_id.clone());

            println!("\n  {} {}", "⚠ Mutual exclusion:".yellow().bold(), reason);
            let resolve_choices = vec![
                format!("Keep {} (drop {})", label_a.bold(), label_b),
                format!("Keep {} (drop {})", label_b.bold(), label_a),
            ];
            let pick = Select::new("Choose which tool to retain in project:", resolve_choices)
                .prompt()
                .map_err(|e| format!("Cancelled: {e}"))?;

            if pick.contains(&label_a) {
                selected_tool_ids.retain(|id| id != &tool_b_id);
                println!("  {} Kept {}", "✔".green(), label_a.bold());
            } else {
                selected_tool_ids.retain(|id| id != &tool_a_id);
                println!("  {} Kept {}", "✔".green(), label_b.bold());
            }
        }
    }

    // Auto-resolve tool dependencies (e.g. Alembic -> SQLAlchemy) and detect Docker
    let resolution = resolve_tool_stack(tree, &selected_tool_ids, false);
    let chosen_tools = resolution.resolved_tools;

    for (dep_label, parent_label) in &resolution.added_dependencies {
        println!(
            "  {} Added required dependency: {} (needed by {})",
            "ℹ".cyan(),
            dep_label.bold(),
            parent_label.bold()
        );
    }

    if resolution.docker_enabled {
        println!(
            "  {} Docker automatically enabled (required by: {})\n",
            "🐳".cyan().bold(),
            resolution.docker_reasons.join(", ").yellow().bold()
        );
    } else {
        println!();
    }

    // ─────────────────────────────────────────────────────────
    // STEP 5: Project Setup & Destination Directory
    // ─────────────────────────────────────────────────────────
    println!("{}", "Step 5/5 • Project Setup & Destination".bold().bright_blue());

    // 1. Project Name
    let initial_name = initial_path
        .as_ref()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-project".to_string());

    let name_input = Text::new("Project name:")
        .with_default(&initial_name)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;
    let project_name = name_input.trim().to_string();

    // 2. Destination directory
    let default_folder = format!("./{}", project_name);
    let dir_prompt = format!(
        "Target directory (press Enter for '{}', or enter '.' for current folder):",
        default_folder
    );

    let path_input = Text::new(&dir_prompt)
        .with_default(&default_folder)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    let resolved_dest = resolve_destination(Some(&path_input), Some(&project_name));
    println!("  📁 {}\n", resolved_dest.display_preview.cyan());

    // Check if non-empty folder
    if is_dir_non_empty(&resolved_dest.dest_path) {
        println!(
            "  {} Directory '{}' is NOT empty. Existing files may be modified.",
            "⚠ WARNING:".yellow().bold(),
            resolved_dest.dest_path.display()
        );
        let proceed = Confirm::new("Proceed with this directory?")
            .with_default(false)
            .prompt()
            .unwrap_or(false);
        if !proceed {
            return Err("Operation cancelled: destination directory is not empty.".into());
        }
    }

    // 3. Docker (if not already auto-enabled by tools)
    let docker_enabled = if resolution.docker_enabled {
        true
    } else {
        Confirm::new("Include Docker setup (Dockerfile & compose)?")
            .with_default(false)
            .prompt()
            .unwrap_or(false)
    };

    // 4. Git init
    let git_init = Confirm::new("Initialize git repository?")
        .with_default(true)
        .prompt()
        .unwrap_or(true);

    // 5. Open in editor
    let open_in_ide = Confirm::new("Open in VS Code when ready?")
        .with_default(true)
        .prompt()
        .unwrap_or(true);

    let ide_to_open = if open_in_ide {
        Some("code".to_string())
    } else {
        None
    };

    let mut all_tools = chosen_tools;
    if docker_enabled && !all_tools.contains(&"docker".to_string()) {
        all_tools.push("docker".to_string());
    }

    let context = WizardContext {
        project_path: Some(resolved_dest.dest_path.clone()),
        project_name: Some(project_name),
        is_existing: false,
        project_type: Some(selected_type.id),
        languages: project_languages,
        backend_languages: chosen_backend_languages,
        frontend_languages: chosen_frontend_languages,
        frameworks: chosen_frameworks,
        tools: all_tools,
        local_infra_tools: vec![],
        features: vec![],
        infrastructure: vec![],
        docker: docker_enabled,
        testing: false,
        ci: false,
        git_init,
        vscode_config: open_in_ide,
        answers: Default::default(),
        environment_binding_id: None,
        readme_locale: None,
    };

    Ok(WizardResult {
        context,
        dest_path: resolved_dest.dest_path,
        open_ide: ide_to_open,
    })
}
