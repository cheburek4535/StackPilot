import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  WizardTreeData,
  ProjectTypeDef,
  WizardSession,
  AnalysisReport,
  ExecutionPlan,
  ExecutionSnapshot,
  RecipePreview,
  WizardContext,
  StackIssue,
  StackRecommendations,
  ProjectFilePreview,
  ProjectFileCount,
} from "./types";

export function pingProjectCreator(): Promise<string> {
  return invoke("ping_project_creator");
}

export function getWizardTree(): Promise<WizardTreeData> {
  return invoke("get_wizard_tree");
}

export function getProjectTypes(): Promise<ProjectTypeDef[]> {
  return invoke("get_project_types");
}

export function startWizard(projectPath?: string): Promise<WizardSession> {
  return invoke("start_wizard", { projectPath: projectPath ?? null });
}

export function submitWizardAnswer(
  session: WizardSession,
  questionId: string,
  answers: string[]
): Promise<WizardSession> {
  return invoke("submit_wizard_answer", { session, questionId, answers });
}

export function analyzeProjectTechnologies(path: string): Promise<AnalysisReport> {
  return invoke("analyze_project_technologies", { path });
}

export async function selectFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false, title: "Select project folder" });
  return selected ?? null;
}

// ---- Recipe Execution ----

export function previewProjectRecipe(
  context: WizardContext,
  projectPath: string,
): Promise<RecipePreview> {
  return invoke("preview_project_recipe", { context, projectPath });
}

/** Предпросмотр файловой структуры проекта: дерево файлов с уровнями
 *  достоверности (certain/expected/unknown), содержимое файлов, которые
 *  создаём мы, и список удалаемых опциональных шагов. */
export function previewProjectFiles(
  context: WizardContext,
  projectPath: string,
  removedStepIds: string[] = [],
): Promise<ProjectFilePreview> {
  return invoke("preview_project_files", { context, projectPath, removedStepIds });
}

export function startProjectExecution(
  context: WizardContext,
  projectPath: string,
  removedStepIds: string[] = [],
): Promise<ExecutionPlan> {
  return invoke("start_project_execution", { context, projectPath, removedStepIds });
}

/** Снимок выполнения: работает ли оно и буфер событий (для восстановления вкладки) */
export function getProjectExecutionSnapshot(): Promise<ExecutionSnapshot> {
  return invoke("project_execution_snapshot");
}

/** Подсчёт реальных файлов сгенерированного проекта на диске (быстрый walk,
 *  включает node_modules). Может вернуть capped=true на очень больших деревьях. */
export function countProjectFiles(path: string): Promise<ProjectFileCount> {
  return invoke("count_project_files", { path });
}

export function checkFolderExists(path: string): Promise<boolean> {
  return invoke("check_project_folder_exists", { path });
}

/** ОС, на которой работает приложение ("windows", "macos", "linux") */
export function getHostPlatform(): Promise<string> {
  return invoke("get_host_platform");
}

/** Проверка стека на ограничения (лимиты, конфликты, платформы, языки,
 *  инструменты: зависимости, языковая совместимость, ответственности) */
export function validateProjectStack(
  projectType: string | null,
  backendLanguages: string[],
  frontendLanguages: string[],
  frameworks: string[],
  tools: string[],
): Promise<StackIssue[]> {
  return invoke("validate_project_stack", {
    projectType,
    backendLanguages,
    frontendLanguages,
    frameworks,
    tools,
  });
}

/** Рекомендации стека: парные фреймворки, side-фреймворки, инструменты, недостающие языки */
export function getStackRecommendations(
  projectType: string | null,
  backendLanguages: string[],
  frontendLanguages: string[],
  frameworks: string[],
): Promise<StackRecommendations> {
  return invoke("get_stack_recommendations", {
    projectType,
    backendLanguages,
    frontendLanguages,
    frameworks,
  });
}
