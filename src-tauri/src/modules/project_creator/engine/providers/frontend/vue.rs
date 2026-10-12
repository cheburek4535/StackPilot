use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct VueProvider;

impl RecipeProvider for VueProvider {
    fn id(&self) -> &'static str {
        "vue"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let template = if has_typescript { "vue-ts" } else { "vue" };
        vec![scaffold_step(
            "vite_create",
            "Create vue app",
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
