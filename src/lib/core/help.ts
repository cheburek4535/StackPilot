/**
 * Adaptive beginner-help system.
 *
 * The goal: a person who has never set up a development environment should
 * always know where to click, and should never be taught the same thing twice.
 *
 * Two layers:
 * - A global "beginner mode" flag (`helpMode`), persisted in localStorage,
 *   OFF by default. When ON, educational hints stay available permanently
 *   (a deliberate learning mode for anyone who wants it).
 * - An adaptive progress log (`helpProgress`). For first-time users the app
 *   automatically shows hints until the related action was actually done
 *   (`did`) or the hint was dismissed. Hints whose prerequisites were seen
 *   (`requiresSaw`) but which the user never touched keep showing; anything
 *   already learned stays hidden.
 *
 * Everything is UI-local (localStorage): the backend never owns onboarding
 * progress. All reads/writes are guarded — a hardened webview or quota error
 * degrades to in-memory defaults instead of throwing.
 */

import { get, writable } from "svelte/store";
import { readLocal, writeLocal, removeLocal } from "./storage";

const VERSION = 1;
export const HELP_MODE_KEY = "stackpilot:help:mode:v1";
export const HELP_PROGRESS_KEY = "stackpilot:help:progress:v1";

/** Milestone reached once the user created their first project. From then on
 *  automatic hints stop (unless beginner mode is explicitly enabled). */
export const HELP_GRADUATED = "flow.first_project_created";

export type HelpProgress = {
  /** Feature / page ids the user has already seen. */
  saw: string[];
  /** Milestones the user completed (actions, flows) or dismissed hints. */
  did: string[];
};

export type HelpHintSpec = {
  /** Stable id; also the milestone remembering an explicit dismissal. */
  id: string;
  /** Milestones that hide the hint once completed (auto mode only). */
  resolvedBy?: string[];
  /** Feature ids that must have been seen before the hint appears. */
  requiresSaw?: string[];
};

export const EMPTY_HELP_PROGRESS: HelpProgress = { saw: [], did: [] };

/**
 * Stable milestone / hint ids shared between the UI that renders a hint and
 * the action that resolves it. Centralised here to avoid drift between the
 * two sides (a typo would otherwise leave a hint showing forever).
 */
export const HELP = {
  /** Reached on the first successful project generation. */
  graduated: HELP_GRADUATED,

  // Actions / flows (recorded with markHelpDid).
  createTypeSelected: "create.type_selected",
  createStackConfirmed: "create.stack_confirmed",
  envInstallStarted: "env.install_started",
  envContinued: "env.continued",
  workspaceExplored: "workspace.explored",
  workspaceFileOpened: "workspace.file_opened",
  workspaceLogsViewed: "workspace.logs_viewed",
  devlProfileOpened: "devl.profile_opened",
  devlProfileRan: "devl.profile_ran",
  devlAnalyzeDone: "devl.analyze_done",
  toolchainExplored: "toolchain.explored",

  // Features/pages seen (recorded automatically by HelpHint).
  sawCreateStack: "page.create.stack",
  sawCreateReview: "page.create.review",
  sawEnvPanel: "page.create.env",

  // Hints.
  hintCreateType: "hint.create.type",
  hintCreateStack: "hint.create.stack",
  hintCreateTools: "hint.create.tools",
  hintCreatePreview: "hint.create.preview",
  hintEnvAutoInstall: "hint.env.auto_install",
  hintWorkspaceWelcome: "hint.workspace.welcome",
  hintWorkspaceFiles: "hint.workspace.files",
  hintWorkspaceLogs: "hint.workspace.logs",
  hintWorkspaceProblems: "hint.workspace.problems",
  hintProfilesHowto: "hint.devl.profiles_howto",
  hintProfileRun: "hint.devl.profile_run",
  hintAnalyze: "hint.devl.analyze",
  hintHomeStart: "hint.home.start",
  hintToolchainWelcome: "hint.toolchain.welcome",
  hintToolchainModes: "hint.toolchain.modes",
} as const;

