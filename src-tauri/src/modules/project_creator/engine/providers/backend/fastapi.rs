use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct FastApiProvider;

impl RecipeProvider for FastApiProvider {
    fn id(&self) -> &'static str {
        "fastapi"
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
                "fastapi_main",
                "Create FastAPI entry point",
                "src/main.py",
                &format!(
                    r#"from fastapi import FastAPI

app = FastAPI(title="{}", version="0.1.0")

@app.get("/")
async def root():
    return {{"message": "Hello from {}!"}}

if __name__ == "__main__":
    import uvicorn
    uvicorn.run("main:app", host="0.0.0.0", port=8000, reload=True)
"#,
                    project_name, project_name
                ),
            )
        ]
    }
}
