use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct AxumProvider;

impl RecipeProvider for AxumProvider {
    fn id(&self) -> &'static str {
        "axum"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![
            Step::WriteFile {
                id: "axum_main".to_string(),
                label: "Create Axum entry point".to_string(),
                description: "Create src/main.rs".to_string(),
                path: "src/main.rs".to_string(),
                content: format!(
                    r#"use axum::{{routing::get, Router}};

#[tokio::main]
async fn main() {{
    let app = Router::new().route("/", get(|| async {{ "Hello from {}!" }}));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}}
"#,
                    project_name
                ),
                overwrite: false,
                policy: None,
                condition: None,
                on_error: ErrorMode::Skip,
            },
            Step::Command {
                id: "add_axum_deps".to_string(),
                label: "Add Axum dependencies".to_string(),
                description: "Add axum + tokio to Cargo.toml".to_string(),
                command: "cargo".to_string(),
                args: vec![
                    "add".to_string(),
                    "axum".to_string(),
                    "tokio".to_string(),
                    "--features".to_string(),
                    "tokio/full".to_string(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(300),
                condition: None,
                on_error: ErrorMode::Skip,
                interactive: vec![],
            },
        ]
    }
}
