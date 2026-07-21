use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================
// Wizard Tree — гибкое дерево решений
// ============================================================

/// Полный набор данных для мастера (загружается из JSON)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardTreeData {
    pub project_types: Vec<ProjectTypeDef>,
    pub languages: Vec<LanguageDef>,
    pub frameworks: Vec<FrameworkDef>,
    pub tools: Vec<ToolDef>,
    pub project_language_map: std::collections::HashMap<String, Vec<String>>,
    pub language_framework_map: std::collections::HashMap<String, Vec<String>>,
    pub framework_tool_map: std::collections::HashMap<String, Vec<String>>,
}

/// Тип проекта (REST API, Desktop App, CLI Tool...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTypeDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub tags: Vec<String>,
    pub allow_custom_stack: bool,
}

/// Язык программирования
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageDef {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub knowledge_key: Option<String>,
}

/// Фреймворк / библиотека
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub knowledge_key: Option<String>,
}

/// Инструмент (БД, кеш, CI, тесты...)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<String>,
    pub category: String,
    pub knowledge_key: Option<String>,
}

// Устаревшие типы — будут удалены после миграции
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub tags: Vec<String>,
    pub children: Vec<SubCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubCategory {
    pub id: String,
    pub label: String,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub tags: Vec<String>,
    pub knowledge_key: Option<String>,
}

// ============================================================
// Wizard — дерево решений
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardQuestion {
    pub id: String,
    pub label: String,
    pub description: String,
    pub question_type: QuestionType,
    pub condition: Option<WizardCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuestionType {
    SingleChoice {
        options: Vec<ChoiceOption>,
    },
    MultiChoice {
        options: Vec<ChoiceOption>,
    },
    Confirm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub tags: Vec<String>,
    pub knowledge_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WizardCondition {
    AnswerEquals { question_id: String, value: String },
    AnswerContains { question_id: String, values: Vec<String> },
    TechnologyDetected { technology: String },
    TechnologyNotDetected { technology: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardContext {
    pub project_path: Option<PathBuf>,
    pub is_existing: bool,
    /// ID типа проекта (rest-api, desktop-app...)
    pub project_type: Option<String>,
    /// Выбранные языки
    pub languages: Vec<String>,
    /// Выбранные фреймворки (language_id → framework_id)
    pub frameworks: Vec<String>,
    /// Выбранные инструменты
    pub tools: Vec<String>,
    /// Включённые фичи
    pub features: Vec<String>,
    /// Инфраструктурные компоненты
    pub infrastructure: Vec<String>,
    pub docker: bool,
    pub testing: bool,
    pub ci: bool,
    pub git_init: bool,
    pub vscode_config: bool,
    pub answers: std::collections::HashMap<String, Vec<String>>,
}

impl Default for WizardContext {
    fn default() -> Self {
        Self {
            project_path: None,
            is_existing: false,
            project_type: None,
            languages: Vec::new(),
            frameworks: Vec::new(),
            tools: Vec::new(),
            features: Vec::new(),
            infrastructure: Vec::new(),
            docker: true,
            testing: true,
            ci: false,
            git_init: true,
            vscode_config: true,
            answers: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardSession {
    pub current_step: usize,
    pub total_steps: usize,
    pub context: WizardContext,
    pub questions: Vec<WizardQuestion>,
    pub is_complete: bool,
}

// ============================================================
// Analysis — анализ существующего проекта
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub project_path: PathBuf,
    pub detected_technologies: Vec<DetectedTechnology>,
    pub existing_configs: Vec<String>,
    pub missing_configs: Vec<String>,
    pub has_docker: bool,
    pub has_git: bool,
    pub has_ci: bool,
    pub has_tests: bool,
    pub has_readme: bool,
    pub has_license: bool,
    pub project_type_hints: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTechnology {
    pub name: String,
    pub version: Option<String>,
    pub confidence: DetectionConfidence,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DetectionConfidence {
    Certain,
    Likely,
    Possible,
}

// ============================================================
// Recipe Engine — рецепты
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Step {
    Command {
        id: String,
        label: String,
        description: String,
        command: String,
        args: Vec<String>,
        working_dir: Option<String>,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
    Generate {
        id: String,
        label: String,
        description: String,
        generator_id: String,
        generator_config: serde_json::Value,
        condition: Option<StepCondition>,
        on_error: ErrorMode,
    },
    Parallel {
        id: String,
        label: String,
        steps: Vec<Step>,
        on_error: ErrorMode,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepCondition {
    Always,
    ContextHas { key: String, value: String },
    ContextMissing { key: String },
    FileExists { path: String },
    FileNotExists { path: String },
    TechnologyDetected { name: String },
    TechnologyNotDetected { name: String },
    FeatureEnabled { feature: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ErrorMode {
    Abort,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepPreview {
    pub id: String,
    pub label: String,
    pub description: String,
    pub action: String,
    pub will_execute: bool,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePreview {
    pub recipe_id: String,
    pub recipe_name: String,
    pub step_previews: Vec<StepPreview>,
    pub total_steps: usize,
    pub will_execute_count: usize,
    pub will_skip_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub label: String,
    pub status: StepStatus,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Running,
    Success { message: String },
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OverallStatus {
    Success,
    PartialFailure { failed_steps: Vec<String> },
    Aborted { last_step: Option<String>, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub recipe_id: String,
    pub total_duration_ms: u64,
    pub step_results: Vec<StepResult>,
    pub overall: OverallStatus,
}

// ============================================================
// Generators — генераторы
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationReport {
    pub created_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub message: String,
}

// ============================================================
// Packs — функциональные и инфраструктурные пакеты
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: PackKind,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub knowledge_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackKind {
    Feature,
    Infrastructure,
    Tooling,
}

// ============================================================
// Knowledge — встроенная база знаний
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub key: String,
    pub title: String,
    pub content: String,
    pub source: Option<String>,
}
