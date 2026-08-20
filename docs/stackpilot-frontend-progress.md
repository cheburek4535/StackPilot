# StackPilot Frontend — Progress Log

Current stage: **5 — Final frontend review** (see §9 below).
Prior stage: 4 — Workspace (see §8); 3 — Home & unified DevLauncher (see §7); 2 — typed frontend integration layer & UI-local stores (see §6); 1 — visual foundation & application shell (see §5); 0 — audit & contract recording (see §1–§4).
Companion file: `docs/stackpilot-frontend-contract.md`.

---

## 1. What was verified

Tech stack (against real files):
- SvelteKit 2 + `adapter-static` SPA (`fallback: index.html`, `ssr = false`) — `svelte.config.js`, `src/routes/+layout.ts`.
- Svelte 5 runes everywhere; TypeScript strict (`tsconfig.json`).
- Tauri v2 with `plugin-dialog` + `plugin-opener`; capability `default.json` grants `core:default`, `dialog:allow-open`.
- CodeMirror 6 (`src/lib/components/CodeEditor.svelte`) and `@xterm/xterm` 6 (`src/routes/processes/+page.svelte`) — both confirmed in `package.json` and used.

Routes (actual files in `src/routes/`):
- `/` (`+page.svelte`), `/analyze`, `/create`, `/environment`, `/processes`, `/profiles`, `/profiles/[name]`, `/settings`, `/workspace` + sub-tabs `runtime`, `session`, `logs`, `problems`, `info`, `files`.
- Canonical `/devlauncher`, `/project-creator`, `/toolchain` **do not exist yet**; legacy aliases are the only routes today.

APIs (frontend wrapper ↔ backend command, verified 1:1 against `src-tauri/src/lib.rs` + module `commands.rs`):
- DevLauncher: `ping`, `getDemoProfile`, `listProfiles`, `getProfile`, `saveProfile`, `deleteProfile`, `executeAction`, `analyzeProject` — all confirmed.
- Workspace: `spawnProcess`, `listProcesses`, `killProcess`, `refreshProcess`, `getProcessLogs`, `setCurrentProject`, `getCurrentProject`, `clearCurrentProject`, `openProjectFromPath`, `getSessionInfo`, `listDirectory`, `readFile`, `writeFile`, `openInVSCode` — all confirmed.
- Toolchain: `pingToolchain`, `getToolDefinitions`, `getEnvironmentInfo`, `checkEnvironment`, `buildInstallPlan`, `runInstall`, `getInstallStatus`, `abortInstall`, `getNewSecrets`, `getToolchainMetadata`, `getHealthReport` — all confirmed, plus events `toolchain:task_event`, `toolchain:install_done`, `toolchain:check_progress`.
- Project Creator: wizard tree/session, analysis, validation, recommendations, recipe preview, execution plan/snapshot, folder checks, host platform — all confirmed on backend.
- Core settings: `get_settings` / `update_settings` / `reset_settings` — confirmed.

Events (backend `app.emit` sites verified):
- `process-output`, `process-status` (workspace `process_manager.rs`).
- `project_creator:step_event`, `project_creator:execution_done` (project_creator `commands.rs`; only `step_event` is consumed by the frontend).
- `toolchain:*` (three events above).

Persistence:
- Only `sessionStorage` key `stackpilot:create:session:v1` (`createSession.ts`, whitelisted light fields). **No `localStorage` anywhere in `src/`.**

Backend-extra commands without frontend wrappers (noted, not used): `get_problems`, `clear_problems`, `get_workspace_overview`, `validate_project_stack_error`, feature-gated `mini_ide` plugin commands.

## 2. What was created

- `docs/stackpilot-frontend-contract.md` — canonical routes + legacy aliases (section A), shell architecture (B), source-of-truth split (C), confirmed DevLauncher/Workspace/Toolchain/Project-Creator contracts (D–G), and the audit lists (3.1–3.6): found APIs, impossible functions, demo-data pages, duplication, listener-leak risks, localStorage/window risks.
- `docs/stackpilot-frontend-progress.md` — this file.
- No components, no routes, no UI rewrites (per stage boundary).

## 3. What CANNOT be considered confirmed

- `launchProfile` — **absent** from the entire codebase; must not be created or used. Profile "run" is a frontend loop over `executeAction`.
- Toolchain uninstall / update / upgrade of individual tools — **no backend command** (`UpdateAvailable` status is display-only).
- Arbitrary marketplace / plugin installation — **no API**.
- `project_creator:execution_done` consumption — backend emits, frontend never listens; semantics unverified.
- `execute_action`'s optional `session_id` — wrapper does not expose it; linking actions to the session is not implemented.
- Applying `AppSettings.theme` to the UI — field is backend-persisted and confirmed, but nothing applies it (UI uses `prefers-color-scheme`); the editor's `matchMedia` check is mount-time only.
- Workspace Problems via backend (`get_problems`) — backend exists, wrapper absent; current page computes client-side from processes.
- Behavior of `get_demo_profile` as "demo data" — confirmed as a real backend command returning a hardcoded profile; pages using it: `/` and `/profiles/[name]`.
- The `/processes?log=<id>` deep link from workspace Runtime — the processes page ignores the query param; dead link.

