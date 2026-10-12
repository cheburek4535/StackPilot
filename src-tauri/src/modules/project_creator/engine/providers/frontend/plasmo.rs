use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct PlasmoProvider;

impl RecipeProvider for PlasmoProvider {
    fn id(&self) -> &'static str {
        "plasmo"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let has_javascript = context.languages.iter().any(|l| l == "javascript");
        vec![scaffold_step(
            "plasmo_init",
            "Init Plasmo",
            "Create browser extension with Plasmo",
            "npx",
            vec!["plasmo", "init", SCAFFOLD_TARGET],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default()
                .expects(&["package.json"])
                .temp_dir(false)
                .interact(vec![
                    serde_json::json!({
                        "trigger": "Project name",
                        "response_type": project_name,
                    }),
                    serde_json::json!({
                        "trigger": "Select your primary framework/compiler",
                        "response_type": if has_typescript || has_javascript {
                            "React (Next-like)"
                        } else {
                            "Vanilla"
                        },
                    }),
                ]),
        )]
    }
}
