use crate::modules::project_creator::engine::{composer_scaffold_step, preflight};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct LaravelProvider;

impl RecipeProvider for LaravelProvider {
    fn id(&self) -> &'static str {
        "laravel"
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
                "laravel_php_check",
                "Check PHP and Composer for Laravel",
                "Verify PHP (version, php.ini, extension_dir, fileinfo) and Composer availability before composer create-project",
                "laravel/laravel",
            ),
            composer_scaffold_step(
                "laravel_new",
                "Create Laravel project",
                "Scaffold Laravel application via PHP Composer",
                "laravel/laravel",
            ),
            preflight::manifest_check_step(
                "laravel_composer_check",
                "Validate Laravel composer.json",
                &composer_path,
                "composer_json",
                &["laravel/framework"],
            ),
        ]
    }
}
