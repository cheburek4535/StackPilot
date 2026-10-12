use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{ScaffoldCapability, Step, WizardContext};

pub struct SveltekitProvider;

impl RecipeProvider for SveltekitProvider {
    fn id(&self) -> &'static str {
        "sveltekit"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let mut sv_args = vec![
            "sv",
            "create",
            SCAFFOLD_TARGET,
            "--template",
            "minimal",
            "--no-add-ons",
            "--no-install",
        ];
        if has_typescript {
            sv_args.push("--types");
            sv_args.push("ts");
        } else {
            sv_args.push("--no-types");
        }
        vec![scaffold_step(
            "sveltekit_create",
            "Create SvelteKit app",
            "Scaffold SvelteKit project",
            "npx",
            sv_args,
            ScaffoldCapability::CreatesNamedDirectory,
            "frontend",
            ScaffoldExtras::default().expects(&["package.json"]),
        )]
    }
}
