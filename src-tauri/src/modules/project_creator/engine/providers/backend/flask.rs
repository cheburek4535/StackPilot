use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct FlaskProvider;

impl RecipeProvider for FlaskProvider {
    fn id(&self) -> &'static str {
        "flask"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![
            write_file(
                "flask_app",
                "Create Flask app",
                "src/app.py",
                &format!(
                    r#"from flask import Flask

app = Flask(__name__)

@app.route("/")
def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=5000, debug=True)
"#,
                    project_name
                ),
            ),
            // flask попадает в union requirements.txt python-скаффолда
        ]
    }
}
