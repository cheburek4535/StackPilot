use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct SpringBootProvider;

impl RecipeProvider for SpringBootProvider {
    fn id(&self) -> &'static str {
        "spring-boot"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        let mut deps: Vec<&str> = vec!["web"];
        for tool in &context.tools {
            match tool.as_str() {
                "mongodb" => deps.push("data-mongodb"),
                "postgresql" => {
                    deps.push("data-jpa");
                    deps.push("postgresql");
                }
                "mysql" => {
                    deps.push("data-jpa");
                    deps.push("mysql");
                }
                "redis" => deps.push("data-redis"),
                _ => {}
            }
        }
        let deps_str = deps.join(",");
        vec![Step::Generate {
            id: "spring_init".into(),
            label: "Generate Spring Boot project".into(),
            description:
                "Download Spring Boot starter from Initializr (validates HTTP response)".into(),
            generator_id: "spring-boot".into(),
            generator_config: serde_json::json!({
                "project_name": project_name,
                "dependencies": deps_str,
            }),
            policy: None,
            condition: None,
            on_error: ErrorMode::Abort,
        }]
    }
}
