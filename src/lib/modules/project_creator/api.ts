import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  WizardTreeData,
  ProjectTypeDef,
  WizardSession,
  AnalysisReport,
  ExecutionPlan,
  RecipePreview,
  WizardContext,
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

export function startProjectExecution(
  context: WizardContext,
  projectPath: string,
): Promise<ExecutionPlan> {
  return invoke("start_project_execution", { context, projectPath });
}
