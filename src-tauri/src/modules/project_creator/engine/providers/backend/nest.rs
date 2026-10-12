use crate::modules::project_creator::engine::providers::{cmd_i, RecipeProvider};
use crate::modules::project_creator::models::{InteractiveEntry, ResponseType, Step, WizardContext};

pub struct NestProvider;

impl RecipeProvider for NestProvider {
    fn id(&self) -> &'static str {
        "nest"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![
            cmd_i(
                "nest_new",
                "Create NestJS project",
                "Scaffold NestJS application",
                "npx",
                vec![
                    "--yes",
                    "@nestjs/cli",
                    "new",
                    ".",
                    "--package-manager",
                    "npm",
                    "--skip-install",
                    "--skip-git",
                ],
                vec![InteractiveEntry {
                    trigger: "Which package manager would you love to use".into(),
                    response_type: ResponseType::Text("npm".to_string()),
                }],
                project_path,
            ),
        ]
    }
}
