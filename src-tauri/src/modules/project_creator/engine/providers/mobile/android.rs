use crate::modules::project_creator::engine::android_steps;
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct AndroidProvider;

impl RecipeProvider for AndroidProvider {
    fn id(&self) -> &'static str {
        "android"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let compose = context.frameworks.iter().any(|f| f == "jetpack-compose");
        let java_lang = context.languages.iter().any(|l| l == "java");
        android_steps(project_name, project_path, compose, java_lang, segment)
    }
}
