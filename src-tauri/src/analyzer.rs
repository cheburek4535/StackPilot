// ============================================================
// ProjectAnalyzer — анализатор проектов.
//
// Анализатор изучает структуру папки проекта и предлагает
// LaunchProfile с предполагаемыми действиями.
//
// ВАЖНО: Анализатор НИКОГДА не запускает действия.
// Он только читает файлы и возвращает предложение.
//
// Текущая реализация — заглушка, всегда возвращает
// одинаковый профиль. Полная версия будет искать:
//   • docker-compose.yml → Start Docker
//   • package.json → Run npm dev
//   • Cargo.toml → Run cargo
//   • swagger/ → Open Swagger URL
// и т.д.
// ============================================================

use crate::models::*;

// --------------------------------------------------
// Трейт ProjectAnalyzer
// --------------------------------------------------
pub trait ProjectAnalyzer: Send + Sync {
    /// Проанализировать проект и предложить профиль
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String>;
}

// --------------------------------------------------
// SimpleAnalyzer — заглушка
//
// Возвращает пример профиля с одним действием.
// --------------------------------------------------
pub struct SimpleAnalyzer;

impl ProjectAnalyzer for SimpleAnalyzer {
    fn analyze(&self, project_path: &str) -> Result<LaunchProfile, String> {
        // TODO: настоящий анализ проекта
        Ok(LaunchProfile {
            name: "проанализированный проект".into(),
            description: format!("Автоматически найденный профиль для {}", project_path),
            project_path: Some(project_path.to_string()),
            actions: vec![
                LaunchAction {
                    id: "act_1".into(),
                    label: "Запустить Docker".into(),
                    enabled: true,
                    action_type: ActionType::RunCommand {
                        command: "docker compose up -d".into(),
                        working_dir: Some(project_path.to_string()),
                    },
                },
                LaunchAction {
                    id: "act_2".into(),
                    label: "Открыть проект в VS Code".into(),
                    enabled: true,
                    action_type: ActionType::OpenApplication {
                        path: "code".into(),
                        args: Some(project_path.to_string()),
                    },
                },
            ],
        })
    }
}
