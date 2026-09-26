use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct FastifyProvider;

impl RecipeProvider for FastifyProvider {
    fn id(&self) -> &'static str {
        "fastify"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let pkg_path = match segment {
            Some(dir) => format!("{}/package.json", dir),
            None => "package.json".to_string(),
        };
        vec![
            write_file(
                "fastify_index",
                "Create Fastify entry",
                "src/index.js",
                &format!(
                    r#"const fastify = require('fastify')({{ logger: true }});

fastify.get('/', async () => {{
    return {{ message: 'Hello from {}!' }};
}});

const start = async () => {{
    try {{
        await fastify.listen({{ port: 3000 }});
        console.log('Server running on http://localhost:3000');
    }} catch (err) {{
        fastify.log.error(err);
        process.exit(1);
    }}
}};
start();
"#,
                    project_name
                ),
            ),
            write_file(
                "fastify_package",
                "Fastify package.json",
                "package.json",
                &format!(
                    r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/index.js",
  "scripts": {{
    "start": "node src/index.js",
    "dev": "node --watch src/index.js"
  }},
  "dependencies": {{
    "fastify": "^4.28.0"
  }}
}}
"#,
                    project_name
                ),
            ),
            preflight::package_json_check_step(
                "fastify_pkg_check",
                "Validate Fastify package.json",
                &pkg_path,
                &["fastify"],
            ),
        ]
    }
}
