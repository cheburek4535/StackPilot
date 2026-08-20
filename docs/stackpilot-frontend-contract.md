# StackPilot Frontend Contract

Status: draft (stage 0 — audit and contract recording only, no UI changes).
Companion file: `docs/stackpilot-frontend-progress.md`.

This document records the *verified* state of the frontend and the *agreed target*
architecture. Every API, event, and route listed below was checked against the real
source files (frontend wrappers in `src/lib/modules/*/api.ts` and backend commands in
`src-tauri/src/**/commands.rs` + `src-tauri/src/lib.rs`). Anything not present in the
codebase is explicitly marked as a backend limitation and is **not** to be invented.

---

## 0. Verified technology baseline

| Item | Verified | Evidence |
|---|---|---|
| SvelteKit 2 + adapter-static SPA (`fallback: index.html`, `ssr = false`) | yes | `svelte.config.js`, `src/routes/+layout.ts` |
| Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`) | yes | every page/component |
| TypeScript strict | yes | `tsconfig.json` (`"strict": true`) |
| Tauri v2 (`@tauri-apps/api`, `plugin-dialog`, `plugin-opener`) | yes | `package.json`, `src-tauri/capabilities/default.json` |
| CodeMirror 6 (`codemirror`, `@codemirror/*`, `theme-one-dark`) | yes | `src/lib/components/CodeEditor.svelte`, `package.json` |
| xterm (`@xterm/xterm` 6) | yes | `src/routes/processes/+page.svelte` (log viewer modal) |

---

## A. Canonical navigation

Canonical routes (target; **only `/workspace` exists today** — see legacy table):

| Route | Purpose | Exists now |
|---|---|---|
| `/` | Home / overview | yes (`src/routes/+page.svelte`, rewritten in stage 3) |
| `/devlauncher` | DevLauncher hub (overview / analyze / profiles / processes views) | **yes** (`src/routes/devlauncher/*`, added in stage 3) |
| `/workspace` | Workspace (project context, runtime, files…) | yes (`src/routes/workspace/*`) |
| `/project-creator` | Project Creator wizard | **no** |
| `/toolchain` | Toolchain / environment | **no** |
| `/settings` | Settings | yes (`src/routes/settings/+page.svelte`) |

### Legacy aliases (must NOT be removed until the canonical routes work)

| Legacy route | Today | Maps to |
|---|---|---|
| `/analyze` | `src/routes/analyze/+page.svelte` (SPA redirect) | DevLauncher analyze view |
| `/profiles` | `src/routes/profiles/+page.svelte` (SPA redirect) | DevLauncher profiles view |
| `/profiles/[name]` | `src/routes/profiles/[name]/+page.svelte` (SPA redirect) | DevLauncher profile detail |
| `/processes` | `src/routes/processes/+page.svelte` (SPA redirect, preserves query) | DevLauncher processes view |
| `/create` | `src/routes/create/+page.svelte` | Project Creator |
| `/environment` | `src/routes/environment/+page.svelte` | Toolchain |

Migration rules:
- Legacy routes stay fully functional until their canonical counterpart renders
  the same view and navigation is switched.
- After migration, legacy aliases may be kept as redirects (SPA-level), not removed
  while any internal link still points at them.
- As of stage 3, `/analyze`, `/profiles`, `/profiles/[name]`, `/processes` are
  SPA redirects to their `/devlauncher` counterparts (the `/processes` redirect
  preserves the query string).
- Internal cross-links today: profile detail `goto("/devlauncher/profiles")`,
  `goto("/workspace")` (`devlauncher/profiles/[name]/+page.svelte`). As of
  stage 4, the workspace empty states link to the canonical
  `/devlauncher`, `/create` and `/devlauncher/processes` routes
  (`workspace/+page.svelte`, `workspace/runtime/+page.svelte`,
  `workspace/files/+page.svelte`). The workspace Runtime no longer deep-links
  to `/processes?log=<id>` — since stage 4 it opens logs inline via
  `getProcessLogs()` in a modal; `/devlauncher/processes` still honors a
  `?log=<id>` query param (kept from stage 3).

---

## B. Main application shell (target architecture)

Single root layout `src/routes/+layout.svelte` must provide, in order:

1. **One global navigation** — a single nav component used by every route (the
   workspace pages currently re-implement their own sidebar 7 times; that is not the
   global nav).
2. **Route-aware active state** — driven by `$page.url.pathname` (as the current
   topbar already does), including sub-route matching for `/workspace/*`.
3. **Global notification layer** — today every page implements its own
   `status`/`resultMsg`/`errorMsg` banner; no shared layer exists. The shell must own
   one notification surface (toasts/banners) that domain pages feed via a store.
4. **Theme layer** — today theming is duplicated CSS `@media (prefers-color-scheme: dark)`
   blocks per page plus non-scoped `:root` redeclarations. The shell must own a single
   theme mechanism. Source of the theme value: **backend field `AppSettings.theme`
   (confirmed, `src/lib/core/types.ts`)** — see section C. The editor component also
   evaluates `matchMedia(...)` once at mount (`CodeEditor.svelte:42`) and is not
   reactive to theme changes — a known integration gap.
5. **Onboarding layer** — does not exist today; the shell must render it (frontend-local
   state only, see section C).
6. **Page content rendered through the layout** — `{@render children()}` as today.
7. **The layout must NOT contain domain business logic** — navigation, notifications,
   theme, onboarding only. No API calls of the domain modules, no process/profile
   logic. (Current layout already complies; the workspace subpages had their own
   sidebar markup, which was **moved into the workspace layout in stage 4** —
   `src/routes/workspace/+layout.svelte` owns the tabs, project header and the
   AI-assistant extension point, and stays free of domain business logic.)

---

## C. Source of truth

### Backend (authoritative — must never be shadowed by local state)

| Domain | Backend state | Confirmed via |
|---|---|---|
| Current workspace project | `set_current_project` / `get_current_project` / `clear_current_project` / `open_project_from_path` (`WorkspaceState.project`) | `src-tauri/src/modules/workspace/commands.rs` |
| Processes | `spawn_process` … `get_process_logs`, tracked in `WorkspaceState.process_manager` | same |
| Session | `get_session_info` (`WorkspaceState.session`) | same |
| Saved profiles | `list_profiles`/`get_profile`/`save_profile`/`delete_profile` (files under `<data_dir>/profiles`) | `src-tauri/src/modules/devlauncher/commands.rs` |
| Toolchain state | `tc_get_metadata`, `tc_get_install_status`, `tc_get_health_report` (`ToolchainState`, `state.json`) | `src-tauri/src/modules/toolchain/commands.rs` |
| Project Creator execution | `start_project_execution`, `project_execution_snapshot`, `project_creator:step_event` buffer | `src-tauri/src/modules/project_creator/commands.rs` |
| App settings | `get_settings` / `update_settings` / `reset_settings` | `src-tauri/src/core/settings.rs` |

### Frontend local persistence (never presented as backend-confirmed)

| Item | Where it lives today | Contract |
|---|---|---|
| Onboarding state | `src/lib/core/onboarding.ts` → localStorage key `stackpilot:onboarding:v1` | localStorage |
| Recent projects | `src/lib/core/recent.ts` → localStorage key `stackpilot:recent:v1` (`{path, name, source, at}`, capped at 20) | localStorage; reference written **only** after a confirmed successful action via `src/lib/core/integration.ts` |
| Last route | `src/lib/core/lastRoute.ts` → localStorage key `stackpilot:last-route:v1`; recorded by `AppShell.svelte` on navigation | localStorage |
| UI activity | `src/lib/core/activity.ts` → localStorage key `stackpilot:activity:v1` (`{key, at}`, capped at 50) | localStorage |
| Theme / preferences | **backend** field `AppSettings.theme` / `language` (confirmed, `src/lib/core/types.ts`) — applied by `src/lib/core/theme.ts`; UI-local (non-backend) preferences in `src/lib/core/preferences.ts` → localStorage key `stackpilot:prefs:v1` | do not duplicate theme in localStorage; read from `getSettings()`; only non-backend prefs are persisted locally |
| Notifications | `src/lib/core/toasts.ts` (in-memory only) | in-memory (UI-local) |

Existing local persistence (must be preserved and kept clearly separated):

- `stackpilot:create:session:v1` in **sessionStorage** (`src/lib/modules/project_creator/createSession.ts`) —
  light wizard fields only (whitelist `LIGHT_FIELDS`); heavy data (logs, events,
  snapshots) lives on the backend and is re-synced via `reSyncLiveSessions()`.
- localStorage keys (introduced in stage 2, all UI-local, schema-versioned via
  `src/lib/core/storage.ts`): `stackpilot:onboarding:v1`, `stackpilot:recent:v1`,
  `stackpilot:last-route:v1`, `stackpilot:activity:v1`, `stackpilot:prefs:v1`.
  None of these mirror backend state, and no "running" flag is ever stored —
  process status is only relayed verbatim from backend `ProcessStatus` data.

Rule: a value is backend state if it is produced by the commands in section D–G;
everything else is UI-local. Never render a sessionStorage/localStorage value as a
backend-confirmed fact.

---

## D. Confirmed DevLauncher APIs

Frontend wrappers: `src/lib/modules/devlauncher/api.ts`; types `types.ts`.
All 8 verified 1:1 against `src-tauri/src/modules/devlauncher/commands.rs` and the
handler registration in `src-tauri/src/lib.rs`:

| Wrapper (api.ts) | Backend command | Notes |
|---|---|---|
| `ping` | `ping_rust` | unused by pages (wrapper only) |
| `getDemoProfile` | `get_demo_profile` | **demo data** — hardcoded profile in backend; used by home and profile detail |
| `listProfiles` | `list_profiles` | |
| `getProfile` | `get_profile` | unused by pages (wrapper only; detail page compares `getDemoProfile().name` first) |
| `saveProfile` | `save_profile` | |
| `deleteProfile` | `delete_profile` | |
| `executeAction` | `execute_action` | backend signature has optional `session_id` — the wrapper does not expose it today |
| `analyzeProject` | `analyze_project` | |

**`launchProfile` does NOT exist** anywhere in the codebase (frontend or backend,
verified by search). It must not be created or used. "Run profile" is a frontend
composition of `executeAction` calls (see `runAll()` in `+page.svelte` and
`profiles/[name]/+page.svelte`).

---

## E. Confirmed Workspace APIs

Frontend wrappers: `src/lib/modules/workspace/api.ts`; types `types.ts`.
All 14 verified against `src-tauri/src/modules/workspace/commands.rs` and `lib.rs`:

`spawnProcess`, `listProcesses`, `killProcess`, `refreshProcess`, `getProcessLogs`,
`setCurrentProject`, `getCurrentProject`, `clearCurrentProject`,
`openProjectFromPath`, `getSessionInfo`, `listDirectory`, `readFile`, `writeFile`,
`openInVSCode`.

Confirmed backend events (workspace): `process-output`, `process-status`
(constants `src-tauri/src/modules/workspace/process_manager.rs:10-11`; listened in
`src/routes/processes/+page.svelte:51,60`).

Backend commands that exist but have **no frontend wrapper** (backend limitation /
unfinished integration — do not invent usage):
- `get_problems`, `clear_problems`, `get_workspace_overview`
  (`src-tauri/src/modules/workspace/commands.rs:146-166`).
  The Workspace "Problems" page currently derives problems client-side from
  `listProcesses()` instead of `get_problems`.

---

## F. Confirmed Toolchain APIs

Frontend wrappers: `src/lib/modules/toolchain/api.ts`; types `types.ts`.
All verified against `src-tauri/src/modules/toolchain/commands.rs` and `lib.rs`:

`pingToolchain`, `getToolDefinitions`, `getEnvironmentInfo`, `checkEnvironment`,
`buildInstallPlan`, `runInstall`, `getInstallStatus`, `abortInstall`,
`getNewSecrets`, `getToolchainMetadata`, `getHealthReport`.

Confirmed event listeners (frontend `api.ts` + backend `app.emit`):

| Listener | Event | Backend emit site |
|---|---|---|
| `listenToolchainEvents` | `toolchain:task_event` | `src-tauri/.../toolchain/commands.rs:311` |
| `listenInstallDone` | `toolchain:install_done` | `commands.rs:219` |
| `listenCheckProgress` | `toolchain:check_progress` | `commands.rs:63` |

**Explicitly NOT available** (do not promise in UI):
- uninstall / update / upgrade of installed tools (no command; `UpdateAvailable`
  status is display-only);
- arbitrary marketplace / plugin installation (no API);
- the frontend must not offer any action that has no backend command behind it.

---

## G. Project Creator

Single page today: `src/routes/create/+page.svelte` (~3700 lines) + module files
`src/lib/modules/project_creator/{api,types,rules,createSession}.ts`.

The **business logic is backend-owned** (verified): `get_wizard_tree`,
`start_wizard`, `submit_wizard_answer`, `analyze_project_technologies`,
`validate_project_stack`, `get_stack_recommendations`, `preview_project_recipe`,
`start_project_execution`, `project_execution_snapshot`,
`check_project_folder_exists`, `get_host_platform`
(`src-tauri/src/modules/project_creator/commands.rs`; all registered in `lib.rs`).

Frontend responsibilities that must be **preserved as-is** during any refactor:

- **Wizard / session**: `WizardSession` flow is backend-driven; the page restores the
  Create tab from sessionStorage + backend snapshots (`reSyncLiveSessions()`).
- **Stack selection**: constructor state (`selectedType`, side languages, frameworks,
  tools, flags) with invariants (`recomputeSideLangs()`).
- **Validation**: dual path — frontend mirror `rules.ts` (`validateStack`) for UX +
  backend `validate_project_stack` as final barrier (`confirmAll()`); rules must stay
  in sync with `validate.rs` (comment in `rules.ts`).
- **Recommendation**: `getStackRecommendations` (wrapper exists; not used by pages yet).
- **Environment check**: `checkEnvironment` + `check_progress` events, optional
  docker→local infra toggle (`envLocalInfra`), selection set (`envSelectedIds`),
  `buildInstallPlan` → `runInstall` → `toolchain:task_event` /
  `toolchain:install_done` / `getNewSecrets`.
- **Recipe preview**: `previewProjectRecipe` (wrapper exists; not used by pages yet).
- **Execution plan**: `startProjectExecution`, `project_creator:step_event`.
- **Execution snapshot**: `project_execution_snapshot` for tab-restore.
- **Event handling**: `handleExecEvent`, `handleToolchainEvent`,
  `handleInstallDone`, `handleCheckProgress` — step/phase/duration bookkeeping.
- **Persistence**: `createSession.ts` (sessionStorage, whitelist), debounced autosave,
  `persistNow()` on terminal points.

Allowed: presentation refactor (extract markup/components), careful integration into
the shell and canonical routes. Not allowed: changing the business logic above,
renaming backend contracts, or moving heavy state into localStorage.

---

## 3. Audit findings

### 3.1 APIs actually found (frontend wrappers ↔ backend commands)

- DevLauncher: 8/8 confirmed (D).
- Workspace: 14/14 confirmed (E). Plus 3 backend-only commands without wrappers:
  `get_problems`, `clear_problems`, `get_workspace_overview`.
- Toolchain: 11/11 confirmed (F) + 3 events.
- Project Creator: 13 wrappers confirmed (G). Backend also exposes
  `validate_project_stack_error` (no wrapper).
- Core: `get_settings`, `update_settings`, `reset_settings` confirmed.
- Feature-gated plugin commands (`mini_ide` completions/diagnostics/hover/
  go_to_definition/format_code) are registered in backend but unused by the frontend.

### 3.2 Product-vision functions impossible with the frontend alone (backend limitations)

- Toolchain uninstall / update / upgrade of a single tool.
- Arbitrary marketplace / plugin installation.
- `launchProfile` (a single command executing a whole profile) — absent by design;
  profile execution is a frontend loop over `executeAction`.
- Linking profile actions to the workspace session (`execute_action`'s optional
  `session_id` is not passed by the wrapper) — would need a wrapper change (frontend),
  but the semantic (actions counted into session errors) is backend.
- Consuming `project_creator:execution_done` (backend emits it, frontend never listens).
- Workspace overview/problems via backend (`get_workspace_overview`,
  `get_problems`) — backend exists, wrapper absent; frontend could wrap them, but that
  is not a new backend feature.

### 3.3 Pages using demo data

- `/` home (`src/routes/+page.svelte`): since stage 3, `getDemoProfile()` is **not**
  the main scenario — it appears only in a separate "Demo profile (diagnostics)"
  card with per-action `executeAction`; the page is driven by `getCurrentProject()`
  + `listProfiles()` (via DevLauncher) + local recent history.
- `/devlauncher/profiles/[name]` (`src/routes/devlauncher/profiles/[name]/+page.svelte`):
  if `getDemoProfile().name === name`, shows the demo profile instead of `getProfile`.
- Note: `get_demo_profile` is a real backend command returning a hardcoded profile —
  not a frontend mock, but it is demo data.

### 3.4 Duplication

- Workspace sidebar markup+styles duplicated in 7 files: `workspace/+page.svelte`,
  `workspace/{runtime,session,logs,problems,info,files}/+page.svelte`. **Resolved in
  stage 4** — the workspace pages now share one `workspace/+layout.svelte` shell
  (tabs + project header), so the duplication no longer exists.
- `statusLabel` / `statusClass` / `formatDuration` trios in `processes/+page.svelte`,
  `workspace/runtime/+page.svelte`, `workspace/+page.svelte`. **Resolved in stage 4**
  for the workspace pages — they now use the shared pure helpers
  `src/lib/modules/workspace/status.ts` (`statusLabel`, `statusTone`, `statusIcon`,
  `formatDuration`, `formatStarted`, `formatDateTime`, `formatFileSize`,
  `isProcessFailed`).
- Action icon/label/summary helpers in `+page.svelte`, `analyze/+page.svelte`,
  `profiles/[name]/+page.svelte`; `formatResult` duplicated in `+page.svelte` and
  `profiles/[name]/+page.svelte`.
- Non-scoped `:root` + `@media (prefers-color-scheme: dark)` blocks in nearly every
  page (global CSS pollution; last rendered route wins for `:root`).
- Per-page `<main>` shell styles (`max-width`, `padding`, `h1` sizing).
- Folder picker: `analyze/+page.svelte` uses `@tauri-apps/plugin-dialog` directly;
  `create/+page.svelte` uses `api.selectFolder` (same plugin).

### 3.5 Event-listener leak risks

- `src/routes/create/+page.svelte`: pattern `unlisten = await listen(...)` after an
  earlier `await` (e.g. `doCreateProject` ~line 1882, `startInstall` ~1663,
  `goToEnvironment` ~1569, `reSyncLiveSessions` ~448/469/484/496). If the component
  unmounts while the `await listen()` is in flight, `onDestroy` (line 547) has already
  run and the late-resolved listener is assigned to `unlisten` but never unsubscribed —
  a leak until webview teardown. Re-entering the page accumulates duplicates (Tauri
  registers one listener per call).
- `src/routes/processes/+page.svelte` (legacy; view now at
  `devlauncher/processes/+page.svelte`): the unmount-during-await race was
  **fixed in stage 3** — listener registrations are resolved via
  `Promise.all` and guarded with a `destroyed` flag, so a mid-await unmount
  unsubscribes the late-resolved listeners; `onDestroy` cleanup is correct.
- `create/+page.svelte` keeps only 4 unsubscribe handles (`unlisten`,
  `unlistenTc`, `unlistenTcDone`, `unlistenTcCheck`) for several registration sites —
  any future registration path must reuse these or leak.
- Minor: `tooltipTimer` (line 276), `secretCopied` timeout (line 1770) are not cleared
  in `onDestroy` (harmless but untidy).

### 3.6 localStorage / window risks

- Only `sessionStorage` key `stackpilot:create:session:v1` (guarded try/catch +
  whitelist; verified safe). No `localStorage` today.
- `sessionStorage` restores are tolerant of stale data (`restoreSnapshot` type-checks
  every field; unknown `typeId` → `selectedType = null`), but stale `phase: 5/6`
  triggers backend re-sync (`reSyncLiveSessions`) — safe due to try/catch, and slow
  on tab return.
- `CodeEditor.svelte:42`: `matchMedia("(prefers-color-scheme: dark)")` evaluated once
  at mount — editor theme is not reactive to a theme change while the app is open.
- `navigator.clipboard.writeText` without fallback (`create/+page.svelte:1768`,
  `processes/+page.svelte:255`) — guarded by try/catch, but no execCommand fallback.
- `window.confirm` for profile deletion (`profiles/[name]/+page.svelte:162`) — blocking
  and untestable in some webviews; keep or replace deliberately.
- Global `:root` style redeclarations per page (see 3.4) can fight the future theme
  layer.

---

## Notes

- Backend files were NOT modified in this stage (per task boundary). The two changed
  files in `git status` (`src-tauri/src/modules/project_creator/knowledge/mod.rs`,
  `packs/mod.rs`) are pre-existing working-tree changes, not part of this task.
- **Stage 4 (Workspace) additions** — see `docs/stackpilot-frontend-progress.md` §8:
  - `src/routes/workspace/+layout.svelte` — common workspace shell (tabs + project
    header + assistant extension point). No domain business logic.
  - `src/lib/modules/workspace/context.ts` — in-memory reactive cache of
    `getCurrentProject()` (only ever written from the backend command; not persisted;
    not a shadow of backend state).
  - `src/lib/modules/workspace/status.ts`, `paths.ts` — pure presentational helpers.
  - `src/lib/modules/assistant/types.ts` + `src/lib/components/workspace/AssistantPanel.svelte` —
    reserved AI-assistant extension point. Deliberately contains **no AI behaviour**;
    the panel is an honest "not implemented" placeholder that receives the current
    project/tab context to prove the seam.