use crate::modules::project_creator::engine::providers::RecipeProvider;
use crate::modules::project_creator::engine::{
    qt_steps_kirigami, qt_steps_qml, qt_steps_webengine, qt_steps_widgets, qt_ui_mode,
};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct QtProvider;

impl RecipeProvider for QtProvider {
    fn id(&self) -> &'static str {
        "qt"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let mode = qt_ui_mode(context);
        match mode {
            "qml" => qt_steps_qml(project_name),
            "kirigami" => qt_steps_kirigami(project_name),
            "webengine" => qt_steps_webengine(project_name, context, segment),
            _ => qt_steps_widgets(project_name),
        }
    }
}

// Варианты UI Qt — генерируются внутри блока "qt" (см. qt_ui_mode);
// отдельные шаги не нужны, чтобы не дублировать файлы.
pub struct QtQmlProvider;

impl RecipeProvider for QtQmlProvider {
    fn id(&self) -> &'static str {
        "qt-qml"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![]
    }
}

pub struct QtWidgetsProvider;

impl RecipeProvider for QtWidgetsProvider {
    fn id(&self) -> &'static str {
        "qt-widgets"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![]
    }
}

pub struct QtWebengineProvider;

impl RecipeProvider for QtWebengineProvider {
    fn id(&self) -> &'static str {
        "qt-webengine"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![]
    }
}

pub struct QtKirigamiProvider;

impl RecipeProvider for QtKirigamiProvider {
    fn id(&self) -> &'static str {
        "qt-kirigami"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        _project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![]
    }
}
