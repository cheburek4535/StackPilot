import { invoke } from "@tauri-apps/api/core";
import type {
  WizardTreeData,
  ProjectTypeDef,
  WizardSession,
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
