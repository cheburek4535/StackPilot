// ============================================================
// Wizard Tree — загружается с бэкенда
// ============================================================

export type ProjectTypeDef = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  tags: string[];
  allow_custom_stack: boolean;
  /** false = тип проекта концептуально не имеет серверной стороны
   *  (browser-extension): UI скрывает шаги Backend Language / Framework */
  has_backend?: boolean;
};

export type LanguageDef = {
  id: string;
  label: string;
  icon: string | null;
  color: string | null;
  category?: string | null;
  knowledge_key: string | null;
  platforms?: string[];
};

export type FrameworkDef = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  /** "standalone" — сам создаёт полное приложение; "inplace" — мини-каркас внутрь проекта языка */
  class: string;
  /** "app" — главный фреймворк; "side" — побочный (aiogram, telegraf...) */
  kind: string;
  /** На какой стороне живёт: "backend" | "frontend" | "either" */
  side: string;
  /** Совместимые языки */
  languages: string[];
  /** Парные рекомендации: идеально сочетающиеся фреймворки (nest → react) */
  recommends?: { framework: string; note: string }[];
  /** Язык, который конструктор подставит автоматически (из languages) */
  recommended_language: string;
  /** Типы проектов, для которых доступен (пусто = везде) */
  project_types?: string[];
  /** ОС, на которых доступен (пусто = все) */
  platforms?: string[];
  knowledge_key: string | null;
  conflicts?: string[];
  /** Почему фреймворк несовместим с конкретным конфликтом (conflict_id → текст) */
  conflict_notes?: Record<string, string>;
  /** Фреймворки, доступные как UI-компаньон (tauri → svelte/vue/react, electron → react/vue/svelte) */
  companions?: string[];
  /** Как фреймворк участвует в создании каркаса: "root", "subdir" или undefined (inplace) */
  scaffold?: "root" | "subdir";
  /** Куда фреймворк создаёт файлы при сегментации: "backend" | "frontend" или undefined (корень) */
  output_subdir?: "backend" | "frontend";
  /** Обязательные системные инструменты сборки (npm, maven...) — безусловные требования окружения */
  required_tools?: string[];
  /** Фреймворк сам создаёт каркас для своих языков — generic-скаффолд языка подавляется */
  suppresses_language_scaffold?: boolean;
  /** Технологии UI внутри фреймворка с собственным стеком (Qt → QML/Widgets/WebEngine/Kirigami) */
  qt_ui_options?: QtUiOption[];
};

/** Технология UI внутри фреймворка с собственным стеком (Qt → QML/Widgets/
 *  WebEngine/Kirigami). id совпадает с id фреймворка-варианта (qt-qml и т.п.),
 *  web_framework_options — веб-фреймворки, которые умеет встраивать режим
 *  (WebEngine → React/Vue/Svelte). */
export type QtUiOption = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  web_framework_options: string[];
};

/** Готовый рецепт для вкладки «Шаблоны» */
export type ProjectPreset = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  tags: string[];
  stack: {
    project_type: string;
    backend_lang: string | null;
    frontend_lang: string | null;
    frameworks: string[];
    tools: string[];
    features: { testing: boolean; git: boolean; vscode: boolean; docker: boolean };
  };
};

export type StackSeverity = "Error" | "Warning";

export type StackIssue = {
  severity: StackSeverity;
  message: string;
};

export type ToolDef = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  category: string;
  knowledge_key: string | null;
  requires_docker: boolean;
  requires: string[];
  conflicts: string[];
  /** Языки, для которых тул имеет смысл (pytest — только python). Пусто = для любого стека */
  for_languages: string[];
  /** Типы проектов, для которых тул «рекомендован» (airflow/clickhouse — для etl). Пусто = нейтрален */
  for_project_types: string[];
};

/** Пара-предупреждение: выбор `a`+`b` не блокируется, но помечается Warning */
export type WarningPair = {
  a: string;
  b: string;
  reason: string;
  alternative: string;
};

