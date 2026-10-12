use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct NuxtProvider;

impl RecipeProvider for NuxtProvider {
    fn id(&self) -> &'static str {
        "nuxt"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "nuxt_create",
            "Create Nuxt app",
            "Scaffold Nuxt project",
            "npx",
            vec![
                "--yes",
                "nuxi@latest",
                "init",
                SCAFFOLD_TARGET,
                "--template",
                "minimal",
                "--packageManager",
                "npm",
                "--gitInit=false",
                "--no-install",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
