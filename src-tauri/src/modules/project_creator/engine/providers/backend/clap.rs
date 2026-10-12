use crate::modules::project_creator::engine::providers::{cmd, write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct ClapProvider;

impl RecipeProvider for ClapProvider {
    fn id(&self) -> &'static str {
        "clap"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        // Легальная связка axum + clap: веб-сервер владеет src/main.rs,
        // CLI становится отдельным бинарником Cargo (src/bin/cli.rs).
        // Поодиночке clap занимает src/main.rs.
        let cli_path = if context.frameworks.iter().any(|f| f == "axum") {
            "src/bin/cli.rs"
        } else {
            "src/main.rs"
        };
        let cli_id = if cli_path == "src/main.rs" {
            "clap_main"
        } else {
            "clap_cli"
        };
        let cli_label = if cli_path == "src/main.rs" {
            "Create CLI entry point"
        } else {
            "Create CLI binary (src/bin/cli.rs)"
        };

        vec![
            cmd(
                "add_clap_deps",
                "Add Clap dependency",
                "Add clap with derive feature",
                "cargo",
                vec!["add", "clap", "--features", "derive"],
                project_path,
            ),
            write_file(
                cli_id,
                cli_label,
                cli_path,
                &format!(
                    r#"use clap::Parser;

#[derive(Parser)]
#[command(name = "{}", version = "0.1.0", about = "A CLI tool")]
struct Cli {{
    /// Optional name to greet
    name: Option<String>,
}}

fn main() {{
    let cli = Cli::parse();
    println!("Hello, {{}}!", cli.name.as_deref().unwrap_or("world"));
}}
"#,
                    project_name
                ),
            ),
        ]
    }
}