/** Choose the project type (step 1 of the creator). */
export const HINT_CREATE_TYPE: HelpHintSpec = {
  id: HELP.hintCreateType,
  resolvedBy: [HELP.createTypeSelected],
};

/** What a "stack" is: one main framework per side + optional tools. */
export const HINT_CREATE_STACK: HelpHintSpec = {
  id: HELP.hintCreateStack,
  resolvedBy: [HELP.createStackConfirmed],
};

/** The tools section: everything is optional and can be changed later. */
export const HINT_CREATE_TOOLS: HelpHintSpec = {
  id: HELP.hintCreateTools,
  resolvedBy: [HELP.createStackConfirmed],
};

/** The review page is a live preview; explain what happens next. */
export const HINT_CREATE_PREVIEW: HelpHintSpec = {
  id: HELP.hintCreatePreview,
  resolvedBy: [HELP.envInstallStarted, HELP.envContinued],
};

/** StackPilot downloads and installs every missing tool itself. */
export const HINT_ENV_AUTO_INSTALL: HelpHintSpec = {
  id: HELP.hintEnvAutoInstall,
  resolvedBy: [HELP.envInstallStarted, HELP.envContinued],
};

/** First visit to the workspace: what this space is for. */
export const HINT_WORKSPACE_WELCOME: HelpHintSpec = {
  id: HELP.hintWorkspaceWelcome,
  resolvedBy: [HELP.workspaceExplored],
};

/** Profiles list: how to get from a folder to a running project. */
export const HINT_PROFILES_HOWTO: HelpHintSpec = {
  id: HELP.hintProfilesHowto,
  resolvedBy: [HELP.devlProfileOpened, HELP.devlProfileRan],
};

/** Profile page: review the steps, then press Run. */
export const HINT_PROFILE_RUN: HelpHintSpec = {
  id: HELP.hintProfileRun,
  resolvedBy: [HELP.devlProfileRan],
};

/** Home: the two ways to start (new project vs. existing folder). */
export const HINT_HOME_START: HelpHintSpec = {
  id: HELP.hintHomeStart,
  resolvedBy: [HELP.createTypeSelected, HELP.devlAnalyzeDone],
};

/** Analyze page: what it does and why it is useful. */
export const HINT_ANALYZE: HelpHintSpec = {
  id: HELP.hintAnalyze,
  resolvedBy: [HELP.devlAnalyzeDone],
};

/** Toolchain: what the Control Center is and that the scan is safe. */
export const HINT_TOOLCHAIN_WELCOME: HelpHintSpec = {
  id: HELP.hintToolchainWelcome,
  resolvedBy: [HELP.toolchainExplored],
};

/** Toolchain: short tour of the three modes. */
export const HINT_TOOLCHAIN_MODES: HelpHintSpec = {
  id: HELP.hintToolchainModes,
  resolvedBy: [HELP.toolchainExplored],
};

/** Workspace — Files tab. */
export const HINT_WORKSPACE_FILES: HelpHintSpec = {
  id: HELP.hintWorkspaceFiles,
  resolvedBy: [HELP.workspaceFileOpened],
};

/** Workspace — Logs tab. */
export const HINT_WORKSPACE_LOGS: HelpHintSpec = {
  id: HELP.hintWorkspaceLogs,
  resolvedBy: [HELP.workspaceLogsViewed],
};

/** Workspace — Problems tab (dismiss-only). */
export const HINT_WORKSPACE_PROBLEMS: HelpHintSpec = {
  id: HELP.hintWorkspaceProblems,
};

// ----------------------------------------------------------
// Pure logic (kept free of stores so it can be unit-tested)
// ----------------------------------------------------------

function appendUnique(list: string[], id: string): string[] {
  return list.includes(id) ? list : [...list, id];
}

export function withSaw(progress: HelpProgress, id: string): HelpProgress {
  if (progress.saw.includes(id)) return progress;
  return { saw: appendUnique(progress.saw, id), did: progress.did };
}

export function withDid(progress: HelpProgress, id: string): HelpProgress {
  if (progress.did.includes(id)) return progress;
  return { saw: progress.saw, did: appendUnique(progress.did, id) };
}

