use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::engine::{
    framework_def, scaffold_step, tauri_identifier, ScaffoldExtras,
};
use crate::modules::project_creator::generators::SCAFFOLD_TARGET;
use crate::modules::project_creator::models::{
    ErrorMode, FilePolicy, ScaffoldCapability, Step, StepCondition, WizardContext,
};

pub struct TauriProvider;

impl RecipeProvider for TauriProvider {
    fn id(&self) -> &'static str {
        "tauri"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let has_typescript = context.languages.iter().any(|l| l == "typescript");
        let identifier = tauri_identifier(project_name);
        let has_companion = context.frameworks.iter().any(|f| {
            framework_def("tauri").is_some_and(|def| def.companions.iter().any(|c| c == f))
        });
        let frontend_dist = "../frontend/dist".to_string();
        let dev_cmd = "cd frontend && npm run dev".to_string();
        let build_cmd = "cd frontend && npm run build".to_string();
        let mut steps: Vec<Step> = Vec::new();

        if !has_companion {
            let template = if has_typescript {
                "vanilla-ts"
            } else {
                "vanilla"
            };
            steps.push(scaffold_step(
                "tauri_web_scaffold",
                "Create frontend for Tauri",
                "Scaffold Vite frontend in frontend/",
                "npx",
                vec![
                    "create-vite@latest",
                    SCAFFOLD_TARGET,
                    "--template",
                    template,
                ],
                ScaffoldCapability::CreatesNamedDirectory,
                "frontend",
                ScaffoldExtras::default()
                    .expects(&["package.json"])
                    .policy(FilePolicy::SkipIfExists),
            ));
            steps.push(Step::Command {
                id: "tauri_web_install".into(),
                label: "Install Tauri frontend dependencies".into(),
                description: "Run npm install inside frontend/".into(),
                command: "npm".into(),
                args: vec!["install".into()],
                working_dir: Some("frontend".into()),
                env: None,
                timeout_secs: Some(600),
                condition: Some(StepCondition::FileExists {
                    path: "frontend/package.json".into(),
                }),
                on_error: ErrorMode::Skip,
                interactive: vec![],
            });
        }

        steps.push(scaffold_step(
            "tauri_init",
            "Initialize Tauri shell",
            "Run tauri init (non-interactive, --ci)",
            "npx",
            vec![
                "--yes",
                "@tauri-apps/cli",
                "init",
                "--ci",
                "--app-name",
                project_name,
                "--window-title",
                project_name,
                "--frontend-dist",
                frontend_dist.as_str(),
                "--dev-url",
                "http://localhost:5173",
                "--before-dev-command",
                dev_cmd.as_str(),
                "--before-build-command",
                build_cmd.as_str(),
            ],
            ScaffoldCapability::GeneratesRootShell,
            ".",
            ScaffoldExtras::default()
                .expects(&["src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"])
                .policy(FilePolicy::SkipIfExists),
        ));

        if let Some(Step::Generate { condition, .. }) = steps.last_mut() {
            *condition = Some(StepCondition::FileExists {
                path: "frontend/package.json".into(),
            });
        }

        steps.push(Step::Generate {
            id: "tauri_config_patch".into(),
            label: "Patch Tauri configuration".into(),
            description: "Adapt src-tauri/tauri.conf.json to the frontend/ layout".into(),
            generator_id: "tauri-config".into(),
            generator_config: serde_json::json!({
                "frontend_dir": "frontend",
                "tauri_dir": "",
                "frontend_dist": frontend_dist,
                "dev_url": "http://localhost:5173",
                "before_dev_command": dev_cmd,
                "before_build_command": build_cmd,
                "identifier": identifier,
            }),
            policy: None,
            condition: Some(StepCondition::FileExists {
                path: "src-tauri/tauri.conf.json".into(),
            }),
            on_error: ErrorMode::Skip,
        });

        steps
    }
}
