use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct AspNetCoreProvider;

impl RecipeProvider for AspNetCoreProvider {
    fn id(&self) -> &'static str {
        "aspnetcore"
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
            Step::Command {
                id: "aspnet_new".into(),
                label: "Create ASP.NET Core Web API".into(),
                description: "Scaffold Web API project".into(),
                command: "dotnet".into(),
                args: vec![
                    "new".into(),
                    "webapi".into(),
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
                "aspnet_csproj_check",
                "Validate ASP.NET Core csproj",
                &csproj_path,
                "csproj_xml",
                &["Microsoft.AspNetCore.OpenApi"],
            ),
        ]
    }
}
