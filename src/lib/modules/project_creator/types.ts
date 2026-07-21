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
};

export type LanguageDef = {
  id: string;
  label: string;
  icon: string | null;
  color: string | null;
  knowledge_key: string | null;
};

export type FrameworkDef = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  knowledge_key: string | null;
};

export type ToolDef = {
  id: string;
  label: string;
  description: string;
  icon: string | null;
  category: string;
  knowledge_key: string | null;
};

export type WizardTreeData = {
  project_types: ProjectTypeDef[];
  languages: LanguageDef[];
  frameworks: FrameworkDef[];
  tools: ToolDef[];
  project_language_map: Record<string, string[]>;
  language_framework_map: Record<string, string[]>;
  framework_tool_map: Record<string, string[]>;
};

// ============================================================
// Wizard Session
// ============================================================

export type WizardContext = {
  project_path: string | null;
  is_existing: boolean;
  project_type: string | null;
  languages: string[];
  frameworks: string[];
  tools: string[];
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
