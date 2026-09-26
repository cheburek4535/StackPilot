use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct MauiProvider;

impl RecipeProvider for MauiProvider {
    fn id(&self) -> &'static str {
        "maui"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let csproj_path = match segment {
            Some(dir) => format!("{}/{}.csproj", dir, project_name),
            None => format!("{}.csproj", project_name),
        };
        vec![
            Step::Generate {
                id: "maui_templates".into(),
                label: "Ensure .NET MAUI templates".into(),
                description: "Install the .NET MAUI workload so the `maui` template is available (no-op when already installed)".into(),
                generator_id: "dotnet-maui-ensure".into(),
                generator_config: serde_json::json!({}),
                policy: None,
                condition: None,
                on_error: ErrorMode::Abort,
            },
            Step::Command {
                id: "maui_new".into(),
                label: "Create MAUI app".into(),
                description: "Scaffold .NET MAUI project".into(),
                command: "dotnet".into(),
                args: vec![
                    "new".into(),
                    "maui".into(),
                    "-n".into(),
                    project_name.into(),
                    "-o".into(),
                    ".".into(),
                    "--force".into(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: None,
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
            preflight::manifest_check_step(
                "maui_csproj_check",
                "Validate MAUI csproj",
                &csproj_path,
                "csproj_xml",
                &["Microsoft.Maui.Controls"],
            ),
        ]
    }
}
