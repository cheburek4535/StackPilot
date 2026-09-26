use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub fn legacy_match(
    fw: &str,
    project_path: &str,
    _project_name: &str,
    _context: &WizardContext,
    _seg: Option<&str>,
) -> Vec<Step> {
    let cmd = |id: &str, label: &str, desc: &str, command: &str, args: Vec<&str>| -> Step {
        Step::Command {
            id: id.to_string(),
            label: label.to_string(),
            description: desc.to_string(),
            command: command.to_string(),
            args: args.into_iter().map(String::from).collect(),
            working_dir: Some(project_path.to_string()),
            env: None,
            timeout_secs: Some(300),
            condition: None,
            on_error: ErrorMode::Skip,
            interactive: vec![],
        }
    };

    match fw.to_lowercase().as_str() {
        _ => vec![cmd(
            "fw_unknown",
            "Unknown framework",
            &format!("Framework '{}' has no specific setup steps", fw),
            "echo",
            vec![&format!(
                "No automated setup available for framework: {}",
                fw
            )],
        )],
    }
}
