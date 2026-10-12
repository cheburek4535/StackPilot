use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct RemixProvider;

impl RecipeProvider for RemixProvider {
    fn id(&self) -> &'static str {
        "remix"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "remix_create",
            "Create Remix app",
            "Scaffold Remix project",
            "npx",
            vec![
                "-y",
                "create-remix@latest",
                SCAFFOLD_TARGET,
                "--template",
                "remix-run/remix/templates/remix",
                "--no-install",
                "--no-git-init",
                "--yes",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
