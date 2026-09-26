use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, StepCondition, WizardContext};

pub struct CobraProvider;

impl RecipeProvider for CobraProvider {
    fn id(&self) -> &'static str {
        "cobra"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let (cli_path, cli_id, cli_label) = if context.frameworks.iter().any(|f| f == "gin") {
            (
                "cmd/cli/main.go",
                "cobra_cli",
                "Create CLI entry (cmd/cli/main.go)",
            )
        } else {
            ("cmd/main.go", "cobra_main", "Create CLI entry")
        };

        let go_mod_path = match segment {
            Some(dir) => format!("{}/go.mod", dir),
            None => "go.mod".to_string(),
        };

        vec![
            Step::Command {
                id: "get_cobra".into(),
                label: "Add Cobra dependency".into(),
                description: "Add cobra to go.mod (modern go get pkg@latest)".into(),
                command: "go".into(),
                args: vec!["get".into(), "github.com/spf13/cobra@latest".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: Some(StepCondition::FileExists { path: go_mod_path }),
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
            write_file(
                cli_id,
                cli_label,
                cli_path,
                &format!(
                    r#"package main

import (
    "fmt"
    "github.com/spf13/cobra"
)

func main() {{
    var rootCmd = &cobra.Command{{
        Use:   "{}",
        Short: "A CLI tool built with Cobra",
        Run: func(cmd *cobra.Command, args []string) {{
            fmt.Println("Hello from {}!")
        }},
    }}
    rootCmd.Execute()
}}
"#,
                    project_name, project_name
                ),
            ),
        ]
    }
}
