use crate::modules::project_creator::engine::android_steps;
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct JetpackComposeProvider;

impl RecipeProvider for JetpackComposeProvider {
    fn id(&self) -> &'static str {
        "jetpack-compose"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        if context.frameworks.iter().any(|f| f == "android") {
            vec![]
        } else {
            android_steps(project_name, project_path, true, false, segment)
        }
    }
}
