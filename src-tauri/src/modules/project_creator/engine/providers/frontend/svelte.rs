use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct SvelteProvider;

impl RecipeProvider for SvelteProvider {
    fn id(&self) -> &'static str {
        "svelte"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let template = if has_typescript { "svelte-ts" } else { "svelte" };
        vec![scaffold_step(
            "vite_create",
            "Create svelte app",
            "Scaffold Vite project",
            "npx",
            vec![
                "create-vite@latest",
                SCAFFOLD_TARGET,
                "--template",
                template,
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
