use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, StepCondition, WizardContext};

pub struct GinProvider;

impl RecipeProvider for GinProvider {
    fn id(&self) -> &'static str {
        "gin"
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
                "gin_main",
                "Create Gin entry",
                "cmd/main.go",
                &format!(
                    r#"package main

import (
    "net/http"
    "github.com/gin-gonic/gin"
)

func main() {{
    r := gin.Default()
    r.GET("/", func(c *gin.Context) {{
        c.JSON(http.StatusOK, gin.H{{"message": "Hello from {}!"}})
    }})
    r.Run(":8080")
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "get_gin".into(),
                label: "Install Gin".into(),
                description: "Add Gin dependency (modern go get pkg@latest)".into(),
                command: "go".into(),
                args: vec!["get".into(), "github.com/gin-gonic/gin@latest".into()],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: Some(StepCondition::FileExists { path: go_mod_path }),
                on_error: ErrorMode::Abort,
                interactive: vec![],
            },
        ]
    }
}
