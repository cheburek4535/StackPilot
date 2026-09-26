use crate::modules::project_creator::engine::providers::{cmd, RecipeProvider};
use crate::modules::project_creator::engine::python_venv_bin;
use crate::modules::project_creator::models::{ErrorMode, Step, WizardContext};

pub struct DjangoProvider;

impl RecipeProvider for DjangoProvider {
    fn id(&self) -> &'static str {
        "django"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        // django-admin startproject требует валидный Python-идентификатор:
        // «my-project» (дефис) не подходит — заменяем на подчёркивание.
        let safe_name = project_name.replace('-', "_");
        // CLI запускается из КАНОНИЧЕСКОГО venv проекта (никакого
        // глобального django-admin и никакого отдельного venv-цикла):
        // каноническое окружение уже создано и манифест установлен
        // (django_start ← py_pip_install в декларациях зависимостей).
        let django_command = if context.languages.iter().any(|l| l == "python") {
            python_venv_bin(project_path, segment.unwrap_or("."), "django-admin")
        } else {
            "django-admin".to_string()
        };
        let mut start = cmd(
            "django_start",
            "Start Django project",
            "Create Django project structure (django-admin from the project venv)",
            &django_command,
            vec!["startproject", &safe_name, "."],
            project_path,
        );
        if let Step::Command { on_error, .. } = &mut start {
            // Обязательный CLI каркаса: провал останавливает пайплайн.
            *on_error = ErrorMode::Abort;
        }
        vec![start]
    }
}
