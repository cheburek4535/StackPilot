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
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let pkg_path = match segment {
            Some(dir) => format!("{}/package.json", dir),
            None => "package.json".to_string(),
        };
        let pkg_flag = if context.tools.iter().any(|t| t == "pnpm") {
            "--pkg=pnpm"
        } else if context.tools.iter().any(|t| t == "yarn") {
            "--pkg=yarn"
        } else if context.tools.iter().any(|t| t == "bun") {
            "--pkg=bun"
        } else {
            "--pkg=npm"
        };
        vec![
            cmd(
                "adonisjs_create",
                "Create AdonisJS app",
                "Scaffold AdonisJS application",
                "npx",
                vec![
                    "-y",
                    "create-adonisjs@latest",
                    ".",
                    "--kit=github:adonisjs/slim-starter-kit",
                    pkg_flag,
                    "--skip-migrations",
                ],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adonisjs_generates_unattended_scaffold_command() {
        let provider = AdonisjsProvider;
        let mut ctx = WizardContext::default();
        ctx.tools = vec!["pnpm".into()];
        let steps = provider.generate_steps("backend", "my-adonis-app", &ctx, Some("backend"));
        assert_eq!(steps.len(), 2);

        match &steps[0] {
            Step::Command {
                id,
                command,
                args,
                working_dir,
                ..
            } => {
                assert_eq!(id, "adonisjs_create");
                assert_eq!(command, "npx");
                assert!(args.contains(&"--kit=github:adonisjs/slim-starter-kit".to_string()));
                assert!(args.contains(&"--pkg=pnpm".to_string()));
                assert!(args.contains(&"--skip-migrations".to_string()));
                assert_eq!(working_dir.as_deref(), Some("backend"));
            }
            _ => panic!("Expected Command step"),
        }

        match &steps[1] {
            Step::Generate {
                id,
                generator_config,
                ..
            } => {
                assert_eq!(id, "adonisjs_pkg_check");
                assert_eq!(generator_config["path"], "backend/package.json");
            }
            _ => panic!("Expected Generate step"),
        }
    }
}

