use std::path::Path;
use std::sync::Arc;

use crate::modules::project_creator::models::*;
use crate::modules::project_creator::generators::GeneratorRegistry;

pub trait RecipeEngine: Send + Sync {
    fn resolve_recipe(
        &self,
        context: &WizardContext,
        analysis: Option<&AnalysisReport>,
    ) -> Result<Recipe, String>;

    fn dry_run(&self, recipe: &Recipe, context: &WizardContext) -> RecipePreview;

    fn execute(
        &self,
        recipe: &Recipe,
        context: &WizardContext,
        project_path: &Path,
        generators: Arc<GeneratorRegistry>,
    ) -> ExecutionResult;
}

pub struct DefaultRecipeEngine;

impl DefaultRecipeEngine {
    pub fn new() -> Self {
        Self
    }
}

impl RecipeEngine for DefaultRecipeEngine {
    fn resolve_recipe(
        &self,
        _context: &WizardContext,
        _analysis: Option<&AnalysisReport>,
    ) -> Result<Recipe, String> {
        Err("RecipeEngine not yet implemented — Milestone 4".to_string())
    }

    fn dry_run(&self, recipe: &Recipe, _context: &WizardContext) -> RecipePreview {
        RecipePreview {
            recipe_id: recipe.id.clone(),
            recipe_name: recipe.name.clone(),
            step_previews: Vec::new(),
            total_steps: 0,
            will_execute_count: 0,
            will_skip_count: 0,
        }
    }

    fn execute(
        &self,
        recipe: &Recipe,
        _context: &WizardContext,
        _project_path: &Path,
        _generators: Arc<GeneratorRegistry>,
    ) -> ExecutionResult {
        ExecutionResult {
            recipe_id: recipe.id.clone(),
            total_duration_ms: 0,
            step_results: Vec::new(),
            overall: OverallStatus::Aborted {
                last_step: None,
                reason: "RecipeEngine not yet implemented — Milestone 4".to_string(),
            },
        }
    }
}