## 4. Next allowed stage (NOT started — do not implement here)

1. Create canonical routes `/devlauncher`, `/project-creator`, `/toolchain` rendering the existing views (legacy aliases stay; navigation points to canonical routes after they work; `?log=` deep link fixed during migration).
2. Shell refactor: single global nav with route-aware active state; global notification layer; theme layer (backend `AppSettings.theme` source); onboarding layer (localStorage); move workspace subpage sidebar into a workspace layout; keep the root layout free of domain logic.
3. Domain-module integration of existing wrappers (`getStackRecommendations`, `previewProjectRecipe`, `get_problems` wrapper, `validate_project_stack_error`, `openProjectFromPath`), listener-registration race fixes in `create/+page.svelte` and `processes/+page.svelte`.
4. Presentation refactor of Project Creator (markup extraction only; business logic per contract section G stays backend/session-backed).
5. Each step must keep the contract (sections A–G) and progress log updated.

---

## 5. Stage 1 — Visual foundation & application shell

Date: 2026-08-21.

Scope delivered (per task): unified design tokens, reusable UI primitives, root-layout
migration to the new shell with route-aware active states, onboarding foundation
(first-run/skip/complete/reopen, localStorage-safe), global notifications foundation.
**Not** started: Home / DevLauncher / Workspace / Toolchain content work; no domain
business logic, no API/Workspace/Toolchain/execution changes, no real `invoke`
re-wiring.

### 5.1 Design tokens — `src/lib/core/theme.css`

Single source of truth, imported once from `src/routes/+layout.svelte`.
- Near-black surfaces (`--sp-bg-0…3`), glass cards (`--sp-glass-bg`,
  `backdrop-filter`), neon accents violet/cyan/blue/lime, semantic success/info/
  warning/danger, text ramp, font stacks (sans + mono), 4px spacing scale, radii,
  3 elevation shadows + accent glow (kept restrained, no flood-lit glow), unified
  focus ring (`:focus-visible`), custom scrollbars, `prefers-reduced-motion` kill-switch,
  shared keyframes (`fade-in`, `rise-in`, `zoom-in`, `slide-in-right`, `spin`,
  `indeterminate`).
- Two complete token sets: `dark` (default, the flagship design) and `light`
  (minimal, consistent) — selected by `data-theme` on `<html>`.

### 5.2 Theme mechanism — `src/lib/core/theme.ts`

- Source of truth is the **backend** `AppSettings.theme` value (per contract C):
  `initTheme()` calls `getSettings()` (core module — allowed in the shell; no domain
  module invoked), normalizes `system|dark|light`, applies via `data-theme`,
  and follows `prefers-color-scheme` while in `system` mode.
- Falls back to `dark` when settings are unavailable (plain browser / non-Tauri).

### 5.3 UI primitives — `src/lib/components/ui/`

| Primitive | Notes |
|---|---|
| `AppShell.svelte` | topbar + sidebar nav + content + toast region + onboarding overlay; theme init + first-run open |
| `PageContainer.svelte` | `narrow/default/wide/full` widths |
| `PageHeader.svelte` | title + description + icon tile + actions snippet |
| `Card.svelte` | `glass/elevated/outline`, `sm/md/lg/none` padding, optional header/actions |
| `Button.svelte` | `primary/secondary/ghost/outline/danger/subtle`, `sm/md/lg`, icon(s), loading, block, renders `<a>` when `href` |
| `Badge.svelte` | `neutral/violet/cyan/blue/lime/amber/red`, optional dot |
| `Tabs.svelte` | controlled `value` + `onchange`, icons, disabled, `tablist`/`tab` a11y |
| `EmptyState.svelte` | icon + title + description + action snippet, compact |
| `LoadingState.svelte` | spinner + label, sizes, centered |
| `ErrorState.svelte` | title + mono message (scrollable) + retry + action |
| `Modal.svelte` | sizes, Escape/backdrop close, focus in/out, `role=dialog`, footer snippet |
| `Toast.svelte` / `ToastRegion.svelte` | per-kind icon, auto-dismiss timer, sticky (0), stack bottom-right, `aria-live` |
| `Progress.svelte` | determinate / indeterminate linear bar, sizes, label, `role=progressbar` |
| `IconButton.svelte` | `ghost/outline/solid/danger`, sizes, `href` support |
| `Icon.svelte` + `icons.ts` | ~25 inline stroke icons; map+type live in `icons.ts` (type export outside the component) |
| `OnboardingOverlay.svelte` | 4 honest steps, dots, Back/Next/Done/Skip, close, version |

### 5.4 Shell & navigation — `+layout.svelte`, `src/lib/core/navigation.ts`

- `+layout.svelte` is now `AppShell` + `{@render children()}` (no domain logic).
  `app.html` title → "StackPilot".
- Nav links point at **working legacy routes**; active-state rules follow the canonical
  mapping (contract A): `/devlauncher, /analyze, /profiles, /processes` → DevLauncher;
  `/project-creator, /create` → Project Creator; `/toolchain, /environment` →
  Toolchain; `/workspace/**` → Workspace; `/settings` → Settings; `/` → Home.
