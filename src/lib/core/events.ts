/**
 * Typed UI event bus.
 *
 * Purpose: notify the shell / local views that a domain action completed
 * successfully. This is NOT a backend mirror — each event carries only a
 * reference to a backend-confirmed fact (paths, names, statuses that were
 * already returned by a successful API call). It never carries secrets,
 * process logs, execution snapshots, or large payloads.
 *
 * Contract rule (contract C): an event is emitted ONLY after the
 * corresponding existing API call succeeded. The integration layer
 * (`src/lib/core/integration.ts`) is the intended emitter, so this rule is
 * enforced by construction — never emit from a page before awaiting the API.
 *
 * Process statuses are relayed verbatim from backend data (`ProcessStatus`)
 * and are never invented locally: a project/process is only ever described
 * as running when the backend says so.
 */

import type { ProcessStatus } from "$lib/modules/workspace/types";

export type ProjectCreatedEvent = {
  type: "project-created";
  projectPath: string;
  at: string;
};

export type ProjectOpenedEvent = {
  type: "project-opened";
  projectPath: string;
  profileName: string | null;
  at: string;
};

export type ProfileSavedEvent = {
  type: "profile-saved";
  profileName: string;
  projectPath: string | null;
  at: string;
};

export type ProcessStateChangedEvent = {
  type: "process-state-changed";
  processId: string;
  status: ProcessStatus;
  at: string;
};

export type ToolchainInstallCompletedEvent = {
  type: "toolchain-install-completed";
  at: string;
};

export type AppEvent =
  | ProjectCreatedEvent
  | ProjectOpenedEvent
  | ProfileSavedEvent
  | ProcessStateChangedEvent
  | ToolchainInstallCompletedEvent;

type Handler = (event: AppEvent) => void;

const listeners = new Map<AppEvent["type"], Set<Handler>>();

/**
 * Subscribes to one event type. Returns an unsubscribe function.
 * Handlers are called synchronously; guard your handler against throws if
 * you cannot tolerate one subscriber breaking the others.
 */
export function onAppEvent<E extends AppEvent>(
  type: E["type"],
  handler: (event: E) => void,
): () => void {
  let set = listeners.get(type);
  if (!set) {
    set = new Set();
    listeners.set(type, set);
  }
  const wrapped = handler as Handler;
  set.add(wrapped);
  return () => {
    set?.delete(wrapped);
  };
}

/** Publishes an event to all subscribers of its type. */
export function emitAppEvent(event: AppEvent): void {
  const set = listeners.get(event.type);
  if (!set || set.size === 0) return;
  for (const handler of Array.from(set)) {
    handler(event);
  }
}

/** Removes every subscriber (e.g. in tests or on app teardown). */
export function clearAppEventListeners(): void {
  listeners.clear();
}
