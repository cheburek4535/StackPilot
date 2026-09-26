use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct NextjsProvider;

impl RecipeProvider for NextjsProvider {
    fn id(&self) -> &'static str {
        "nextjs"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let ts_flag = if has_typescript {
            "--typescript"
        } else {
            "--javascript"
        };
        vec![scaffold_step(
            "nextjs_create",
            "Create Next.js app",
            "Scaffold Next.js project",
            "npx",
            vec![
                "create-next-app@latest",
                SCAFFOLD_TARGET,
                ts_flag,
                "--tailwind",
                "--eslint",
                "--app",
                "--no-src-dir",
                "--import-alias",
                "@/*",
                "--use-npm",
                "--skip-install",
                "--yes",
            ],
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
