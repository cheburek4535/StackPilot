//! Регрессионные тесты известных граничных случаев и исправленных багов
//!
//! Покрывают: пустой список инструментов, неизвестные id инструментов,
//! инструменты без responsibility, а также обратную совместимость JSON без
//! новых полей (Session 2+ должны применяться дефолты).

use crate::modules::project_creator::models::WizardTreeData;
use crate::modules::project_creator::validate::{validate_stack, StackSeverity};

fn load_test_tree() -> WizardTreeData {
    let json_str = include_str!("../knowledge/wizard_tree.json");
    serde_json::from_str(json_str).unwrap()
}

#[test]
fn test_empty_tools_array() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["django".to_string()],
        &[], // Empty tools
        "linux",
    );

    // Should not crash; existing framework validation still works.
    // django + python on backend is a valid stack — no errors expected.
    let has_errors = issues
        .iter()
        .any(|issue| matches!(issue.severity, StackSeverity::Error));
    assert!(
        !has_errors,
        "Django + Python should validate cleanly with no tools: {:?}",
        issues
    );
}

#[test]
fn test_unknown_tool_id() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["django".to_string()],
        &["nonexistent_tool_id".to_string()],
        "linux",
    );

    // Should not crash; unknown IDs are skipped silently and produce no
    // tool-related issues (Django + Python itself is a valid stack).
    let has_tool_issue = issues.iter().any(|issue| {
        issue
            .message_key
            .as_deref()
            .is_some_and(|k| k.starts_with("stack.tool."))
    });
    assert!(
        !has_tool_issue,
        "Unknown tool id should be ignored, got issues: {:?}",
        issues
    );
}

#[test]
fn test_tool_without_responsibility() {
    let tree = load_test_tree();

    // Find a tool without responsibility (e.g., redis, docker).
    let tool = tree.tools.iter().find(|t| t.responsibility.is_none());

    if let Some(tool) = tool {
        let issues = validate_stack(
            &tree,
            None,
            &["python".to_string()],
            &["python".to_string()],
            &[],
            &[],
            &[tool.id.clone()],
            "linux",
        );

        // Should not crash; a lone tool without responsibility produces
        // no overlap issues.
        let has_overlap = issues.iter().any(|issue| {
            matches!(
                issue.message_key.as_deref(),
                Some("stack.tool.exclusive_alternatives" | "stack.tool.overlapping_responsibility")
            )
        });
        assert!(
            !has_overlap,
            "Tool without responsibility should not trigger overlap logic: {:?}",
            issues
        );
    }
}

#[test]
fn test_backward_compatibility_old_json() {
    // Old wizard_tree.json without new fields still parses. Only required
    // (non-defaulted) top-level fields are present; Session 2+ fields
    // (responsibility, alternatives, provides_capabilities) are absent and
    // must fall back to defaults.
    let minimal_json = r#"{
        "languages": [],
        "frameworks": [{
            "id": "test_fw",
            "label": "Test",
            "description": "test",
            "icon": "test.svg",
            "conflicts": [],
            "languages": ["python"],
            "side": "backend",
            "kind": "app"
        }],
        "tools": [{
            "id": "test_tool",
            "label": "Test Tool",
            "description": "test",
            "icon": "test.svg",
            "category": "other",
            "knowledge_key": null,
            "requires_docker": false,
            "requires": [],
            "conflicts": [],
            "platforms": [],
            "for_languages": [],
            "for_project_types": []
        }],
        "project_types": [],
        "allowed_main_pairs": [],
        "warning_pairs": [],
        "project_language_map": {},
        "language_framework_map": {},
        "framework_tool_map": {}
    }"#;

    let tree: WizardTreeData = serde_json::from_str(minimal_json).unwrap();

    // Verify defaults applied
    assert_eq!(tree.tools[0].responsibility, None);
    assert_eq!(tree.tools[0].alternatives.len(), 0);
    assert_eq!(tree.frameworks[0].provides_capabilities.len(), 0);
}
