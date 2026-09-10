use crate::modules::project_creator::models::*;
use serde::Deserialize;

// Реальная база знаний собирается из wizard_tree.json (того же файла, из
// которого мастер читает фреймворки/языки/инструменты): каждая запись — это
// факт о реально поддерживаемой движком технологии, а не заглушка.

#[derive(Deserialize)]
pub(crate) struct WizardLanguage {
    #[allow(dead_code)]
    pub(crate) id: String,
    #[allow(dead_code)]
    pub(crate) label: String,
}

#[derive(Deserialize)]
struct WizardFramework {
    id: String,
    label: String,
    description: String,
    #[allow(dead_code)]
    side: Option<String>,
    languages: Vec<String>,
    #[allow(dead_code)]
    knowledge_key: Option<String>,
    #[allow(dead_code)]
    output_subdir: Option<String>,
    #[allow(dead_code)]
    kind: Option<String>,
    #[allow(dead_code)]
    recommends: Vec<FrameworkRecommendation>,
}

#[derive(Deserialize)]
struct WizardTool {
    id: String,
    label: String,
    description: String,
    category: String,
    #[allow(dead_code)]
    requires_docker: bool,
    #[allow(dead_code)]
    requires: Vec<String>,
}

#[derive(Deserialize)]
struct WizardTree {
    #[allow(dead_code)]
    languages: Vec<WizardLanguage>,
    frameworks: Vec<WizardFramework>,
    tools: Vec<WizardTool>,
}

fn load_tree() -> WizardTree {
    serde_json::from_str(include_str!("wizard_tree.json"))
        .expect("wizard_tree.json must parse (build-time invariant)")
}

#[allow(dead_code)]
pub trait KnowledgeBase: Send + Sync {
    fn get_entry(&self, key: &str) -> Option<KnowledgeEntry>;
    fn search(&self, query: &str) -> Vec<KnowledgeEntry>;
}

#[allow(dead_code)]
pub struct DefaultKnowledgeBase {
    entries: Vec<KnowledgeEntry>,
}

impl DefaultKnowledgeBase {
    pub fn new() -> Self {
        // Записи строятся один раз при старте из данных мастера.
        let tree = load_tree();
        let mut entries: Vec<KnowledgeEntry> = Vec::new();

        for fw in &tree.frameworks {
            let langs = fw.languages.join(", ");
            let mut content = format!("{}\n\nПоддерживаемые языки: {}\n", fw.description, langs);
            if let Some(side) = &fw.side {
                content.push_str(&format!("Сторона проекта: {}\n", side));
            }
            if let Some(dir) = &fw.output_subdir {
                content.push_str(&format!("Каталог развёртывания: {}\n", dir));
            }
            entries.push(KnowledgeEntry {
                key: format!("framework:{}", fw.id),
                title: format!("{} ({})", fw.label, fw.id),
                content,
                source: Some("wizard_tree.json".to_string()),
            });
        }

        for t in &tree.tools {
            let mut content = format!("{}\n", t.description);
            if t.requires_docker {
                content.push_str("Требуется Docker: да (разворачивается как контейнер проекта)\n");
            }
            content.push_str(&format!("Категория: {}\n", t.category));
            entries.push(KnowledgeEntry {
                key: format!("tool:{}", t.id),
                title: format!("{} ({})", t.label, t.id),
                content,
                source: Some("wizard_tree.json".to_string()),
            });
        }

        Self { entries }
    }
}

impl KnowledgeBase for DefaultKnowledgeBase {
    fn get_entry(&self, key: &str) -> Option<KnowledgeEntry> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .cloned()
            // Приемлемый fallback по суффиксу ключа (framework:foo ↔ foo).
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.key.ends_with(&format!(":{}", key)))
                    .cloned()
            })
    }

    fn search(&self, query: &str) -> Vec<KnowledgeEntry> {
        let q = query.to_lowercase();
        let mut found: Vec<&KnowledgeEntry> = self
            .entries
            .iter()
            .filter(|e| {
                e.title.to_lowercase().contains(&q)
                    || e.key.to_lowercase().contains(&q)
                    || e.content.to_lowercase().contains(&q)
            })
            .collect();
        found.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        found.into_iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_tree_parses_and_builds_entries() {
        let kb = DefaultKnowledgeBase::new();
        assert!(!kb.entries.is_empty());
        assert!(kb.get_entry("framework:tauri").is_some());
        assert!(kb.get_entry("tool:postgresql").is_some());
    }
}
