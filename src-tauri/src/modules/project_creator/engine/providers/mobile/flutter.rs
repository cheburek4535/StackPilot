use crate::modules::project_creator::engine::{scaffold_step, ScaffoldExtras};
use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{
    ErrorMode, FilePolicy, ScaffoldCapability, Step, WizardContext,
};

pub struct FlutterProvider;

impl RecipeProvider for FlutterProvider {
    fn id(&self) -> &'static str {
        "flutter"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let safe_name = project_name.replace('-', "_");
        vec![
            Step::Command {
                id: "flutter_preflight".into(),
                label: "Check Flutter SDK".into(),
                description: "Verify the Flutter SDK is installed (flutter --version) before scaffolding the app".into(),
                command: "flutter".into(),
                args: vec!["--version".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(60),
                condition: None,
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
            scaffold_step(
                "flutter_create",
                "Create Flutter project",
                "Scaffold Flutter app",
                "flutter",
                vec!["create", "--project-name", &safe_name, SCAFFOLD_TARGET],
                ScaffoldCapability::CreatesInCurrentDirectory,
                "frontend",
                ScaffoldExtras::default()
                    .expects(&["pubspec.yaml", "lib"])
                    .policy(FilePolicy::SkipIfExists),
            ),
        ]
    }
}