export type WizardTreeData = {
  /** Легальные связки главных фреймворков одной стороны (gin+cobra, axum+clap, android+jetpack-compose...) */
  allowed_main_pairs: string[][];
  /** Фреймворки, не занимающие лимит «одного главного на сторону» (zig-cli) */
  main_limit_exempt: string[];
  /** Мобильные/десктопные «клиентские оболочки» (expo, react-native, plasmo,
   *  electron, tauri): серверная сторона для них имеет смысл только как
   *  разделённый REST API. Шаги Backend Language/Framework скрываются,
   *  бэкенд-фреймворки без rest-api в project_types блокируются. */
  client_shell_frameworks: string[];
  /** Нежёсткие предупреждения для концептуально спорных связок (Phoenix + SPA) */
  warning_pairs: WarningPair[];
  project_types: ProjectTypeDef[];
  languages: LanguageDef[];
  frameworks: FrameworkDef[];
  tools: ToolDef[];
  presets: ProjectPreset[];
  project_language_map: Record<string, string[]>;
  language_framework_map: Record<string, string[]>;
  framework_tool_map: Record<string, string[]>;
  /** Тип проекта → рекомендованные тулы wizard_tree (для пресетов выбора). */
  project_tool_map?: Record<string, string[]>;
};

// ============================================================
// Recommendations — подсказки стека (recommend.rs)
// ============================================================

export type FrameworkSuggestion = {
  id: string;
  note: string;
};

export type ToolSuggestion = {
  id: string;
  note: string;
};

export type StackRecommendations = {
  /** Главные фреймворки, подсвечиваемые парными рекомендациями (nest → react) */
  frameworks: FrameworkSuggestion[];
  /** Побочные фреймворки (kind="side": aiogram, telegraf...) */
  side_frameworks: FrameworkSuggestion[];
  /** Инструменты из трёх карт: тип проекта, языки, фреймворки */
  tools: ToolSuggestion[];
  /** Языки, которые стоит подставить, чтобы стек стал валидным */
  missing_languages: string[];
};

// ============================================================
// Wizard Session
// ============================================================

export type WizardContext = {
  project_path: string | null;
  project_name: string | null;
  is_existing: boolean;
  project_type: string | null;
  languages: string[];
  /** Языки, назначенные серверной стороне (шаг «Backend») — драйвер сегментации backend/ frontend/ */
  backend_languages: string[];
  /** Языки, назначенные клиентской стороне (шаг «Frontend») */
  frontend_languages: string[];
  frameworks: string[];
  tools: string[];
  /** Docker-инструменты мастера (postgresql, redis, ...), выбранные для
   * локальной установки вместо docker-compose: исключаются из compose,
   * в .env.example и LOCAL_INFRA.md уходят локальные настройки. */
  local_infra_tools: string[];
  features: string[];
  infrastructure: string[];
  docker: boolean;
  testing: boolean;
  ci: boolean;
  git_init: boolean;
  vscode_config: boolean;
  answers: Record<string, string[]>;
};

export type WizardSession = {
  current_step: number;
  total_steps: number;
  context: WizardContext;
  questions: WizardQuestion[];
  is_complete: boolean;
};

export type WizardQuestion = {
  id: string;
  label: string;
  description: string;
  question_type: QuestionType;
  condition: WizardCondition | null;
};

export type QuestionType =
  | { SingleChoice: { options: ChoiceOption[] } }
  | { MultiChoice: { options: ChoiceOption[] } }
  | "Confirm";

export type ChoiceOption = {
  id: string;
  label: string;
  description: string;
  tags: string[];
  knowledge_key: string | null;
};

export type WizardCondition =
  | { AnswerEquals: { question_id: string; value: string } }
  | { AnswerContains: { question_id: string; values: string[] } }
  | { TechnologyDetected: { technology: string } }
  | { TechnologyNotDetected: { technology: string } };

// ============================================================
// Legacy (deprecated)
// ============================================================

// ============================================================
// Analysis — отчет об анализе существующего проекта
// ============================================================

export type AnalysisReport = {
  project_path: string;
  detected_technologies: DetectedTech[];
  existing_configs: string[];
  missing_configs: string[];
  has_docker: boolean;
  has_git: boolean;
  has_ci: boolean;
  has_tests: boolean;
  has_readme: boolean;
  has_license: boolean;
  project_type_hints: string[];
  summary: string;
};

