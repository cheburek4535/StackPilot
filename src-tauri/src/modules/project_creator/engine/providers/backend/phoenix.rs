use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct PhoenixProvider;

impl RecipeProvider for PhoenixProvider {
    fn id(&self) -> &'static str {
        "phoenix"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "phoenix_new",
            "Create Phoenix project",
            "Scaffold Phoenix application",
            "mix",
            vec!["phx.new", SCAFFOLD_TARGET],
            ScaffoldCapability::CreatesNamedDirectory,
            ".",
            ScaffoldExtras::default()
                .expects(&["mix.exs"])
                .temp_dir(false)
                .interact(vec![
                    serde_json::json!({
                        "trigger": "Fetch and install dependencies?",
                        "response_type": "y",
                    }),
                    serde_json::json!({
                        "trigger": "Would you like to build assets?",
                        "response_type": "y",
                    }),
                ]),
        )]
    }
}
