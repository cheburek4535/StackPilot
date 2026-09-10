// ============================================================
// Рекомендации стека (recommend.rs)
// ============================================================
// Чисто data-driven: всё выводится из wizard_tree.json —
// project_tool_map, language_tool_map, framework_tool_map,
// kind="side", recommends. Никаких захардкоженных сценариев:
// новые комбинации появляются добавлением данных.

use super::models::{FrameworkDef, WizardTreeData};

/// Рекомендуемый фреймворк с пояснением «зачем».
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrameworkSuggestion {
    pub id: String,
    pub note: String,
}

/// Рекомендуемый инструмент с пояснением «зачем».
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSuggestion {
    pub id: String,
    pub note: String,
}

/// Полный набор рекомендаций для текущего выбора пользователя.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StackRecommendations {
    /// Главные фреймворки, подсвечиваемые парными рекомендациями
    /// (например nest → react). Пусто, если ничего не подходит.
    pub frameworks: Vec<FrameworkSuggestion>,
    /// Побочные фреймворки (kind="side": aiogram, telegraf...),
    /// совместимые с выбранными языками и типом проекта.
    pub side_frameworks: Vec<FrameworkSuggestion>,
    /// Инструменты из трёх карт: тип проекта, языки, фреймворки.
    pub tools: Vec<ToolSuggestion>,
    /// Языки, которые стоит подставить, чтобы выбранные фреймворки
    /// стали валидными (из recommended_language фреймворков).
    pub missing_languages: Vec<String>,
}

impl StackRecommendations {
    fn empty() -> Self {
        Self {
            frameworks: Vec::new(),
            side_frameworks: Vec::new(),
            tools: Vec::new(),
            missing_languages: Vec::new(),
        }
    }
}

fn side_langs<'a>(backend: &'a [String], frontend: &'a [String]) -> Vec<&'a str> {
    backend
        .iter()
        .map(String::as_str)
        .chain(frontend.iter().map(String::as_str))
        .collect()
}

/// Рекомендации для текущего выбора. Не валидирует стек — только
/// подсказывает; валидация остаётся в validate.rs.
pub fn recommend_stack(
    tree: &WizardTreeData,
    project_type: Option<&str>,
    backend_languages: &[String],
    frontend_languages: &[String],
    frameworks: &[String],
) -> StackRecommendations {
    let mut rec = StackRecommendations::empty();

    let selected: Vec<&FrameworkDef> = frameworks
        .iter()
        .filter_map(|id| tree.frameworks.iter().find(|f| &f.id == id))
        .collect();
    let langs = side_langs(backend_languages, frontend_languages);

    // 1. Парные рекомендации главных фреймворков (recommends выбранных)
    for fw in &selected {
        for r in &fw.recommends {
            rec.frameworks.push(FrameworkSuggestion {
                id: r.framework.clone(),
                note: if r.note.is_empty() {
                    fw.label.clone()
                } else {
                    r.note.clone()
                },
            });
        }
    }

    // 2. Побочные фреймворки: kind="side", язык совпадает, тип подходит
    for fw in &tree.frameworks {
        if fw.kind != "side" {
            continue;
        }
        let lang_ok = fw.languages.iter().any(|l| langs.contains(&l.as_str()));
        let type_ok = project_type.is_none_or(|pt| {
            fw.project_types.is_empty() || fw.project_types.iter().any(|p| p == pt)
        });
        if lang_ok && type_ok {
            rec.side_frameworks.push(FrameworkSuggestion {
                id: fw.id.clone(),
                note: fw.description.clone(),
            });
        }
    }

    // 3. Инструменты: тип проекта + языки + фреймворки (union, без дублей)
    let mut seen = std::collections::HashSet::new();
    let push_tool = |id: &str,
                         note: String,
                         rec: &mut StackRecommendations,
                         seen: &mut std::collections::HashSet<String>| {
        if seen.insert(id.to_string()) {
            rec.tools.push(ToolSuggestion {
                id: id.to_string(),
                note,
            });
        }
    };
    if let Some(pt) = project_type {
        if let Some(tools) = tree.project_tool_map.get(pt) {
            for t in tools {
                push_tool(
                    t,
                    format!("Рекомендовано для проекта «{pt}»"),
                    &mut rec,
                    &mut seen,
                );
            }
        }
    }
    for lang in &langs {
        if let Some(tools) = tree.language_tool_map.get(*lang) {
            for t in tools {
                push_tool(
                    t,
                    format!("Рекомендовано для языка «{lang}»"),
                    &mut rec,
                    &mut seen,
                );
            }
        }
    }
    for fw in &selected {
        if let Some(tools) = tree.framework_tool_map.get(&fw.id) {
            for t in tools {
                push_tool(
                    t,
                    format!("Хорошо сочетается с «{}»", fw.label),
                    &mut rec,
                    &mut seen,
                );
            }
        }
    }

    // 4. Недостающие языки: языки фреймворка не пересекаются с выбранными
    let mut seen_lang = std::collections::HashSet::new();
    for fw in &selected {
        if !fw.languages.iter().any(|l| langs.contains(&l.as_str()))
            && seen_lang.insert(fw.recommended_language.clone())
        {
            rec.missing_languages.push(fw.recommended_language.clone());
        }
    }

    rec
}
