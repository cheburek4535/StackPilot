use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct ActixWebProvider;

impl RecipeProvider for ActixWebProvider {
    fn id(&self) -> &'static str {
        "actix-web"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![
            write_file(
                "actix_main",
                "Create Actix Web app",
                "src/main.rs",
                &format!(
                    r#"use actix_web::{{get, App, HttpServer, Responder, HttpResponse}};

#[get("/")]
async fn index() -> impl Responder {{
    HttpResponse::Ok().json(serde_json::json!({{"message": "Hello from {}!"}}))
}}

#[actix_web::main]
async fn main() -> std::io::Result<()> {{
    println!("Listening on http://127.0.0.1:8080");
    HttpServer::new(|| {{
        App::new().service(index)
    }})
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}}
"#,
                    project_name
                ),
            ),
            Step::Command {
                id: "add_actix_deps".to_string(),
                label: "Add Actix Web dependencies".to_string(),
                description: "Add actix-web + serde_json to Cargo.toml".to_string(),
                command: "cargo".to_string(),
                args: vec![
                    "add".to_string(),
                    "actix-web".to_string(),
                    "serde_json".to_string(),
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