- `navigation.ts` exposes `NAV_GROUPS` / `isNavItemActive` / `isNavGroupActive`
  (pure, reusable, match rules independent of hrefs so the canonical switch is a 1-line change).

### 5.5 Onboarding — `src/lib/core/onboarding.ts`

- localStorage key `stackpilot:onboarding:v1` (`{status, at}`), guarded read/write with
  shape validation; malformed → treated as not-seen; storage failure → session-only
  fallback (never throws). Never presented as backend-confirmed.
- API: `showOnboarding / hideOnboarding / completeOnboarding / skipOnboarding /
  reopenOnboarding`; auto-open on `firstRun` from the shell.
- Reopen entry points: sidebar footer "Getting started" + topbar help icon.
- Copy contains **no fake product claims** (no launchProfile/uninstall/update promises;
  honestly notes that some screens keep the classic layout during migration).

### 5.6 Notifications — `src/lib/core/toasts.ts`

- `writable` store + `pushToast / dismissToast / clearToasts / notifyInfo / Success /
  Warning / Error`. Foundation only — domain pages are **not** re-wired yet (their own
  banners remain), so no business logic was touched.

### 5.7 Verification (real scripts from `package.json`)

- `npm run check` → **0 errors** (16 warnings, all pre-existing in legacy
  `analyze/+page.svelte` + `create/+page.svelte`, untouched).
- `npm run build` → **passes** (adapter-static, writes `build/`).
- Dev-server smoke test: all new modules + root route compile/serve (HTTP 200).

### 5.8 Errors & limitations (open items)

1. **Canonical routes do not exist yet** — nav still links to `/analyze`, `/create`,
   `/environment`; canonical `/devlauncher`, `/project-creator`, `/toolchain` are the
   next stage. `navigation.ts` match rules are ready for the flip.
2. **Legacy routes keep their own look** — the new shell is dark; legacy pages carry
   their own light-gray `:root`/card styles, so there is a visible chrome/content
   mismatch on legacy routes during migration. No legacy page was restyled (per scope).
3. **Workspace pages still render their own inner sidebar** inside the new shell
   sidebar — moving it into a workspace layout is a later step (contract B.7).
4. **Toasts are not consumed by any domain page yet** — foundation only, by design.
5. **`AppSettings.theme` is now applied** but the CodeMirror editor's theme remains
   mount-time `matchMedia` (known gap, contract 3.6) — not addressed here.
6. **Store implementation note**: `toasts`/`onboarding` use classic `writable` stores
   instead of module `$state` — svelte-check 4.7.2 rejects `$state` reassignment at
   module scope in `.svelte.ts`; `writable` is fully supported and auto-subscribes in
   Svelte 5 components.
7. **Modal/Onboarding focus** — foundation-level (focus first element, restore on
   close, Escape); no full focus-trap loop yet.
8. **`body { overflow: hidden }`** — the shell owns scroll via `.sp-content`; any
   legacy page that depended on body scroll still works because content scrolls inside
   the content pane.
9. **Theme** only supports `dark`/`light`/`system`; `AppSettings.theme` values outside
   that set normalize to `dark`.

---

## 6. Stage 2 — Typed frontend integration layer & UI-local stores

Date: 2026-08-21.

Scope delivered (per task): typed UI-local stores + storage primitives + a typed
event bus + an integration layer that records recent projects and publishes events
**only after confirmed backend success**. No backend calls are invented; no pages
were rewritten (pages are not re-wired yet — foundation only).

### 6.1 Changed files

New files:

| File | Purpose |
|---|---|
| `src/lib/core/storage.ts` | storage primitives: safe JSON parse, schema version (`{v,data}`), guarded `localStorage` read/write/remove, graceful fallback when storage is unavailable |
| `src/lib/core/events.ts` | typed UI event bus (5 events, see §6.4); emits only after confirmed success (enforced by the integration layer) |
| `src/lib/core/recent.ts` | recent project references store (`stackpilot:recent:v1`), deduped, capped at `RECENT_PROJECTS_LIMIT = 20`, cleanup by age |
| `src/lib/core/lastRoute.ts` | last route store (`stackpilot:last-route:v1`), UI-local, for restore-on-restart |
| `src/lib/core/activity.ts` | local UI activity store (`stackpilot:activity:v1`), deduped, capped at `ACTIVITY_LIMIT = 50`, cleanup by age |
| `src/lib/core/preferences.ts` | UI-local preferences store (`stackpilot:prefs:v1`); theme/language stay backend-owned (contract C) |
| `src/lib/core/integration.ts` | typed integration layer wrapping confirmed API calls → recent references + events after success |

Changed files:

| File | Change |
|---|---|
| `src/lib/components/ui/AppShell.svelte` | shell now records `lastRoute` on every navigation (two-line addition; no page rewrites) |
| `docs/stackpilot-frontend-progress.md` | this entry |
| `docs/stackpilot-frontend-contract.md` | section C updated to list the new localStorage keys |

