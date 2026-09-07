//! Интеграционные тесты логики валидации стека (validate.rs)
//!
//! Проверяют поведение validate_stack на реальных данных wizard_tree.json:
//! ошибки зависимостей, языковых несовпадений, предупреждения о
//! пересечении ответственностей и чистые стеки без ошибок.

use crate::modules::project_creator::models::WizardTreeData;
use crate::modules::project_creator::validate::{validate_stack, StackSeverity};

fn load_test_tree() -> WizardTreeData {
    let json_str = include_str!("../knowledge/wizard_tree.json");
    serde_json::from_str(json_str).unwrap()
}

#[test]
fn test_alembic_without_sqlalchemy_errors() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["fastapi".to_string()],
        &["alembic".to_string()], // Missing sqlalchemy
        "linux",
    );

    // Should have at least one error about missing dependency
    let has_dependency_error = issues.iter().any(|issue| {
        matches!(issue.severity, StackSeverity::Error)
            && issue.message.to_lowercase().contains("sqlalchemy")
    });

    assert!(
        has_dependency_error,
        "Should error when Alembic selected without SQLAlchemy"
    );
}

#[test]
fn test_alembic_with_sqlalchemy_succeeds() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["fastapi".to_string()],
        &["sqlalchemy".to_string(), "alembic".to_string()],
        "linux",
    );

    // Should not have dependency errors
    let has_dependency_error = issues.iter().any(|issue| {
        matches!(issue.severity, StackSeverity::Error)
            && issue.message_key.as_deref() == Some("stack.tool.missing_dependency")
    });

    assert!(
        !has_dependency_error,
        "Should not error when Alembic selected with SQLAlchemy"
    );
}

#[test]
fn test_npm_without_javascript_errors() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()], // No JS/TS
        &["python".to_string()],
        &[],
        &["fastapi".to_string()],
        &["npm".to_string()],
        "linux",
    );

    let has_language_error = issues.iter().any(|issue| {
        matches!(issue.severity, StackSeverity::Error)
            && issue.message_key.as_deref() == Some("stack.tool.language_mismatch")
    });

    assert!(
        has_language_error,
        "Should error when npm selected without JavaScript/TypeScript"
    );
}

#[test]
fn test_django_sqlalchemy_warns() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["django".to_string()],
        &["sqlalchemy".to_string()],
        "linux",
    );

    // Should have warning (not error) about Django ORM overlap
    let has_warning = issues.iter().any(|issue| {
        matches!(issue.severity, StackSeverity::Warning)
            && (issue.message.to_lowercase().contains("sqlalchemy")
                || issue.message.to_lowercase().contains("orm"))
    });

    assert!(
        has_warning,
        "Should warn when Django selected with SQLAlchemy (ORM overlap)"
    );
}

#[test]
fn test_clean_stack_no_errors() {
    let tree = load_test_tree();
    let issues = validate_stack(
        &tree,
        None,
        &["python".to_string()],
        &["python".to_string()],
        &[],
        &["fastapi".to_string()],
        &[
            "sqlalchemy".to_string(),
            "alembic".to_string(),
            "postgresql".to_string(),
        ],
        "linux",
    );

    let has_errors = issues
        .iter()
        .any(|issue| matches!(issue.severity, StackSeverity::Error));

    assert!(
        !has_errors,
        "Clean stack (FastAPI + SQLAlchemy + Alembic + PostgreSQL) should not have errors: {:?}",
        issues
    );
}
