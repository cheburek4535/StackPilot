use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct VaporProvider;

impl RecipeProvider for VaporProvider {
    fn id(&self) -> &'static str {
        "vapor"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "vapor_new",
            "Create Vapor project",
            "Scaffold Vapor application",
            "vapor",
            vec!["new", SCAFFOLD_TARGET],
            ScaffoldCapability::CreatesNamedDirectory,
            ".",
            ScaffoldExtras::default()
                .expects(&["Package.swift"])
                .temp_dir(false)
                .interact(vec![
                    serde_json::json!({
                        "trigger": "Would you like to use Fluent?",
                        "response_type": "y",
                    }),
                    serde_json::json!({
                        "trigger": "Choose a database engine:",
                        "response_type": "SQLite",
                    }),
                    serde_json::json!({
                        "trigger": "Would you like to use Leaf?",
                        "response_type": "n",
                    }),
                ]),
        )]
    }
}
