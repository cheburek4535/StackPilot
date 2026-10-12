use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{
    FilePolicy, ScaffoldCapability, Step, WizardContext,
};

pub struct ElectronProvider;

impl RecipeProvider for ElectronProvider {
    fn id(&self) -> &'static str {
        "electron"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![scaffold_step(
            "electron_init",
            "Init Electron",
            "Create Electron app with electron-forge",
            "npx",
            vec!["create-electron-app", SCAFFOLD_TARGET, "--template", "vite"],
            ScaffoldCapability::CreatesProjectAndMayPrompt,
            "frontend",
            ScaffoldExtras::default()
                .expects(&["package.json"])
                .policy(FilePolicy::SkipIfExists)
                .interact(vec![serde_json::json!({
                    "trigger": "Initialize a git repository?",
                    "response_type": "n",
                })]),
        )]
    }
}
