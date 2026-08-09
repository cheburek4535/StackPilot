pub mod analysis;
pub mod commands;
pub mod engine;
pub mod generators;
pub mod knowledge;
pub mod models;
pub mod packs;
pub mod recommend;
pub mod validate;
pub mod wizard;

use std::sync::Arc;

use analysis::ProjectAnalyzer;
use engine::RecipeEngine;
use generators::GeneratorRegistry;
use knowledge::KnowledgeBase;
use packs::PackRegistry;
use wizard::WizardEngine;

pub struct ProjectCreatorState {
    pub wizard: WizardEngine,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
    pub engine: Arc<dyn RecipeEngine>,
    pub generators: Arc<GeneratorRegistry>,
    pub packs: Arc<dyn PackRegistry>,
    pub knowledge: Arc<dyn KnowledgeBase>,
}

impl ProjectCreatorState {
    pub fn new(
        analyzer: Arc<dyn ProjectAnalyzer>,
        engine: Arc<dyn RecipeEngine>,
        generators: Arc<GeneratorRegistry>,
        packs: Arc<dyn PackRegistry>,
        knowledge: Arc<dyn KnowledgeBase>,
    ) -> Self {
        Self {
            wizard: WizardEngine::new(),
            analyzer,
            engine,
            generators,
            packs,
            knowledge,
        }
    }
}