export type DetectedTech = {
  name: string;
  version: string | null;
  confidence: "Certain" | "Likely" | "Possible";
  evidence: string[];
};

// ============================================================
// Execution — выполнение рецепта
// ============================================================

export type Step =
  | { Command: CommandStep }
  | { WriteFile: WriteFileStep }
  | { RenderTemplate: RenderTemplateStep }
  | { CreateDirectory: CreateDirStep }
  | { Generate: GenerateStep }
  | { Parallel: ParallelStep };

export type CommandStep = {
  id: string; label: string; description: string;
  command: string; args: string[];
  working_dir: string | null; env: Record<string, string> | null;
  timeout_secs: number | null;
  condition: StepCondition | null; on_error: "Abort" | "Skip";
};

export type WriteFileStep = {
  id: string; label: string; description: string;
  path: string; content: string; overwrite: boolean;
  condition: StepCondition | null; on_error: "Abort" | "Skip";
};

export type RenderTemplateStep = {
  id: string; label: string; description: string;
  path: string; template: string; context: Record<string, string>;
  overwrite: boolean;
  condition: StepCondition | null; on_error: "Abort" | "Skip";
};

export type CreateDirStep = {
  id: string; label: string; description: string;
  path: string;
  condition: StepCondition | null; on_error: "Abort" | "Skip";
};

export type GenerateStep = {
  id: string; label: string; description: string;
  generator_id: string; generator_config: unknown;
  condition: StepCondition | null; on_error: "Abort" | "Skip";
};

export type ParallelStep = {
  id: string; label: string; description: string;
  steps: Step[]; condition: StepCondition | null;
  on_error: "Abort" | "Skip";
};

export type StepCondition =
  | { Always: null }
  | { ContextHas: { key: string; value: string } }
  | { ContextMissing: { key: string } }
  | { FileExists: { path: string } }
  | { FileNotExists: { path: string } }
  | { TechnologyDetected: { name: string } }
  | { TechnologyNotDetected: { name: string } }
  | { FeatureEnabled: { feature: string } };

export type ExecutionPlan = {
  recipe: Recipe;
  context: WizardContext;
  project_path: string;
  steps: Step[];
};

export type Recipe = {
  id: string; name: string; description: string;
  tags: string[]; steps: Step[];
};

export type RecipePreview = {
  recipe_id: string; recipe_name: string;
  step_previews: StepPreview[];
  total_steps: number; will_execute_count: number;
  will_skip_count: number;
};

export type StepPreview = {
  id: string; label: string; description: string;
  action: string; will_execute: boolean;
  skip_reason: string | null;
};

export type ExecutionEvent = {
  event_type: ExecutionEventType;
  step_id: string; step_index: number; total_steps: number;
  step_name: string; step_description: string;
  timestamp: string;
};

export type ExecutionEventType =
  | "StepStarted"
  | { StepProgress: { stdout: string; stderr: string } }
  | { StepCompleted: { status: StepStatus; duration_ms: number } }
  | { AllCompleted: { result: ExecutionResult } }
  | { Error: { message: string } };

/** Снимок выполнения проекта (для восстановления вкладки Create) */
export type ExecutionSnapshot = {
  running: boolean;
  events: ExecutionEvent[];
};

export type StepStatus =
  | "Pending"
  | "Running"
  | { Success: { message: string } }
  | { Skipped: { reason: string } }
  | { Failed: { error: string } };

export type ExecutionResult = {
  recipe_id: string; total_duration_ms: number;
  step_results: StepResult[];
  overall: OverallStatus;
};

export type StepResult = {
  step_id: string; label: string;
  status: StepStatus; duration_ms: number;
};

export type OverallStatus =
  | "Success"
  | { PartialFailure: { failed_steps: string[] } }
  | { Aborted: { last_step: string | null; reason: string } };

/** @deprecated Use ProjectTypeDef instead */
export type Category = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  tags: string[];
  children: SubCategory[];
};

/** @deprecated Use LanguageDef + FrameworkDef instead */
export type SubCategory = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  tags: string[];
  knowledge_key: string | null;
};