export function graduated(progress: HelpProgress): boolean {
  return progress.did.includes(HELP_GRADUATED);
}

/** Master switch: automatic help runs until the first project is created;
 *  beginner mode keeps it on permanently. */
export function helpEnabled(progress: HelpProgress, mode: boolean): boolean {
  return mode || !graduated(progress);
}

/** Whether the prerequisites to show a hint were seen already. */
export function hintPrereqsMet(
  progress: HelpProgress,
  spec: HelpHintSpec,
): boolean {
  return spec.requiresSaw?.every((id) => progress.saw.includes(id)) ?? true;
}

/**
 * Decide whether a hint should be rendered.
 * - Beginner mode: show until the user explicitly dismisses it. Completing
 *   the underlying action does NOT remove it — it is a permanent reference.
 * - Automatic mode (first-time users): show until dismissed OR the related
 *   action was completed. This is the "don't teach it twice" behaviour.
 */
export function isHintVisible(
  progress: HelpProgress,
  mode: boolean,
  spec: HelpHintSpec,
): boolean {
  if (!helpEnabled(progress, mode)) return false;
  if (!hintPrereqsMet(progress, spec)) return false;
  if (progress.did.includes(spec.id)) return false; // dismissed
  if (mode) return true;
  return !(spec.resolvedBy?.some((id) => progress.did.includes(id)) ?? false);
}

// ----------------------------------------------------------
// Persistence
// ----------------------------------------------------------

function isProgress(value: unknown): value is HelpProgress {
  if (typeof value !== "object" || value === null) return false;
  const p = value as Record<string, unknown>;
  return (
    Array.isArray(p.saw) &&
    p.saw.every((x) => typeof x === "string") &&
    Array.isArray(p.did) &&
    p.did.every((x) => typeof x === "string")
  );
}

function loadMode(): boolean {
  return readLocal<boolean>(HELP_MODE_KEY, VERSION) === true;
}

function loadProgress(): HelpProgress {
  const data = readLocal<unknown>(HELP_PROGRESS_KEY, VERSION);
  return isProgress(data)
    ? { saw: [...data.saw], did: [...data.did] }
    : { ...EMPTY_HELP_PROGRESS };
}

function persistMode(on: boolean): void {
  writeLocal(HELP_MODE_KEY, on, VERSION);
}

function persistProgress(progress: HelpProgress): void {
  writeLocal(HELP_PROGRESS_KEY, progress, VERSION);
}

// ----------------------------------------------------------
// Stores
// ----------------------------------------------------------

/** Global beginner mode. OFF by default. */
export const helpMode = writable<boolean>(loadMode());

/** Adaptive progress log used to skip already-learned hints. */
export const helpProgress = writable<HelpProgress>(loadProgress());

export function setHelpMode(on: boolean): void {
  helpMode.set(on);
  persistMode(on);
}

export function toggleHelpMode(): void {
  setHelpMode(!get(helpMode));
}

/** Reset only the adaptive progress (keeps the mode flag). */
export function resetHelpProgress(): void {
  helpProgress.set({ ...EMPTY_HELP_PROGRESS });
  removeLocal(HELP_PROGRESS_KEY);
}

/** Remember that the user saw a feature/page (never hides on its own). */
export function markHelpSaw(id: string): void {
  helpProgress.update((p) => {
    const next = withSaw(p, id);
    if (next !== p) persistProgress(next);
    return next;
  });
}

/** Remember that the user completed an action (hides hints resolved by it,
 *  or dismisses a hint outright). */
export function markHelpDid(id: string): void {
  helpProgress.update((p) => {
    const next = withDid(p, id);
    if (next !== p) persistProgress(next);
    return next;
  });
}

/** Dismiss a specific hint (same as marking `did` with the hint id). */
export function dismissHelpHint(id: string): void {
  markHelpDid(id);
}

/** The user finished the first full flow — automatic hints stand down. */
export function markHelpGraduated(): void {
  markHelpDid(HELP_GRADUATED);
}
