use std::path::Path;

use crate::modules::project_creator::models::*;

// NOTE: Этот модуль занимается общим анализом проектов (детектирование технологий,
// конфигов, поиск отсутствующих компонентов). В будущем он должен заменить
// часть детектирования из DevLauncher's FsProjectAnalyzer
// (см. src-tauri/src/modules/devlauncher/analyzer.rs).
//
// План рефакторинга:
//   1. Вынести общие детекторы в core/analysis/
//   2. Сделать DevLauncher'ский анализатор зависимым от результатов этого модуля
//   3. Оставить генерацию launch profile в DevLauncher как отдельную задачу

pub trait ProjectAnalyzer: Send + Sync {
    fn analyze(&self, path: &Path) -> Result<AnalysisReport, String>;
}

pub struct DefaultProjectAnalyzer;

impl DefaultProjectAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl ProjectAnalyzer for DefaultProjectAnalyzer {
    fn analyze(&self, path: &Path) -> Result<AnalysisReport, String> {
        Err(format!(
            "ProjectAnalyzer not yet implemented — Milestone 3. Path: {}",
            path.display()
        ))
    }
}
