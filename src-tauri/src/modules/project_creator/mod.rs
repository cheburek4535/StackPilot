pub mod analysis;
pub mod commands;
pub mod engine;
pub mod generators;
pub mod knowledge;
pub mod models;
pub mod normalize;
pub mod packs;
pub mod recommend;
pub mod validate;
pub mod wizard;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use analysis::ProjectAnalyzer;
use engine::RecipeEngine;
use generators::GeneratorRegistry;
use knowledge::KnowledgeBase;
use models::ExecutionEvent;
use packs::PackRegistry;
use wizard::WizardEngine;

/// Сколько последних событий выполнения хранить для восстановления UI
/// (когда пользователь переключил вкладку и вернулся). 20_000 событий
/// покрывает типичную генерацию проекта и держит память в разумных границах.
pub const EXECUTION_SNAPSHOT_LIMIT: usize = 20_000;

pub struct ProjectCreatorState {
    pub wizard: WizardEngine,
    pub analyzer: Arc<dyn ProjectAnalyzer>,
    pub engine: Arc<dyn RecipeEngine>,
    pub generators: Arc<GeneratorRegistry>,
    pub packs: Arc<dyn PackRegistry>,
    pub knowledge: Arc<dyn KnowledgeBase>,
    /// Буфер событий текущего выполнения (для `project_execution_snapshot`).
    pub execution_events: Arc<Mutex<Vec<ExecutionEvent>>>,
    /// true, пока выполнение проекта активно (tokio-задача не завершилась).
    pub execution_running: Arc<AtomicBool>,
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
            execution_events: Arc::new(Mutex::new(Vec::new())),
            execution_running: Arc::new(AtomicBool::new(false)),
        }
    }
}
