//! Тесты целостности метаданных wizard_tree.json
//!
//! Проверяют, что все ссылки в данных (requires, alternatives, tool_warnings,
//! tool_conflicts) указывают на реально существующие сущности, а ключевые
//! инструменты размечены ожидаемыми полями (responsibility, requires).
//! Служат первым барьером против «конфигурационных ошибок» в JSON.

use crate::modules::project_creator::models::WizardTreeData;
use std::collections::HashSet;

/// Загрузка реального wizard_tree.json (не hardcoded копия данных).
fn load_test_tree() -> WizardTreeData {
    let json_str = include_str!("../knowledge/wizard_tree.json");
    serde_json::from_str(json_str).expect("Failed to parse wizard_tree.json")
}

#[test]
fn test_tool_requires_valid_ids() {
    let tree = load_test_tree();
    let tool_ids: HashSet<&str> = tree.tools.iter().map(|t| t.id.as_str()).collect();

    for tool in &tree.tools {
        for required_id in &tool.requires {
            assert!(
                tool_ids.contains(required_id.as_str()),
                "Tool '{}' requires '{}', but it doesn't exist in wizard_tree.json",
                tool.id,
                required_id
            );
        }
    }
}

#[test]
fn test_tool_alternatives_valid_ids() {
    let tree = load_test_tree();
    let tool_ids: HashSet<&str> = tree.tools.iter().map(|t| t.id.as_str()).collect();
    let fw_ids: HashSet<&str> = tree.frameworks.iter().map(|f| f.id.as_str()).collect();

    for tool in &tree.tools {
        for alt_id in &tool.alternatives {
            assert!(
                tool_ids.contains(alt_id.as_str()) || fw_ids.contains(alt_id.as_str()),
                "Tool '{}' lists '{}' as alternative, but it doesn't exist",
                tool.id,
                alt_id
            );
        }
    }
}

#[test]
fn test_tool_conflicts_valid_ids() {
    let tree = load_test_tree();
    let tool_ids: HashSet<&str> = tree.tools.iter().map(|t| t.id.as_str()).collect();

    for tool in &tree.tools {
        for conflict_id in &tool.conflicts {
            assert!(
                tool_ids.contains(conflict_id.as_str()),
                "Tool '{}' conflicts with '{}', but it doesn't exist",
                tool.id,
                conflict_id
            );
        }
    }
}

#[test]
fn test_framework_tool_warnings_valid_ids() {
    let tree = load_test_tree();
    let tool_ids: HashSet<&str> = tree.tools.iter().map(|t| t.id.as_str()).collect();

    for framework in &tree.frameworks {
        for tool_id in framework.tool_warnings.keys() {
            assert!(
                tool_ids.contains(tool_id.as_str()),
                "Framework '{}' has warning for tool '{}', but it doesn't exist",
                framework.id,
                tool_id
            );
        }
    }
}

#[test]
fn test_framework_tool_conflicts_valid_ids() {
    let tree = load_test_tree();
    let tool_ids: HashSet<&str> = tree.tools.iter().map(|t| t.id.as_str()).collect();

    for framework in &tree.frameworks {
        for tool_id in &framework.tool_conflicts {
            assert!(
                tool_ids.contains(tool_id.as_str()),
                "Framework '{}' conflicts with tool '{}', but it doesn't exist",
                framework.id,
                tool_id
            );
        }
    }
}

#[test]
fn test_orm_tools_have_responsibility() {
    let tree = load_test_tree();
    let orm_tools = ["sqlalchemy", "prisma", "drizzle"];

    for orm_id in orm_tools {
        let tool = tree.tools.iter().find(|t| t.id == orm_id);
        if let Some(tool) = tool {
            assert_eq!(
                tool.responsibility.as_deref(),
                Some("orm"),
                "ORM tool '{}' should have responsibility='orm'",
                orm_id
            );
        }
    }
}

#[test]
fn test_alembic_requires_sqlalchemy() {
    let tree = load_test_tree();
    let alembic = tree.tools.iter().find(|t| t.id == "alembic");

    if let Some(alembic) = alembic {
        assert!(
            alembic.requires.contains(&"sqlalchemy".to_string()),
            "Alembic must require SQLAlchemy"
        );
    }
}
