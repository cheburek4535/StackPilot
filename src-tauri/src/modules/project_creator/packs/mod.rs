use crate::modules::project_creator::models::*;
use serde::Deserialize;

// Реестр пакетов строится из тех же данных, что и мастер (wizard_tree.json):
// фреймворки → Feature-пакеты, инфраструктурные инструменты (БД, кэши,
// контейнеры, observability) → Infrastructure, средства разработки (тесты,
// сборка, линтеры) → Tooling. Это не выдуманные пакеты — каждый соответствует
// реально реализованной в движке технологии.

#[derive(Deserialize)]
struct WizardFramework {
    id: String,
    label: String,
    description: String,
    #[allow(dead_code)]
    side: Option<String>,
    #[allow(dead_code)]
    languages: Vec<String>,
    #[allow(dead_code)]
    knowledge_key: Option<String>,
    #[allow(dead_code)]
    output_subdir: Option<String>,
    #[allow(dead_code)]
    kind: Option<String>,
    #[allow(dead_code)]
    recommends: Vec<FrameworkRecommendation>,
    #[allow(dead_code)]
    required_tools: Vec<String>,
    #[allow(dead_code)]
    project_types: Vec<String>,
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
    languages: Vec<crate::modules::project_creator::knowledge::WizardLanguage>,
    frameworks: Vec<WizardFramework>,
    tools: Vec<WizardTool>,
}

fn load_tree() -> WizardTree {
    serde_json::from_str(include_str!("../knowledge/wizard_tree.json"))
        .expect("wizard_tree.json must parse (build-time invariant)")
}

fn infra_categories() -> &'static [&'static str] {
    &[
        "database",
        "cache",
        "queue",
        "broker",
        "search",
        "container",
        "observability",
        "proxy",
        "storage",
        "analytics",
        "streaming",
        "etl",
    ]
}

fn tooling_categories() -> &'static [&'static str] {
    &[
        "testing",
        "build",
        "linting",
        "formatting",
        "package-manager",
        "cli",
        "security",
    ]
}

#[allow(dead_code)]
pub trait PackRegistry: Send + Sync {
    fn list_packs(&self, kind: Option<PackKind>) -> Vec<PackInfo>;
    fn get_pack(&self, id: &str) -> Option<PackInfo>;
}

#[allow(dead_code)]
pub struct DefaultPackRegistry {
    packs: Vec<PackInfo>,
}

impl DefaultPackRegistry {
    pub fn new() -> Self {
        let tree = load_tree();
        let mut packs: Vec<PackInfo> = Vec::new();

        for fw in &tree.frameworks {
            let mut dependencies = fw.languages.clone();
            dependencies.extend(fw.required_tools.clone());
            dependencies.extend(fw.recommends.iter().map(|r| r.framework.clone()));
            packs.push(PackInfo {
                id: format!("framework:{}", fw.id),
                name: format!("{} (framework)", fw.label),
                description: fw.description.clone(),
                kind: PackKind::Feature,
                tags: vec!["framework".to_string(), fw.side.clone().unwrap_or_default()],
                dependencies,
                knowledge_key: Some(fw.id.clone()),
            });
        }

        for t in &tree.tools {
            let kind = if infra_categories().contains(&t.category.as_str()) {
                PackKind::Infrastructure
            } else if tooling_categories().contains(&t.category.as_str()) {
                PackKind::Tooling
            } else {
                PackKind::Infrastructure
            };
            let mut tags = vec![t.category.clone()];
            if t.requires_docker {
                tags.push("docker".to_string());
            }
            packs.push(PackInfo {
                id: format!("tool:{}", t.id),
                name: format!("{} (tool)", t.label),
                description: t.description.clone(),
                kind,
                tags,
                dependencies: t.requires.clone(),
                knowledge_key: Some(t.id.clone()),
            });
        }

        Self { packs }
    }
}

impl PackRegistry for DefaultPackRegistry {
    fn list_packs(&self, kind: Option<PackKind>) -> Vec<PackInfo> {
        match kind {
            Some(k) => self
                .packs
                .iter()
                .filter(|p| std::mem::discriminant(&p.kind) == std::mem::discriminant(&k))
                .cloned()
                .collect(),
            None => self.packs.clone(),
        }
    }

    fn get_pack(&self, id: &str) -> Option<PackInfo> {
        self.packs.iter().find(|p| p.id == id).cloned().or_else(|| {
            self.packs
                .iter()
                .find(|p| p.id.ends_with(&format!(":{}", id)))
                .cloned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_tree_parses_and_builds_packs() {
        let registry = DefaultPackRegistry::new();
        assert!(!registry.packs.is_empty());
        assert!(registry.get_pack("framework:tauri").is_some());
        assert!(registry.get_pack("tool:postgresql").is_some());
    }
}
