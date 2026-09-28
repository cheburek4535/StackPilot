use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, StepCondition, WizardContext};

pub struct EchoProvider;

impl RecipeProvider for EchoProvider {
    fn id(&self) -> &'static str {
        "echo"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let go_mod_path = match segment {
            Some(dir) => format!("{}/go.mod", dir),
            None => "go.mod".to_string(),
        };

        vec![
            write_file(
                "echo_main",
                "Create Echo entry",
                "cmd/main.go",
                &format!(
                    r#"package main

import (
    "net/http"
    "github.com/labstack/echo/v4"
)

func main() {{
    e := echo.New()
    e.GET("/", func(c echo.Context) error {{
        return c.JSON(http.StatusOK, map[string]string{{"message": "Hello from {}!"}})
    }})
    e.Logger.Fatal(e.Start(":1323"))
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "get_echo".into(),
                label: "Install Echo".into(),
                description: "Add Echo dependency (go get github.com/labstack/echo/v4@latest)".into(),
                command: "go".into(),
                args: vec!["get".into(), "github.com/labstack/echo/v4@latest".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: Some(StepCondition::FileExists { path: go_mod_path }),
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ]
    }
}