Pre-existing stores kept as-is (already existed): `onboarding.ts` (`stackpilot:onboarding:v1`),
`toasts.ts` (notifications), `theme.ts` (backend `AppSettings.theme` source).

### 6.2 Storage semantics

- **Only lightweight, UI-local values** are persisted: onboarding status, recent
  project **references** (`{path, name, source, at}`), last route (a string),
  activity records (`{key, at}`), and non-backend UI preferences.
- **Never stored** (per task + contract C): secrets, process logs, execution
  snapshots, large payloads, and any local replacement/copy of the backend
  current project or saved profiles. `getCurrentProject()` / `listProfiles()`
  remain the single source of truth.
- **No "running" flags** exist in any local store. Process status is only ever
  relayed verbatim from backend `ProcessStatus` data.
- Every value is **schema-versioned** (`{v, data}`); mismatched/malformed values
  are treated as absent (safe JSON parse, never throws).
- **Graceful fallback**: when `localStorage` is unavailable (plain browser,
  hardened webview, quota), reads return null and writes are dropped; stores keep
  session-only in-memory state (same pattern as `onboarding.ts`).
- **Limits & cleanup**: recent projects capped at 20 and deduped by path;
  activity capped at 50 and deduped per key; both expose age-based cleanup
  (`cleanupRecentProjects`, `cleanupActivity`). `clearRecentProjects`,
  `clearActivity`, `clearLastRoute`, `resetPreferences` remove their keys.
- Existing `sessionStorage` key `stackpilot:create:session:v1` (`createSession.ts`,
  whitelisted light fields) is untouched and clearly separate.

### 6.3 APIs intentionally NOT added

- `launchProfile` — absent from the entire codebase; not created (profile "run"
  stays a frontend loop over `executeAction`).
- No new backend commands / wrappers; the integration layer only calls the
  already-confirmed wrappers (`openProjectFromPath`, `saveProfile`,
  `spawnProcess`, `killProcess`, `refreshProcess`).
- No local mirror of `getCurrentProject()` or `listProfiles()`.
- No invented process/project state — `confirmProcessStateChanged` relays only
  backend-confirmed `ProcessStatus`; `confirmToolchainInstallCompleted` is
  callable only after backend confirms install completion.
- Backend-only commands with no wrapper (`get_problems`, `clear_problems`,
  `get_workspace_overview`, `validate_project_stack_error`) remain unused.

### 6.4 Event bus (typed)

`onAppEvent(type, handler)` / `emitAppEvent(event)` with 5 events:
`project-created`, `project-opened`, `profile-saved`, `process-state-changed`,
`toolchain-install-completed`. Rule: an event is published **only** after the
corresponding existing API call succeeds — the integration layer is the intended
emitter, so pages must call it instead of emitting directly.

### 6.5 Known limitations

1. **Pages are not re-wired** — `openProject`, `saveProfileConfirmed`,
   `spawnProcessConfirmed`, etc. are available but no domain page calls them yet;
   their own banners/states remain. Wiring is a later stage (task: no page
   rewrites yet).
2. **Events are not consumed anywhere** yet — the bus is infrastructure; no toast/
   view subscription exists yet (toasts stay foundation-only).
3. **Recent projects are not rendered** anywhere yet — the store is ready for the
   Home/DevLauncher work.
4. **No age-based cleanup is scheduled** — `cleanupRecentProjects` /
   `cleanupActivity` exist but nothing invokes them automatically yet.
5. **`preferences.ts` intentionally excludes theme/language** — those remain
   backend-owned per contract C; the "theme/local preferences" group is covered by
   `theme.ts` (backend source) + this store (local, non-backend prefs).
6. **`killProcessConfirmed` hardcodes `status: "Killed"`** — mirrors the current
   `kill_process` contract (void return); if the backend ever returns a richer
   status, prefer passing it through `confirmProcessStateChanged`.
7. **Onboarding store predates `storage.ts`** — it keeps its own guarded helpers
   rather than the new primitives (working, lower risk to leave as-is).

---

## 7. Stage 3 — Home & unified DevLauncher

Date: 2026-08-21.

Scope delivered (per task): Home rewritten around real backend state + real
actions, and a canonical `/devlauncher` hub (Overview / Analyze / Profiles /
Processes) with the legacy routes kept as SPA redirects. Workspace was **not**
touched (task boundary: do not continue into Workspace).

### 7.1 Home — `src/routes/+page.svelte` (rewritten)

- **No longer demo-driven.** `getDemoProfile()` is NOT the main scenario; it is
  confined to a clearly-labeled **"Demo profile (diagnostics)"** card at the
  bottom with per-action `executeAction` only (no "launch"/run-all button).
- Shows the backend **current project** via `getCurrentProject()` when present
  (name, path, description, stack, "current" badge).
- Shows **local recent history** via the `recentProjects` store (stage 2) —
  explicitly labeled UI-local history, never presented as the backend project.
- **Empty state** when there is no current project and no recent history.
- **Real actions**:
  - Open Workspace — `goto("/workspace")` when a current project exists; for a
    recent entry it calls `openProject(path)` (integration layer →
    `openProjectFromPath`) then `goto("/workspace")`.
  - Open DevLauncher → `goto("/devlauncher")`; Open Project Creator →
    `goto("/create")` (canonical `/project-creator` still out of scope).
  - Open in VS Code → `openInVSCode(path)` only when a path exists (current
    project path / recent entry path).
