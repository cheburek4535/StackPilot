use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, StepCondition, WizardContext};

pub struct FiberProvider;

impl RecipeProvider for FiberProvider {
    fn id(&self) -> &'static str {
        "fiber"
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
                "fiber_main",
                "Create Fiber entry",
                "cmd/main.go",
                &format!(
                    r#"package main

import (
    "github.com/gofiber/fiber/v2"
)

func main() {{
    app := fiber.New()

    app.Get("/", func(c *fiber.Ctx) error {{
        return c.JSON(fiber.Map{{"message": "Hello from {}!"}})
    }})

    app.Listen(":3000")
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "get_fiber".into(),
                label: "Install Fiber".into(),
                description: "Add Fiber dependency (go get github.com/gofiber/fiber/v2@latest)".into(),
                command: "go".into(),
                args: vec!["get".into(), "github.com/gofiber/fiber/v2@latest".into()],
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
