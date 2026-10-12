use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct ExpressProvider;

impl RecipeProvider for ExpressProvider {
    fn id(&self) -> &'static str {
        "express"
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
                "express_index",
                "Create Express entry",
                "src/index.js",
                &format!(
                    r#"const express = require('express');
const app = express();
const PORT = process.env.PORT || 3000;

app.get('/', (req, res) => {{
    res.json({{ message: 'Hello from {}!' }});
}});

app.listen(PORT, () => {{
    console.log(`Server running on http://localhost:${{PORT}}`);
}});
"#,
                    project_name
                ),
            ),
            write_file(
                "express_package",
                "Express dependencies",
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
    "express": "^4.18.2"
  }}
}}
"#,
                    project_name
                ),
            ),
            preflight::package_json_check_step(
                "express_pkg_check",
                "Validate Express package.json",
                &pkg_path,
                &["express"],
            ),
        ]
    }
}