- No "Launch project" button (no confirmed mechanism). No health/statistics
  panels (no guaranteed backend source). Feedback via the shell toasts.

### 7.2 DevLauncher — canonical `/devlauncher`

Structure (SvelteKit sub-routes under one hub):

| Route | Content |
|---|---|
| `/devlauncher` | **Overview** (`+page.svelte`) |
| `/devlauncher/analyze` | Analyze (profile editor — **moved** from `/analyze`) |
| `/devlauncher/profiles` | Profiles list (**moved** from `/profiles`) |
| `/devlauncher/profiles/[name]` | Profile detail (**moved** from `/profiles/[name]`) |
| `/devlauncher/processes` | Processes (**moved** from `/processes`) |
| `/devlauncher` (+layout) | DevLauncher chrome: sticky Overview/Analyze/Profiles/Processes tabs, no domain logic |

Legacy aliases `/analyze`, `/profiles`, `/profiles/[name]`, `/processes` are now
**SPA redirect pages** (`onMount` → `goto(..., { replaceState: true })`); the
`/processes` redirect preserves the query string so the workspace runtime
`/processes?log=<id>` deep link keeps working.

- **Overview** (`devlauncher/+page.svelte`): current backend project card
  (`getCurrentProject`), saved profiles list (`listProfiles`) with a selectable
  detail panel (name, description, path, actions). Individual actions run via
  `executeAction` **one at a time** (per-action Execute, result inline, enabled
  state respected). **No "launch whole profile" button.** "Open Workspace" only
  when the path/project context can be passed: current project → `goto('/workspace')`;
  a selected profile → `setCurrentProject(...)` **only when it carries a
  `project_path`**. Delete via `deleteProfile` + confirm.
- **Analyze** — the existing profile editor was preserved verbatim (action union
  types, RunCommand/OpenUrl/OpenApplication/WaitForUrl/WaitForPort/Delay/
  ExecuteScript fields, nested-field payload updates, drag reordering, enabled
  toggles, expandable editor, save). Only its internal link moved to
  `/devlauncher/profiles` and its styles were converted to the shell `--sp-*`
  tokens (was hardcoded light/dark `:root`).
- **Profiles / Profile detail** — moved and token-styled; detail keeps the
  `getDemoProfile` name-compare path, per-action execute, sequential run-all
  (frontend composition over `executeAction` — the documented "run" mechanism,
  not a nonexistent `launchProfile`), retry-failed, delete, open-in-workspace.
- **Processes** — moved and token-styled; all real workspace process APIs kept
  (`listProcesses`, `spawnProcess`, `killProcess`, `refreshProcess`,
  `getProcessLogs`), xterm terminal log modal, `process-output`/`process-status`
  listeners with correct `onDestroy` cleanup. **Fixed** the stage-0 listener
  race (registrations resolved via `Promise.all` + `destroyed` flag → no leak on
  mid-await unmount). Added `?log=<id>` deep-link handling (opens the log modal
  on mount) — fixing the previously-dead workspace link.

### 7.3 Shared helpers

- `src/lib/modules/devlauncher/actionMeta.ts` — pure presentational helpers
  (`actionVariant`, `actionIcon`, `actionTypeLabel`, `actionSummary`,
  `formatResult`, `resultClass`); used by Home + Overview (new code).

### 7.4 Navigation

- `src/lib/core/navigation.ts` — DevLauncher group now points at canonical
  routes (`/devlauncher`, `/devlauncher/analyze`, `/devlauncher/profiles`,
  `/devlauncher/processes`) with match rules that also cover the legacy aliases.

### 7.5 Verification (real scripts from `package.json`)

- `npm run check` → **0 errors** (16 warnings, all pre-existing: `create/+page.svelte`
  untouched + the legacy Analyze editor's pre-existing a11y warnings, now located
  at `devlauncher/analyze/+page.svelte`).
- `npm run build` → **passes** (adapter-static SPA, writes `build/`).
- Dev-server smoke test (port 5199): `/`, `/devlauncher`, `/devlauncher/profiles`,
  `/devlauncher/processes`, `/analyze`, `/processes?log=x` all HTTP 200.

### 7.6 Errors & limitations (open items)

1. **Workspace pages unchanged** (task boundary) — they still link to the legacy
   `/profiles`, `/analyze`, `/processes` aliases; those now redirect to the
   canonical DevLauncher views, so the links keep working (the `/processes?log=`
   deep link is now honored by `/devlauncher/processes`).
2. **Canonical `/project-creator` and `/toolchain` still do not exist** — Home
   points Project Creator at the legacy `/create`; the shell nav still uses
   `/create` and `/environment`. Switching those is a later stage.
3. **`actionMeta.ts` is used only by new code** — the moved pages keep their own
   local icon/label/summary helpers (preserved verbatim); deduplicating them
   across pages is optional follow-up.
4. **Processes modal is a dark xterm surface** regardless of theme (unchanged
   behavior from the legacy page).
