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

#[cfg(test)]
mod tests {
    mod metadata_tests;
    mod regression_tests;
    mod validation_tests;
}

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

/// Лимит подсчёта файлов сгенерированного проекта на диске. 500k файлов —
/// это заметно больше даже плотного node_modules; подсчёт до этого лимита
/// занимает миллисекунды, а «хвост» сверх лимита показывается как N+.
pub const FILE_COUNT_LIMIT: u64 = 500_000;

/// Очень быстрый подсчёт всех файлов в директории (включая node_modules и
/// прочие сторонние артефакты): итеративный обход через std::fs::read_dir
/// без рекурсии, без follow-symlink (защита от циклов и повторного счёта).
/// Ошибки чтения отдельных каталогов/записей пропускаются — даже потеряв
/// часть дерева, мы вернём точную нижнюю границу реального числа файлов.
pub fn count_project_files_on_disk(path: &std::path::Path, limit: u64) -> models::ProjectFileCount {
    let mut count: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(entry.path()),
                Ok(ft) if ft.is_file() => {
                    count += 1;
                    if count >= limit {
                        return models::ProjectFileCount {
                            count,
                            capped: true,
                            limit,
                        };
                    }
                }
                _ => {}
            }
        }
    }
    models::ProjectFileCount {
        count,
        capped: false,
        limit,
    }
}

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
