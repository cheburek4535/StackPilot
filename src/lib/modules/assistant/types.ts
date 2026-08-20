/**
 * AI Assistant — reserved extension point.
 *
 * This module defines the SEAM a future AI assistant will plug into. It is
 * deliberately empty of behaviour: there is no chat, no model calls, no
 * inference. Only the context shape the assistant will receive is defined
 * so that wiring it in later is a drop-in change (the workspace shell feeds
 * `WorkspaceAssistantContext` to `AssistantPanel` today).
 *
 * Do not add "fake" assistant behaviour here.
 */

export type WorkspaceAssistantContext = {
  /** Backend current project name (null when no project is open). */
  projectName: string | null;
  /** Backend current project path (null when unset). */
  projectPath: string | null;
  /** Active workspace tab id (e.g. "runtime", "files"). */
  tab: string;
};