5. **Demo profile diagnostics section** remains on Home by design (real backend
   `get_demo_profile` command, clearly labeled demo, per-action execute only).


---

## 8. Stage 4 — Workspace

Date: 2026-08-21.

Scope delivered (per task): the Workspace implemented around the real backend
contract — `getCurrentProject()` as the source of truth, a common workspace
shell for the nested routes, real process/session/file data only, an AI-assistant
extension point (no fake AI), Windows path handling on the frontend, and all
timers/listeners cleaned up. No backend file was modified.

### 8.1 Common Workspace shell — `src/routes/workspace/+layout.svelte`

- The 7 duplicated per-page sidebars (contract 3.4) are gone; the workspace now
  has ONE shell layout owning:
  - a sticky glass header with the project context (profile name, path, stack
    tags, "current" badge) — or "No project open" when the backend has none;
  - a tab bar (Overview / Runtime / Session / Logs / Problems / Info / Files)
    with route-aware active state (`$page.url.pathname`);
  - a "Close workspace" action (`clear_current_project` + reload) and an
    "Open in VS Code" action (only when a path exists);
  - the AI-assistant extension point (see 8.4).
- The shell loads the project via `getCurrentProject()` and shares it with the
  pages through `src/lib/modules/workspace/context.ts` (see 8.2). It contains no
  domain business logic.

### 8.2 Workspace context — `src/lib/modules/workspace/context.ts`

- `workspaceContext` is a reactive in-memory cache of `getCurrentProject()`
  (the backend remains the single source of truth). It is written ONLY from that
  command (plus `clear_current_project` for "Close workspace") — never invented,
  never persisted to localStorage, and always re-syncable via
  `reloadWorkspaceContext()`.
- Pages subscribe to it for the project and keep their own supplementary fetches
  (processes / session) behind a `dataLoaded` guard so they react when the
  project becomes available.

### 8.3 Pages (all rewritten to the shell + tokens; real data only)

| Route | Data source | Notes |
|---|---|---|
| `/workspace` (Overview) | `getCurrentProject` + `listProcesses` + `getSessionInfo` | hero card, real stat strip (running/total/restarts/session uptime from confirmed fields), running-process list, session card; empty state shows recent projects as **references with an explicit Open** (never auto-presented as current); no invented statistics |
| `/workspace/runtime` | **only** `listProcesses`, `refreshProcess`, `killProcess`, `getProcessLogs` | per-process cards, auto-refresh timer (cleanup in `onDestroy`), inline log modal via `getProcessLogs` |
| `/workspace/session` | `getSessionInfo` (only) + project | shows `started_at`/`duration_secs`/`process_count`/`error_count` verbatim; honest "No session started" empty state instead of fabricated zeros |
| `/workspace/logs` | `listProcesses` + `getProcessLogs` | process selector + log viewer; live re-fetch only while the selected process is Running; timer cleaned up |
| `/workspace/problems` | `listProcesses` (client-side derivation, per contract E — backend `get_problems` has no wrapper) | real problems from crashed / non-zero-exit / last_error processes; honest "no problems" state |
| `/workspace/info` | `getCurrentProject` + `listProcesses` | project facts table (real fields only) |
| `/workspace/files` | **only** `listDirectory`, `readFile`, `writeFile`, `openInVSCode` | file tree + breadcrumbs + parent navigation, CodeMirror editor (lifecycle preserved), save with cleared timeout, "Open in VS Code" |

### 8.4 AI-assistant extension point (no fake AI)

- `src/lib/modules/assistant/types.ts` defines `WorkspaceAssistantContext`
  (project name / path / active tab) — the typed seam a future assistant plugs
  into. No chat, no model calls, no inference anywhere.
- `src/lib/components/workspace/AssistantPanel.svelte` is an honest placeholder:
  it receives the real current context from the shell (proving the wiring) and
  states clearly that nothing is implemented and no data is sent anywhere.

### 8.5 Windows path handling (frontend-only, backend contract untouched)

- `src/lib/modules/workspace/paths.ts` — `toBreadcrumbs()` / `parentPath()` /
  `pathSegments()` / `normalizePath()` work on path strings only; the Rust side
  already accepts both separators, so no backend change was needed.
- Fixes the previous drive-root breadcrumb bug (`C:` → `C:/` is a valid
  directory) and adds "up one level" parent navigation for the file explorer.
- The files page auto-navigates to the project root only when the project itself
  changes (guarded by `lastProjectPath`) — browsing deeper into the tree is never
  yanked back.

### 8.6 Shared helpers

- `src/lib/modules/workspace/status.ts` — pure presentational helpers
  (`statusLabel`, `statusTone`, `statusIcon`, `isProcessFailed`, `formatDuration`,
  `formatStarted`, `formatDateTime`, `formatFileSize`), removing the duplicated
  status/format trios in the workspace pages (contract 3.4).

### 8.7 Verification (real scripts from `package.json`)

- `npm run check` → **0 errors** (16 warnings, all pre-existing in legacy
  `create/+page.svelte` + `devlauncher/analyze/+page.svelte`, untouched).
