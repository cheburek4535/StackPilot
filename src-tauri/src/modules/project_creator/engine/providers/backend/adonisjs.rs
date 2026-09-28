use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{cmd, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct AdonisjsProvider;

impl RecipeProvider for AdonisjsProvider {
    fn id(&self) -> &'static str {
        "adonisjs"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let pkg_path = match segment {
            Some(dir) => format!("{}/package.json", dir),
            None => "package.json".to_string(),
        };
        vec![
            cmd(
                "adonisjs_create",
                "Create AdonisJS app",
                "Scaffold AdonisJS application",
                "npx",
                vec!["-y", "create-adonisjs@latest", ".", "--yes"],
                project_path,
            ),
            preflight::package_json_check_step(
                "adonisjs_pkg_check",
                "Validate AdonisJS package.json",
                &pkg_path,
                &["@adonisjs/core"],
            ),
        ]
    }
}
