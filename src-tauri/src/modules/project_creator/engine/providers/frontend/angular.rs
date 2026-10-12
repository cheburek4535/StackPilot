use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct AngularProvider;

impl RecipeProvider for AngularProvider {
    fn id(&self) -> &'static str {
        "angular"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "angular_create",
            "Create Angular app",
            "Scaffold Angular project via @angular/cli",
            "npx",
            vec![
                "-y",
                "@angular/cli",
                "new",
                SCAFFOLD_TARGET,
                "--defaults",
                "--skip-git",
                "--skip-install",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
