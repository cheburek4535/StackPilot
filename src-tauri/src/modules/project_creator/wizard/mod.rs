use crate::modules::project_creator::models::*;

/// WizardEngine — управляет мастером создания проекта.
///
/// Данные загружаются из knowledge/wizard_tree.json.
///
/// Шаги мастера (current_step):
///   0 — Project Type    (что создаём?)
///   1 — Language(s)     (на чём пишем?)
///   2 — Framework(s)    (какой фреймворк?)
///   3 — Tools           (какие инструменты?)
///   4 — Confirm         (сводка + подтверждение)
pub struct WizardEngine {
    tree: WizardTreeData,
}

impl WizardEngine {
    pub fn new() -> Self {
        let raw = include_str!("../knowledge/wizard_tree.json");
        let tree: WizardTreeData =
            serde_json::from_str(raw).expect("Failed to parse wizard_tree.json");
        Self { tree }
    }

    /// Возвращает полное дерево мастера (фронтенд использует для навигации)
    pub fn get_wizard_tree(&self) -> &WizardTreeData {
        &self.tree
    }

    /// Возвращает типы проектов (первый шаг мастера)
    pub fn get_project_types(&self) -> &[ProjectTypeDef] {
        &self.tree.project_types
    }

    /// Возвращает языки, доступные для указанного типа проекта
    pub fn get_languages_for(&self, project_type: &str) -> Vec<&LanguageDef> {
        self.tree
            .project_language_map
            .get(project_type)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.tree.languages.iter().find(|l| l.id == *id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Возвращает фреймворки, доступные для указанного языка
    pub fn get_frameworks_for(&self, language: &str) -> Vec<&FrameworkDef> {
        self.tree
            .language_framework_map
            .get(language)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.tree.frameworks.iter().find(|f| f.id == *id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Возвращает инструменты, доступные для указанного фреймворка
    pub fn get_tools_for(&self, framework: &str) -> Vec<&ToolDef> {
        self.tree
            .framework_tool_map
            .get(framework)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.tree.tools.iter().find(|t| t.id == *id))
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- Управление сессией ----

    pub fn start_session(&self, project_path: Option<String>) -> WizardSession {
        WizardSession {
            current_step: 0,
            total_steps: 5,
            context: WizardContext {
                project_path: project_path.map(std::path::PathBuf::from),
                ..Default::default()
            },
            questions: Vec::new(),
            is_complete: false,
        }
    }

    pub fn submit_answer(
        &self,
        session: &WizardSession,
        question_id: &str,
        answers: Vec<String>,
    ) -> WizardSession {
        let mut ctx = session.context.clone();

        match question_id {
            "project_type" => {
                ctx.project_type = answers.first().cloned();
                ctx.is_existing = ctx.project_path.is_some();
            }
            "languages" => {
                ctx.languages = answers;
            }
            "frameworks" => {
                ctx.frameworks = answers;
            }
            "tools" => {
                ctx.docker = answers.contains(&"docker".to_string());
                ctx.tools = answers;
            }
            "features" => {
                ctx.features = answers;
            }
            "confirm" => {}
            "__back__" => {
                return WizardSession {
                    current_step: (session.current_step.saturating_sub(1)).max(0),
                    ..session.clone()
                };
            }
            _ => {}
        }

        let next_step = if question_id == "confirm" {
            session.current_step
        } else {
            session.current_step + 1
        };

        let is_complete = question_id == "confirm";

        WizardSession {
            current_step: next_step,
            total_steps: 5,
            context: ctx,
            questions: Vec::new(),
            is_complete,
        }
    }
}