- `npm run build` → **passes** (adapter-static SPA, writes `build/`).
- Preview smoke test (port 5199): `/workspace`, `/workspace/runtime`,
  `/workspace/session`, `/workspace/logs`, `/workspace/problems`,
  `/workspace/info`, `/workspace/files` all HTTP 200.

### 8.8 Errors & limitations (open items)

1. **Workspace Problems remains client-side** — `get_problems` / `clear_problems`
   / `get_workspace_overview` still have no frontend wrapper; not invented per
   contract E.
2. **`spawn_process` is not exposed in the workspace Runtime** — the runtime page
   uses only the four APIs listed in rule 9; spawning stays in the DevLauncher
   Process Manager (`/devlauncher/processes`).
3. **CodeMirror theme is still mount-time `matchMedia`** (`CodeEditor.svelte:42`) —
   known pre-existing gap (contract 3.6); not addressed here.
4. **The workspace context is a UI cache, not a subscription to backend changes** —
   it is re-read on layout mount and after explicit actions (open recent / close);
   no backend push exists for project changes.
5. **Assistant panel is intentionally non-functional** — it is an extension point
   only (see 8.4).

---

## 9. Stage 5 — Final frontend review

Date: 2026-08-21.

Scope delivered (per task): a final read-only review of the whole frontend
against the contract. **No new features added; no backend file modified.** The
task checklist (points 1–9) was walked item by item against the real source.
Only the progress/contract docs were touched.

### 9.1 Point-by-point result

1. **No APIs were invented** — verified.
   - `launchProfile` does not exist in the codebase; the only references are
     comments/docs stating it must not be created. Profile "run" is the
     documented frontend loop over `executeAction` (`runAll()` in
     `devlauncher/profiles/[name]/+page.svelte`).
   - No arbitrary tool install: only `buildInstallPlan`/`runInstall` over the
     real environment-check requirements; no marketplace/plugin install.
   - No uninstall/update/upgrade action: `UpdateAvailable` is display-only
     (a status label in `toolchain/types.ts`; no action button anywhere).
   - No accumulated work time: only backend `SessionInfo.duration_secs` is shown.
   - No invented project statistics: the Workspace Overview stat strip is
     running/total/restarts/session-uptime, all derived from real
     `listProcesses()`/`getSessionInfo()` fields.
   - No persistent project registry: only UI-local recent project *references*
     (`stackpilot:recent:v1`), clearly labeled local history, never the backend
     current project.
   - No fake backend events: the typed event bus (`events.ts`) emits only after
     a confirmed backend call; in practice only `openProject` is wired to it.

2. **Real APIs and payloads preserved** — verified 1:1 against
   `src-tauri/src/lib.rs` + module `commands.rs`: DevLauncher 8/8, Workspace
   14/14, Toolchain 11/11 + 3 events, Project Creator 13/13, Core settings
   3/3. Backend-only commands without wrappers (`get_problems`,
   `clear_problems`, `get_workspace_overview`, `validate_project_stack_error`,
   feature-gated `mini_ide`) remain unwrapped and unused, as documented.

3. **Project Creator preserved** (`src/routes/create/+page.svelte`, ~3700
   lines, untouched) — verified all contract-G responsibilities:
   - wizard (constructor/presets/analyze modes), session
     (`createSession.ts` sessionStorage whitelist + `reSyncLiveSessions()`),
   - validation (frontend mirror `rules.ts` + backend `validate_project_stack`
     as final barrier in `confirmAll()`),
   - recommendation (`getStackRecommendations` wrapper exists, unused — as
     before), environment check (`checkEnvironment` + `check_progress` +
     `buildInstallPlan` → `runInstall` → `toolchain:task_event` /
     `toolchain:install_done` / `getNewSecrets`),
   - recipe preview (`previewProjectRecipe` wrapper exists, unused — as
     before), execution (`startProjectExecution` +
     `project_creator:step_event` + `project_execution_snapshot`),
   - event handling (`handleExecEvent`, `handleToolchainEvent`,
     `handleInstallDone`, `handleCheckProgress`).

4. **Legacy routes work** — `/analyze`, `/profiles`, `/profiles/[name]`,
   `/processes` are SPA redirects to `/devlauncher/*`; the `/processes`
   redirect preserves the query string and `/devlauncher/processes` honors
   `?log=<id>`. `/create` and `/environment` are the full legacy pages.
   Workspace empty states link to `/devlauncher`, `/create`,
   `/devlauncher/processes`.

5. **Cleanup / safety** — verified:
   - listeners: `/devlauncher/processes` resolves both listeners via
     `Promise.all` + `destroyed` flag and unsubscribes in `onDestroy`;
     `create/+page.svelte` unsubscribes all 4 handles (`unlisten`,
     `unlistenTc`, `unlistenTcDone`, `unlistenTcCheck`) in `onDestroy`;
   - timers: runtime/logs auto-refresh intervals and files `saveMsgTimer`
     cleared in `onDestroy`; `create` `stopTick()` on destroy; Toast timer
     cleared on unmount;
   - xterm disposed (`onDestroy` + `closeLogs`); CodeMirror `view.destroy()`
     in `onDestroy`;
   - localStorage: only the documented versioned keys; safe JSON parse +
     graceful fallback; no secrets/logs/snapshots; no "running" flags.
     `newSecrets` is in-memory only and excluded from the create-session
     whitelist;
   - loading/error/empty states exist on every workspace page, home and
     DevLauncher; no fake success — every success message follows an awaited
     backend call; no dead buttons found.

