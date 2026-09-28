use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct AstroProvider;

impl RecipeProvider for AstroProvider {
    fn id(&self) -> &'static str {
        "astro"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let ts_flag = if has_typescript { "--typescript" } else { "--no-typescript" };
        vec![scaffold_step(
            "astro_create",
            "Create Astro app",
            "Scaffold Astro project",
            "npx",
            vec![
                "-y",
                "create-astro@latest",
                SCAFFOLD_TARGET,
                "--template",
                "basics",
                ts_flag,
                "--no-install",
                "--no-git",
                "--yes",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
