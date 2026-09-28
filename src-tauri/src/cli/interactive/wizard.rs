use colored::Colorize;
use inquire::{Confirm, MultiSelect, Select, Text};
use std::fmt;
use std::path::PathBuf;

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
        write!(f, "{:<20} {}", self.label.bold(), self.desc.dimmed())
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
                "{:<16} {:<12} {}",
                self.label.bold(),
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
    desc: String,
}

impl fmt::Display for ToolChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<16} {:<14} {}",
            self.label.bold(),
            format!("[{}]", self.category).blue(),
            self.desc.dimmed()
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
    println!("{}", "\n🚀 Interactive Project Wizard".bold().cyan());
    println!(
        "{}",
        "Use arrow keys ↑/↓ to navigate, Space to toggle checkboxes, Enter to select.\n".dimmed()
    );

    let os = current_os();

    // 1. Choose Project Type
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

    // 2. Backend Framework
    let mut chosen_frameworks: Vec<String> = Vec::new();
    let mut chosen_backend_languages: Vec<String> = Vec::new();
    let mut chosen_frontend_languages: Vec<String> = Vec::new();

    if has_backend {
        let be_frameworks: Vec<FrameworkChoice> = std::iter::once(FrameworkChoice {
            id: "none".into(),
            label: "None (Skip Backend Framework)".into(),
            language: "-".into(),
            desc: "Don't add a backend framework".into(),
        })
        .chain(
            tree.frameworks
                .iter()
                .filter(|fw| {
                    (fw.side == "backend" || fw.side == "either")
                        && (fw.platforms.is_empty() || fw.platforms.iter().any(|p| p == os))
                        && (fw.project_types.is_empty()
                            || fw.project_types.iter().any(|t| t == &selected_type.id))
                })
                .map(|fw| FrameworkChoice {
                    id: fw.id.clone(),
                    label: fw.label.clone(),
                    language: fw.recommended_language.clone(),
                    desc: crate::cli::i18n::tr(&fw.description),
                }),
        )
        .collect();

        let selected_be = Select::new("Choose backend framework:", be_frameworks)
            .prompt()
            .map_err(|e| format!("Cancelled: {e}"))?;

        if selected_be.id != "none" {
            chosen_frameworks.push(selected_be.id.clone());
            if let Some(fw) = tree.frameworks.iter().find(|f| f.id == selected_be.id) {
                if !fw.recommended_language.is_empty() {
                    chosen_backend_languages.push(fw.recommended_language.clone());
                }
            }
        }
    }

    // 3. Frontend Framework
    let fe_frameworks: Vec<FrameworkChoice> = std::iter::once(FrameworkChoice {
        id: "none".into(),
        label: "None (Skip Frontend Framework)".into(),
        language: "-".into(),
        desc: "Don't add a frontend framework".into(),
    })
    .chain(
        tree.frameworks
            .iter()
            .filter(|fw| {
                (fw.side == "frontend" || fw.side == "either")
                    && (fw.platforms.is_empty() || fw.platforms.iter().any(|p| p == os))
                    && (fw.project_types.is_empty()
                        || fw.project_types.iter().any(|t| t == &selected_type.id))
            })
            .map(|fw| FrameworkChoice {
                id: fw.id.clone(),
                label: fw.label.clone(),
                language: fw.recommended_language.clone(),
                desc: crate::cli::i18n::tr(&fw.description),
            }),
    )
    .collect();

    let selected_fe = Select::new("Choose frontend framework:", fe_frameworks)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    if selected_fe.id != "none" {
        chosen_frameworks.push(selected_fe.id.clone());
        if let Some(fw) = tree.frameworks.iter().find(|f| f.id == selected_fe.id) {
            if !fw.recommended_language.is_empty() {
                chosen_frontend_languages.push(fw.recommended_language.clone());
            }
        }
    }

    // 4. Tools & Databases (MultiSelect)
    let tool_choices: Vec<ToolChoice> = tree
        .tools
        .iter()
        .filter(|t| t.platforms.is_empty() || t.platforms.iter().any(|p| p == os))
        .map(|t| ToolChoice {
            id: t.id.clone(),
            label: t.label.clone(),
            category: t.category.clone(),
            desc: crate::cli::i18n::tr(&t.description),
        })
        .collect();

    let selected_tools = MultiSelect::new(
        "Select tools, databases and services (Space toggles, Enter confirms):",
        tool_choices,
    )
    .prompt()
    .map_err(|e| format!("Cancelled: {e}"))?;

    let chosen_tools: Vec<String> = selected_tools.into_iter().map(|t| t.id).collect();

    // 5. Destination Path
    let default_path_str = if let Some(ref p) = initial_path {
        p.to_string_lossy().to_string()
    } else {
        ".".to_string()
    };

    let path_input = Text::new("Destination directory:")
        .with_default(&default_path_str)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    let dest_path = PathBuf::from(&path_input);

    // 6. Project Name
    let default_name = if dest_path.as_os_str() == "." || dest_path.as_os_str().is_empty() {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "my-project".into())
    } else {
        dest_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "my-project".into())
    };

    let name_input = Text::new("Project name:")
        .with_default(&default_name)
        .prompt()
        .map_err(|e| format!("Cancelled: {e}"))?;

    // 7. Extra Options
    let git_init = Confirm::new("Initialize git repository?")
        .with_default(true)
        .prompt()
        .unwrap_or(true);

    let open_in_ide = Confirm::new("Open in VS Code when done?")
        .with_default(true)
        .prompt()
        .unwrap_or(true);

    let ide_to_open = if open_in_ide {
        Some("code".to_string())
    } else {
        None
    };

    let mut languages = chosen_backend_languages.clone();
    languages.extend(chosen_frontend_languages.clone());

    let context = WizardContext {
        project_path: Some(dest_path.clone()),
        project_name: Some(name_input),
        is_existing: false,
        project_type: Some(selected_type.id),
        languages,
        backend_languages: chosen_backend_languages,
        frontend_languages: chosen_frontend_languages,
        frameworks: chosen_frameworks,
        tools: chosen_tools,
        local_infra_tools: vec![],
        features: vec![],
        infrastructure: vec![],
        docker: false,
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
        dest_path,
        open_ide: ide_to_open,
    })
}
