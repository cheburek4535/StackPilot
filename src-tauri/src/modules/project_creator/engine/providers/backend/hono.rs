use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct HonoProvider;

impl RecipeProvider for HonoProvider {
    fn id(&self) -> &'static str {
        "hono"
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
                "hono_index",
                "Create Hono entry",
                "src/index.ts",
                &format!(
                    r#"import {{ serve }} from '@hono/node-server';
import {{ Hono }} from 'hono';

const app = new Hono();

app.get('/', (c) => {{
  return c.json({{ message: 'Hello from {}!' }});
}});

const port = 3000;
console.log(`Server is running on http://localhost:${{port}}`);

serve({{
  fetch: app.fetch,
  port,
}});
"#,
                    project_name
                ),
            ),
            write_file(
                "hono_package",
                "Hono dependencies",
                "package.json",
                &format!(
                    r#"{{
  "name": "{}",
  "version": "1.0.0",
  "type": "module",
  "scripts": {{
    "dev": "tsx watch src/index.ts",
    "build": "tsc",
    "start": "node dist/index.js"
  }},
  "dependencies": {{
    "@hono/node-server": "^1.13.0",
    "hono": "^4.6.0"
  }},
  "devDependencies": {{
    "@types/node": "^20.11.0",
    "tsx": "^4.7.0",
    "typescript": "^5.3.0"
  }}
}}
"#,
                    project_name
                ),
            ),
            write_file(
                "hono_tsconfig",
                "Hono TypeScript config",
                "tsconfig.json",
                r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "skipLibCheck": true,
    "outDir": "dist"
  },
  "include": ["src/**/*"]
}
"#,
            ),
            preflight::package_json_check_step(
                "hono_pkg_check",
                "Validate Hono package.json",
                &pkg_path,
                &["hono", "@hono/node-server"],
            ),
        ]
    }
}
