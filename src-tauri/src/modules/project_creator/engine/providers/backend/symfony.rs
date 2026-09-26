use crate::modules::project_creator::engine::{composer_scaffold_step, preflight};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct SymfonyProvider;

impl RecipeProvider for SymfonyProvider {
    fn id(&self) -> &'static str {
        "symfony"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let composer_path = match segment {
            Some(dir) => format!("{}/composer.json", dir),
            None => "composer.json".to_string(),
        };
        vec![
            preflight::php_preflight_step(
                "symfony_php_check",
                "Check PHP and Composer for Symfony",
                "Verify PHP (version, php.ini, extension_dir, fileinfo) and Composer availability before composer create-project",
                "symfony/skeleton",
            ),
            composer_scaffold_step(
                "symfony_new",
                "Create Symfony project",
                "Scaffold Symfony application via PHP Composer",
                "symfony/skeleton",
            ),
            preflight::manifest_check_step(
                "symfony_composer_check",
                "Validate Symfony composer.json",
                &composer_path,
                "composer_json",
                &["symfony/framework-bundle"],
            ),
        ]
    }
}
