use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct SolidjsProvider;

impl RecipeProvider for SolidjsProvider {
    fn id(&self) -> &'static str {
        "solidjs"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let template = "basic";
        vec![scaffold_step(
            "solid_init",
            "Create SolidStart app",
            "Scaffold SolidStart project in frontend/",
            "npx",
            vec![
                "--yes",
                "create-solid",
                SCAFFOLD_TARGET,
                template,
                "--solidstart",
                "--v2",
                "--ts",
            ],
            ScaffoldCapability::CreatesProjectAndMayPrompt,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
