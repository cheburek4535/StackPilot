use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct ReactNativeProvider;

impl RecipeProvider for ReactNativeProvider {
    fn id(&self) -> &'static str {
        "react-native"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "rn_init",
            "Init React Native",
            "Create React Native project",
            "npx",
            vec![
                "@react-native-community/cli",
                "init",
                SCAFFOLD_TARGET,
                "--skip-install",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default()
                .expects(&["package.json"])
                .temp_dir(false)
                .interact(vec![
                    serde_json::json!({
                        "trigger": "Do you want to install CocoaPods dependencies?",
                        "response_type": "n",
                    }),
                    serde_json::json!({
                        "trigger": "Downloading and installing the modern architecture dependencies. Proceed?",
                        "response_type": "n",
                    }),
                ]),
        )]
    }
}
