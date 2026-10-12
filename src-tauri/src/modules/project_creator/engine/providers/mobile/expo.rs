use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct ExpoProvider;

impl RecipeProvider for ExpoProvider {
    fn id(&self) -> &'static str {
        "expo"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let expo_template = if has_typescript {
            "blank-typescript"
        } else {
            "blank"
        };
        vec![scaffold_step(
            "expo_init",
            "Init Expo",
            "Create Expo project",
            "npx",
            vec![
                "create-expo-app",
                SCAFFOLD_TARGET,
                "--yes",
                "--no-install",
                "--template",
                expo_template,
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