6. **Accessibility** — new shell/components are solid: global `:focus-visible`
   ring, global `prefers-reduced-motion` kill-switch, `Modal` (Escape,
   focus-in/out, `role=dialog`, `aria-modal`, `aria-label`), `Tabs`
   (`role=tablist/tab`, `aria-selected`), `ToastRegion` (`aria-live=polite`),
   toasts (`role=status`), keyboard support on the alt-framework chips.
   Remaining: **16 pre-existing svelte-check warnings** in the two preserved
   legacy files — `create/+page.svelte` (overlay/dialog `<div>` backdrops
   without keyboard handling or focus management; `role=dialog` without
   `tabindex`) and `devlauncher/analyze/+page.svelte` (drag&drop cards,
   unlabeled inputs). These predate stages 1–4, are documented in the contract
   (3.4/3.6) and are intentionally NOT touched (create page business logic must
   stay as-is; analyze page was preserved verbatim).

7. **Responsive layout** — verified: `auto-fit/minmax` grids throughout,
   workspace files/logs collapse to one column (`@media` ≤900px/760px), create
   builder collapses at ≤900px, xterm modal is `90vw`, toasts cap at
   `calc(100vw - 3rem)`, `PageContainer` widths. The shell sidebar is fixed
   width with the content pane scrolling (desktop-app layout).

8. **package.json scripts** — only real scripts run: `npm run check` →
   **0 errors, 16 warnings** (the pre-existing a11y warnings above);
   `npm run build` → **passes** (adapter-static SPA, writes `build/`).
   `dev`, `preview`, `check:watch`, `tauri` exist but were not run (no
   feature work to iterate on; nothing invented).

### 9.2 Implemented (re-confirmed across the app)

Stages 1–4 are all present and consistent with the contract: shell +
navigation, typed integration layer and UI-local stores, Home + unified
DevLauncher (canonical `/devlauncher` hub + legacy redirects), Workspace shell
with 7 real-data pages, and the honest assistant extension point. No invented
UI, no fake product claims (onboarding copy, demo profile card, assistant
panel).

### 9.3 Backend limitations (unchanged — must not be invented in the frontend)

- `launchProfile` — absent by design; profile run is a frontend loop over
  `executeAction`.
- Toolchain uninstall / update / upgrade of a single tool — no backend command.
- Arbitrary marketplace / plugin installation — no API.
- `get_workspace_overview` / `get_problems` / `clear_problems` — backend exists,
  no frontend wrapper; Workspace Problems stays a client-side derivation from
  `listProcesses()`.
- `validate_project_stack_error` — backend exists, no wrapper, unused.
- `project_creator:execution_done` — backend emits, frontend never listens.
- `execute_action` optional `session_id` — not exposed by the wrapper.
- Applying `AppSettings.theme` to the CodeMirror editor — editor theme is still
  mount-time `matchMedia` (`CodeEditor.svelte:42`), not reactive to theme
  changes.

### 9.4 Errors (open items, all pre-existing / non-blocking)

1. The contract §3.5 listener race in `create/+page.svelte` (an `await listen()`
   that resolves after the component unmounted is assigned to `unlisten` but
   never unsubscribed) is still present — the create page's event wiring was
   intentionally preserved as-is. Low impact (webview teardown cleans up);
   flagged for a future refactor of that page.
2. `secretCopied` and `tooltipTimer` timeouts in `create/+page.svelte` are not
   cleared in `onDestroy` (harmless; contract §3.5 already noted this).
3. `devlauncher/profiles/[name]/+page.svelte` "Open in Workspace" does not
   guard on `project_path` (unlike the Overview page); opening a pathless
   profile sets a current project with `project_path = null`, which the
   workspace handles gracefully (Files shows the "No project path" empty
   state). Minor inconsistency, preserved behavior.
4. `devlauncher/processes/+page.svelte` defines an unused `copyCommand()`
   helper (dead code, no UI impact).

### 9.5 Checks (real scripts)

- `npm run check` → 0 errors / 16 warnings (pre-existing, see 9.1.6).
- `npm run build` → passes, writes `build/`.

### 9.6 Recommendations for a future backend session

1. Provide a real `get_problems` wrapper (or document it) so Workspace Problems
   stops deriving client-side from `listProcesses()`.
2. Expose `execute_action`'s `session_id` so profile actions can be counted
   into the workspace session errors (semantics are already backend-owned).
3. Make the CodeMirror theme follow `AppSettings.theme` reactively (frontend
   change) once the shell theme layer is fully settled.
4. Consider a backend-pushed project-change event so the workspace context
   (`context.ts`) can react without a manual reload.
5. (Future refactor, not backend) Fix the `create/+page.svelte` listener race
   and dialog a11y (Escape/focus) when the page is next touched, without
   changing its business logic.
