# Toolchain Control Center — Implementation Contract

Status: **final v3** (acceptance stage — backend and frontend verified end-to-end;
§8 is the normative serialization truth; §9 is the normative compatibility appendix;
dead commands from §9.5 have been deleted).
Companion files: `docs/toolchain-architecture.md` (verified inventory, migration
matrix, issue status), `docs/toolchain-progress.md` (session log incl. final report),
`docs/toolchain-acceptance.md` (manual acceptance checklist).

This document defines the **target product** for the refactored Toolchain module of
StackPilot (Tauri v2 + Rust backend + Svelte 5 frontend). Everything marked **target**
is a requirement for future stages. Everything about the *current* code is documented
in `docs/toolchain-architecture.md` and was verified against the real sources.

Out of scope for the refactor session that produced this contract: redesigning Project
Creator, removing old commands, implementing the visual page.

---

## 1. Product definition

Toolchain is a **developer environment control center**. It answers four questions and
acts on them safely:

1. *What exists on this machine?* (catalog vs. live reality)
2. *What does this project need?* (requirements resolution)
3. *What is broken or risky?* (health, PATH, updates, permissions)
4. *What do you want me to do about it?* (approved install/update plans, jobs)

It has exactly **two primary modes**:

### 1.1 Mode: Build Environment

Purpose: make a chosen project stack buildable before generation.

Flow:

1. User selects **languages, frameworks, project types, databases, infrastructure,
   containers, cloud tools, testing, desktop/mobile/game requirements**.
2. The **backend** resolves a canonical *environment profile* from that selection
   (the frontend selection is an input, never an authority).
3. The UI renders every resolved tool classified into groups:
   **required**, **recommended**, **optional**, **Docker-managed**, **manual**,
   **unsupported**, **conflicting**.
4. The backend builds and validates a canonical **install plan**; the user inspects
   and approves it before anything runs.
5. Execution happens through the same job system as Manage Everything (identity,
   progress, cancellation, terminal states).

Rules:

- The resolved profile must be reproducible from `(selection, catalog version)` —
  the same input always yields the same requirement set.
- Docker-managed tools default to container execution; local installation is an
  explicit opt-in (this mirrors today's `local_infra_tools` behavior, which must keep
  working for Project Creator).
- Manual-only tools (engines, SDKs) never enter an auto-install plan; they surface as
  manual instructions.
- A scan inside Build Environment mode must be read-only (see §5).

### 1.2 Mode: Manage Everything

Purpose: full control over every supported tool, independent of any project.

Contents:

- **Complete catalog** of all supported tools (today: `tools.json`, 48 entries).
- **Live detection state** per tool (what discovery actually finds right now).
- **Health state** (health checks from the catalog, daemon liveness, etc.).
- **Updates** (installed version vs. recommended version; update action).
- **PATH state** (which entries exist, which are broken, fix action).
- **Managed/unmanaged state** (installed by StackPilot vs. pre-existing/user-installed).
- **Manual installation instructions** for non-installable tools.
- **Docker capability** (can this tool run containerized instead of host-installed).
- Actions: **install / update / recheck / fix-PATH** — each is a job with identity,
  progress, cancellation and terminal state.

---

## 2. Independent dimensions (no boolean collapse)

The current `ToolStatus` enum conflates several orthogonal facts. The target system
must track these dimensions **independently**; no single boolean or single enum may
substitute for them:

| # | Dimension | Question it answers | Examples |
|---|---|---|---|
| D1 | Catalog state | Is this tool defined? | present / deprecated / unknown id |
| D2 | Live detection state | What did discovery find just now? | not found / found binary / found install footprint |
| D3 | Installation provenance | Who put it there? | StackPilot-managed / manually installed / OS-provided (built-in) |
| D4 | Health state | Does it actually work? | healthy / unhealthy / unknown / not checked |
| D5 | Version & update state | Is the version acceptable? | ok / below-min / update-available / unparseable |
| D6 | Platform capability | Is it installable/supported here? | supported / unsupported-on-platform / manual-only |
| D7 | Execution mode (Docker/host) | Where does it run? | host / docker-managed / either |
| D8 | Permission requirement | What does installation require? | user-scope / admin-elevation required |
| D9 | Dependency state | What does it need to function? | self-contained / bundled-with X / requires runtime Y / satisfied |

A tool view-model is the **composition** of these dimensions, not one flag. Example:
“git” may simultaneously be `detected` (D2), `manually installed` (D3), `healthy`
(D4), `update available` (D5), `supported` (D6), `host` (D7), `user-scope` (D8),
`satisfied` (D9). Rendering collapses dimensions into a *presentation status*; storage
and logic never do.

---

## 3. Conceptual status model (target)

Presentation-level statuses derived from §2 dimensions. All of these must be
representable; the list is the minimum:

| Status | Meaning (dimension composition) |
|---|---|
| `missing` | Not detected on machine; installable here. |
| `installed-healthy` | Detected, provenance recorded, health checks pass, version ≥ recommended. |
| `installed-health-unknown` | Detected but health never checked (or check unavailable). Distinct from healthy — “we don't know” ≠ “ok”. |
| `installed-unhealthy` | Detected but at least one health check fails. |
| `update-available` | Installed, works, version < recommended (or < min → stronger variant). |
| `path-broken` | Install footprint found but binary not reachable/responding via PATH; fix-PATH applies. |
| `manually-installed` | Detected; provenance says user/external install; managed actions limited. |
| `docker-managed` | Tool is provisioned via Docker/docker-compose instead of host install (dual tools: postgresql, redis, mongodb, kafka, grafana, mysql…). |
| `built-in-system` | OS-provided/bundled (curl, tar on Windows 10+, shell utilities); nothing to install. |
| `unsupported-platform` | Catalog says this platform cannot install/run it (e.g. msvc-build-tools on Linux, xcodebuild off macOS). |
| `install-unavailable` | Would be relevant, but no usable source exists on this platform (no source, source broken/disabled). |
| `scan-failed` | Detection for this tool errored or hit the deadline — result is *unknown*, explicitly not “missing”. |
| `scan-pending` | Scan queued/in progress; cached value shown until replaced. |

Additional derived flags (not statuses): `needs-admin`, `bundled-with`,
`conflicts-with`, `size-known`, `stale-data`.

Rules:

- Every catalog entry always has a status; `scan-pending`/`scan-failed` are first-class.
- Status transitions happen only from backend-computed facts; the frontend may filter
  and group but may not *decide* a status.
- `missing` must never be displayed when the true result is `scan-failed`.

---

## 4. Target UX structure

Single module surface (“Toolchain Control Center”), responsive, theme-compatible.

### 4.1 Overview dashboard

- Environment summary card: OS/os-version, package managers, disk headroom, admin
  availability, overall health score, counts per status.
- Health summary (healthy / unhealthy / unknown / unchecked).
- Disk/admin/system summary block (free space on install root, elevation capability,
  OS details from `EnvironmentInfo`).
- Entry point buttons into both modes.

### 4.2 Automatic read-only diagnostic scan on entry

- On opening the control center, a **read-only scan starts automatically**.
- **Cached state is rendered immediately** when available (from persisted snapshot);
  live results replace it per-tool as they arrive.
- Live scan progress is visible (per-tool rows: pending → scanning → result), with
  cancel support for the scan itself.
- Scans mutate nothing on the machine (§5) and may persist only their own snapshot.

### 4.3 Build Environment mode

- Requirement picker fed by the Project Creator-compatible selection model
  (languages/frameworks/project types/databases/infrastructure/containers/cloud/
  testing/desktop-mobile-game).
- Resolved profile grouped: required / recommended / optional / Docker-managed /
  manual / unsupported / conflicting.
- Plan review screen: exact tasks, sizes, admin warnings, execution mode; explicit
  approve step; then jobs run through the job center.

### 4.4 Manage Everything mode

- Searchable full catalog (search by name/id/category).
- Filters: category, status, platform capability, provenance, execution mode.
- Per-tool actions: install / update / recheck / fix-PATH (availability derived from
  §2 dimensions; disabled actions explain why).

### 4.5 Tool detail drawer

Side drawer (modal on narrow screens) per tool: description, notes, detection result,
version(s), health check results, PATH entries, provenance, install sources for this
platform (descriptions only), manual instructions when applicable, Docker option,
history/log excerpt, actions.

### 4.6 Job center & activity panel

- All long-running operations are **jobs**: stable job id, kind (install/update/recheck/
  fix-path/scan), per-task breakdown, progress, cancellable while running, terminal
  state (`success`/`failed`/`cancelled`/`partial`) and recovery hint after restart.
- Activity/log panel streams job output lines (the existing `tc:*` line protocol moves
  into structured events over time; see architecture doc for the migration path).

### 4.7 Cross-cutting UX requirements

- Responsive layout (sidebar content collapses; drawer becomes modal).
- Dark and light theme compatibility via the app's `--sp-*` design tokens
  (`src/lib/components/ui/*`); no hardcoded palette-only styling.
- Full empty / loading / error / stale / cancelled states everywhere:
  - empty: no tools match filters; nothing installed;
  - loading: first-run scan with zero cache;
  - error: backend command failed (retry offered);
  - stale: cached snapshot older than N or from previous app run, labeled with age;
  - cancelled: scan/job cancelled shows terminal state, not an error.
- Navigation: the canonical route `/toolchain` must exist; legacy `/environment` keeps
  working (redirect or alias). Nav item already matches both (see architecture doc §3.9).

---

## 5. Non-negotiable safety rules

These hold for every stage of the refactor. Violations are bugs even if convenient.

1. **Scans never mutate the machine.** Detection, health checks, plan building and
   diagnostic scans must not install, upgrade, configure, or write outside their own
   state store. (Today's `ensure_maui_workload()` inside environment checking violates
   this and must be moved out of scan paths into explicit, user-approved jobs.)
2. **Backend builds and validates canonical install plans.** The plan is derived from
   catalog + fresh detection server-side. The frontend may select/deselect tools; it
   cannot inject sources, URLs, arguments, versions or task definitions. Plans carry a
   fingerprint of the inputs they were built from and are re-validated at execution.
3. **The frontend is not an authority** for tool metadata, source URLs, arguments,
   versions, or task state. It renders backend facts and sends selections/intents.
4. **Secrets are not returned** in normal metadata or diagnostic responses. Generated
   secrets (DB passwords) are delivered once through an explicit, dedicated channel
   (keep-and-clear semantics like today's `tc_take_new_secrets`), never embedded in
   metadata snapshots, sessions, logs, or events.
5. **Archive extraction is traversal-safe.** Any unpacking (zip/tgz/7z) validates entry
   paths against the destination root (no absolute paths, no `..` escape, symlink
   policy explicit) before writing.
6. **Downloads have integrity validation.** Every downloadable source gains a checksum
   (sha256) or signed-manifest verification in the catalog; mismatch aborts the task
   before execution. Until a source provides one, the task must warn and require
   explicit user confirmation.
7. **PATH mutations are explicit and auditable.** Adding/removing PATH entries happens
   only inside approved jobs, records before/after entries in job output and in
   persistent metadata, and is reversible (remove exactly what we added — the
   StackPilot-marked set).
8. **All long-running operations have job identity, progress, cancellation, terminal
   state, and recovery behavior.** One global job registry replaces ad-hoc session
   slots; a restart mid-job yields a deterministic recovery state (resume-or-failed),
   never a stuck “running” flag.
9. **Project Creator remains functional and uses the same backend domain engine.**
   The wizard's environment step consumes the same profile resolution, planning, and
   job APIs as the control center; `tc_*` legacy commands stay until migration
   completes (compatibility matrix in architecture doc).

---

## 6. Target backend contract (shape, not final signatures)

New-versioned commands are additive (`tcx_` prefix suggested); legacy `tc_*` commands
remain during migration. Exact naming is decided at implementation time; the *shapes*
below are contractual.

### 6.1 Commands (target)

| Group | Command (illustrative) | Purpose |
|---|---|---|
| Catalog | `catalog_list` | Full catalog with platform capability + dependency info. |
| Profile | `profile_resolve(selection) -> EnvironmentProfile` | Canonical profile: required/recommended/optional/docker/manual/unsupported/conflicting groups. |
| Scan | `scan_start(scope) -> job_id`; `scan_status(job_id)` | Read-only diagnostic scan; per-tool streamed results; cancellable. |
| Snapshot | `snapshot_get()` | Last persisted scan snapshot (+ staleness metadata). Never contains secrets. |
| Plan | `plan_build(profile_or_selection, chosen_ids?) -> InstallPlan` | Backend-canonical plan incl. sizes, admin flags, execution modes. |
| Jobs | `job_start(plan_or_action) -> job_id`; `job_status(job_id)`; `job_cancel(job_id)`; `job_list()` | Unified job registry for install/update/recheck/fix-path/scan. |
| Health | `health_check(tool_ids) -> ...` | On-demand health run as part of scan/job system. |
| Path | `path_report()`; fix-PATH via `job_start` | Read-only report; mutations only as audited jobs. |
| Secrets | `secrets_take_once()` | Only secret-revealing call; clears on read. |

### 6.2 Events (target, names illustrative)

| Event | Payload shape | Notes |
|---|---|---|
| `toolchainx:job_event` | `{job_id, kind, task_index, total, task, phase, line?, ts}` | Replaces `toolchain:task_event` over time. |
| `toolchainx:job_done` | `{job_id, summary}` | Terminal snapshot. |
| `toolchainx:scan_progress` | `{done, total, tool_id, display, icon, outcome}` | Per-tool read-only scan result. |

Legacy event names (`toolchain:check_progress`, `toolchain:task_event`,
`toolchain:install_done`) continue to be emitted while Project Creator uses them.

### 6.3 Persistence (target)

- `app_data_dir/toolchain/state.json` remains the store, extended with:
  - per-tool provenance (`stackpilot` / `external` / `system`),
  - last scan snapshot (statuses per tool, timestamped, versioned schema),
  - PATH audit trail (entries added by us, when, by which job),
  - job history (bounded ring),
  - schema `version` field with forward-compatible defaults.
- Secrets stay in a separate section, excluded from every read/model except the
  take-once channel.
- Frontend caching mirrors the snapshot (render-before-scan), keyed by snapshot id.

---

## 7. Compatibility constraints (hard requirements)

1. **Old commands stay.** All eleven registered commands (list in architecture doc
   §3.2) keep working unchanged until every consumer is migrated; removal is a
   separate, explicit stage.
2. **Old events keep flowing** alongside new ones during migration.
3. **Project Creator stays functional end-to-end**: wizard → environment check →
   plan approval → install → generate. Its `ProjectRequirements` payload (languages,
   frameworks, tools, `local_infra_tools`, git/vscode/docker flags) remains accepted.
4. **Dual docker tools semantics preserved**: default docker-compose deployment,
   opt-in local install, exclusion from generated compose file, LOCAL_INFRA.md guide.
5. **state.json stays readable** across the refactor (additive fields only; missing
   fields default).
6. **Windows-first behavior preserved** where it exists today; Linux/macOS gaps are
   tracked, not regressed further.

---

## 8. Final JSON serialization contract (implemented — domain model stage)

This section is **normative** for every new (`tcx_*`) API payload. It describes the
serde shapes as implemented in `src-tauri/src/modules/toolchain/domain/models.rs`
and `domain/profile.rs`; tests in those files assert these exact encodings.

### 8.1 One tagged representation for all data-carrying enums

Every new enum that has at least one variant with data uses **internal tagging**
with a `"kind"` discriminator and `snake_case` names:

```json
{ "kind": "update_available", "installed": "1.9", "recommended": "1.10" }
```

Unit variants serialize as single-field objects — never as bare strings:

```json
{ "kind": "missing" }
```

Frontend rules:

- discriminate by reading `kind`; never by shape-sniffing (object vs string);
- unknown `kind` values must render as "unknown state", not crash.

Enums that are unit-only (no data) serialize as plain `snake_case` strings
(e.g. `PathFindingKind`: `"stale_entry"`). This is unambiguous because such enums
have no object form.

Enums covered by the tagged rule: `ToolState`, `DetectionOutcome`, `EvidenceKind`,
`VersionAssessment`, `Provenance`, `PlatformApplicability`, `HealthState`.

**Job-engine exception (normative).** The `engine/*` job family
(`OperationKind`, `TaskAction`, `NoopReason`, `Phase`, `JobStatus`,
`EngineTaskStatus`) serializes as **externally tagged snake_case**
(`{"already_installed": {"version": "1.2"}}`, `"docker_managed"`,
`{"running": {"phase": "downloading"}}`). This deviates from the blanket
`kind`-rule above deliberately: the engine predates it, its encoding is frozen
by persisted records (`jobs/*.json`) and consumed as-is by the TS mirrors.
Rule of thumb: *domain enums → internal `kind` tagging; engine enums →
external tagging*. Both are mirrored exactly in `types.ts`.

### 8.2 Catalog: extended metadata (tolerant)

`ToolDefinition` gains a flattened, fully-defaulted metadata block
(`ToolExtendedMetadata`). Old catalog entries without these keys stay valid; an
absent key means **not declared**, never "guessed yes":

```json
{
  "id": "postgresql",
  "...": "...",
  "aliases": ["pg"],
  "dependencies": [],
  "conflicts": [],
  "docs_url": "https://www.postgresql.org/docs/",
  "source_url": null,
  "platform_availability": ["windows", "linux"],
  "declared_capabilities": { "removable": true, "repairable": false },
  "docker": { "image": "postgres:17", "notes": "WSL2 required" }
}
```

- `aliases`/`dependencies`/`conflicts`/`platform_availability`: omitted when empty
  (`skip_serializing_if`);
- `declared_capabilities.removable/repairable`: `Option<bool>` — missing = catalog
  says nothing; capability consumers must treat absence as `false` + reason
  "not declared".

### 8.3 Dimensions

**Provenance** (who put the tool there):

```json
{"kind": "stack_pilot_managed"}
{"kind": "external"}
{"kind": "package_manager"}
{"kind": "system"}
{"kind": "bundled_with", "tool": "node"}
{"kind": "docker"}
{"kind": "unknown"}
```

**Detection outcome** (live scan of one tool; `failed` ≠ `not_detected`):

```json
{"kind": "pending"}
{"kind": "not_detected"}
{"kind": "failed", "reason": "probe timeout"}
```

Per-install evidence (`DetectedInstall.evidence`, `EvidenceKind`):
`{"kind":"version_probe"} | {"kind":"known_path"} | {"kind":"footprint"}`;
duplicates are represented by multiple entries in `ToolScanResult.installs`;
conflicting versions = >1 distinct `parsed_version` among installs.

**Version assessment** (advisory policy):

```json
{"kind": "unknown"} {"kind": "unparseable"} {"kind": "meets_recommended"}
{"kind": "below_recommended"} {"kind": "below_min"} {"kind": "policy_violation"}
```

`policy_violation` = the installed version cannot be honestly judged because the
catalog policy itself contradicts itself (min > recommended).

**Health state** — all nine states are representable:

```json
{"kind": "not_checked"}      // no data at all yet this session
{"kind": "checking"}         // scan is running checks right now
{"kind": "healthy"}          // all declared checks passed
{"kind": "degraded"}         // checks passed, but version < recommended/min
{"kind": "unhealthy"}        // ≥1 check ran and failed its condition
{"kind": "unavailable"}      // tool absent/path broken → checks not applicable
{"kind": "unsupported"}      // platform cannot run the tool at all
{"kind": "no_checks_defined"}// catalog declares no health_checks
{"kind": "failed_to_run"}    // check process could not be executed/timeout
```

Each check result preserves label, verdict, sanitized detail, duration and a
process-failure flag:

```json
{ "label": "daemon responsive", "passed": true, "process_failed": false,
  "detail": "Docker version 27.x", "duration_ms": 412 }
```

**Platform capabilities** — eight independent booleans per tool
(`ToolScanResult.capabilities`):

```json
{ "detectable": true, "installable": true, "updatable": true, "removable": false,
  "repairable": false, "health_checkable": true,
  "manual_instructions_available": false, "docker_alternative_available": true }
```

Derivation rules (documented in code): `removable`/`repairable` come **only** from
explicit catalog declarations; `installable` requires an OS source + implemented
execution backend + not manual-only.

### 8.4 Presentation status composition (`ToolState`)

```json
{"kind": "scan_pending"}
{"kind": "scan_failed", "reason": "..."}
{"kind": "missing"}
{"kind": "installed_healthy", "version": "22.12.0"}
{"kind": "installed_health_unknown", "version": "22.12.0"}
{"kind": "installed_unhealthy", "version": "22.12.0"}
{"kind": "update_available", "installed": "18.19", "recommended": "22"}
{"kind": "path_broken", "reason": "..."}
{"kind": "manual_install", "reason": "..."}
{"kind": "docker_managed"}
{"kind": "built_in_system"}
{"kind": "unsupported_platform"}
{"kind": "install_unavailable"}
```

Composition order (implemented in `domain/detect.rs::compose_state`):
scan failure wins over everything → manual/docker/built-in classification →
footprint-only = path-broken → below-min/below-recommended = update-available →
health verdict → healthy/health-unknown. `scan_pending`/`scan_failed` are
first-class; `missing` must never stand in for them.

### 8.5 Environment snapshot

`EnvironmentSnapshot` (fields added in this stage are defaulted, so cached
snapshots from earlier builds deserialize):

```json
{
  "snapshot_id": "tcx-snap-…", "job_id": "…", "scan_id": "…",
  "os": "windows", "os_version": "Microsoft Windows 11 …", "arch": "x86_64",
  "package_managers": ["winget", "choco"],
  "disk": [{ "root": "C:\\", "free_mb": 123456 }],
  "admin": { "elevation_supported": true, "required_by_tools": true },
  "started_at": "…", "finished_at": "…",
  "complete": true, "cancelled": false,
  "tools": [ /* ToolScanResult, catalog order */ ],
  "path_report": { "entries": [], "findings": [] },
  "score": { "score": 87, "counted_tools": 30, "healthy_required": 26, "...": 0 },
  "summary": { "missing": 3, "update_available": 2, "installed_healthy": 26, "...": 0 },
  "warnings": [{ "code": "scan_cancelled", "message": "…" }],
  "errors":   [{ "code": "partial_scan", "message": "…" }],
  "active_jobs": ["tcxj-…"],
  "age_seconds": 12, "stale": false, "from_cache": false
}
```

Score formula (unchanged, documented in `domain/score.rs`):
denominator = applicable required tools only;
healthy=100, degraded=50, missing/broken/unhealthy=0;
unchecked / scan-failed / optional / docker / manual / built-in / not-applicable
are excluded from numerator and denominator.
`score = round(Σ points / counted_tools)` clamped to 0..100; empty catalog → 0.

### 8.6 Environment profile (Build Environment mode)

`EnvironmentProfile` — pure function of `(selection, catalog, OS)`:

```json
{
  "created_at": "…", "os": "windows",
  "requirements": { /* canonical request echo — see §9.3; wire shape
                       identical to the legacy wizard payload */ },
  "required":     [ /* ProfileTool */ ],
  "recommended":  [],                       // reserved: no catalog metadata yet
  "optional":     [ /* dual docker tools, local opt-in possible */ ],
  "docker_managed": [ /* wizard docker tools relevant to selection */ ],
  "local_alternatives": ["postgresql"],     // explicit user opt-ins
  "manual":       [ /* engines/SDKs */ ],
  "unsupported":  [ /* no source for OS / unknown id */ ],
  "conflicts":    [{ "tool": "a", "conflicts_with": "b" }],   // declared only
  "dependency_closure": ["winget", "node", "npm"],
  "estimated_download_size_mb": 1234,       // Σ size_mb over `required`
  "warnings": [{ "code": "admin_required", "message": "…" }],
  "readiness": { "required_total": 12, "satisfied_count": null, "all_ready": false }
}
```

Resolution reuses `core::requirements::resolve` unchanged — Project Creator's
`ProjectRequirements` behavior is preserved by construction (winget-first order,
dual-docker opt-in semantics, bundled-tool dropping happen inside that engine).
`satisfied_count` stays `null` until a snapshot is applied via
`EnvironmentProfile::apply_snapshot`.

### 8.7 Jobs

Two job families exist; both carry full operation identity:

- **Engine jobs** (`engine/jobs.rs::PersistedJob`, operations
  `install | update | repair_path | health_check`): stable `job_id`, timestamps
  (created/started/updated/finished), typed `JobStatus`
  (`queued/running/succeeded/partial/failed/cancelled/interrupted`), requested
  tool ids, full task list inside `CanonicalPlan`, audited `path_changes`,
  `recovered` flag after restart, bounded history on disk (`jobs/*.json`),
  one active mutating job at a time. Secrets have no field in the record
  (`assert_secret_free` guard).
- **Scan jobs** (`domain/engine.rs::ScanJobSnapshot`, operation `scan`): job_id +
  scan_id, phase (`queued/environment/tools/finalizing/done`), progress counters,
  `cancel_requested`, terminal
  (`running/completed/partial/cancelled/failed/interrupted`), journal-based
  restart recovery to `interrupted`.

Progress/cancellation are represented as typed states and counters on the records —
there is no global boolean for "installation in progress"; legacy
`InstallSession.running` survives only inside the old `tc_*` façade.

Scan-family enums serialize as **PascalCase strings** (`ScanPhase`:
`"Queued"/"Environment"/"Tools"/"Finalizing"/"Done"`; `ScanTerminal`:
`"Running"/"Completed"/"Partial"/"Cancelled"/"Failed"/"Interrupted"`) — mirrored
exactly by the TS types and translated to human labels only in `format.ts`.

**Adoption surface.** `tcx_adopt_tool(tool_id)` records an explicit
track-mark (`state.json: adopted`, exposed via `ToolchainMetadataView.adopted`
of `tc_get_metadata`). Adoption never claims StackPilot provenance — scans keep
reporting `external`; the Control Center drawer shows an «отслеживается» badge
and offers the action only for external/unknown provenance.

### 8.8 Compatibility adapters

- Legacy `ToolStatus` (externally tagged PascalCase, e.g.
  `{"Installed":{"version":"…"}}`) remains byte-compatible for all eleven `tc_*`
  commands. Mapping both directions between `ToolState` and `ToolStatus` lives in
  `domain/detect.rs` (`to_legacy_status`) and is covered by table tests.
- New APIs must never emit the legacy encoding; old APIs never emit the `kind`
  encoding. No mixed encodings within one payload.

---

## 9. Project Creator compatibility layer (implemented — normative)

Project Creator runs **on the same canonical domain engine** as the Control Center;
there is no second toolchain implementation. The wizard speaks the legacy protocol,
which is a set of thin adapters over the canonical engine. This section documents
the adapters that MUST keep working until the wizard migrates to `tcx_*`.

### 9.1 Compatibility commands (wire-compatible surface)

| Legacy command | Implementation today | Canonical backing |
|---|---|---|
| `tc_check_environment(requirements)` | unchanged legacy check path (`core::check::run_check`, read-only; emits per-tool progress) | same catalog + discovery as the scan engine; no machine mutation |
| `tc_build_install_plan(check, selected)` | takes only tool ids + install options from the payload | plan content rebuilt from catalog (`core::planner::canonicalize_plan`); unknown ids rejected |
| `tc_run_install(plan)` | **adapter over the canonical job engine**: from the payload only task ids + Qt options are read (`force_reinstall=true` reproduces wizard semantics; unverified-source/UAC confirmations implied by the wizard's Install button); backend rebuilds the canonical plan fresh, persists it before execution, executes through the engine; legacy session slot + `jobs.json` maintained | `engine/` pipeline (jobs, phases, cancellation, restart recovery, integrity gates) |
| `tc_get_install_status()` | legacy in-memory session slot | reflects the engine-run legacy plan; secrets never serialized (`#[serde(skip)]`) |
| `tc_abort_install()` | sets legacy abort flag **and** cancels the active engine job (kill current task, remaining → Cancelled) | engine cancellation |
| `tc_take_new_secrets()` | one-shot take-and-clear of the pending map | secrets live only in the isolated `SecretStore`; never in metadata/status/jobs/events/logs |
| `tc_get_metadata()` / `tc_get_health_report()` | sanitized view only | dual-docker tools stay hidden until locally installed |

Not used by Project Creator but part of the compat surface: `ping_toolchain`,
`tc_get_tool_definitions`, `tc_get_environment_info`.

### 9.2 Compatibility event mappings

Legacy events continue to be emitted for every wizard-driven run; new events flow
in parallel:

| Legacy event (wizard listens) | Source today | Mapping rules |
|---|---|---|
| `toolchain:task_event` `{event_type,…,session_id}` | bridge sink attached to the engine job for the duration of `tc_run_install` (removed at finalize) | `TaskStarted→index/total cursor`; phase mapping: validating/preparing/downloading → `Downloading`, verifying/checking_health → `Verifying`, installing/configuring → `Installing`, updating_path → `UpdatingPath`; terminal math: Succeeded→`Success{version}`, Failed→`Failed{error}`, Cancelled/Interrupted→`Skipped{reason}`; final aggregate event `AllCompleted{success_count, failed[]}` emitted at finalize. `session_id = engine job_id`. |
| `toolchain:install_done` `{plan}` | emitted once after engine finalize | payload is the legacy projection of the persisted engine record (`legacy_plan_from`): `session_id = job_id`, tasks in engine order (winget-first preserved), sizes/admin/source descriptions from catalog |
| `toolchain:check_progress` `{done,total,tool_id,…,scan_id}` | legacy check path | carries a per-run random `scan_id`; consumers must drop events whose `scan_id` differs from the established current run |

New-family events (`toolchainx:job_event`, `toolchainx:scan_progress`,
`toolchainx:scan_done`) always carry full operation identity; wizard state must
filter foreign identities via the shared pure rule `identityMatches(current,
incoming)` (empty incoming = accept for legacy backends; empty current = accept
and establish identity on first event).

Wizard-side stale-event contract (tested): after `tc_run_install` resolves, the
wizard re-reads `tc_get_install_status` and adopts the authoritative
`session.plan.session_id` (= engine `job_id`) before filtering subsequent events;
a late/foreign `install_done` or `task_event` never mutates task-state maps.

### 9.3 Requirements mapping (explicit, tested)

`ProjectRequirements` (wizard payload) → `EnvironmentProfileRequest` (canonical,
`domain/profile.rs`) — field-by-field explicit conversion, no logic duplication:
resolution still runs through the single-source `core::requirements::resolve`.
Semantics pinned by tests:

- `languages` → language toolchains (typescript/javascript → node; python → python);
- `frameworks` → framework build tools + missing language runtimes + Qt UI options
  (`fastapi` → python stack; `react`/`nextjs` → node/npm);
- `tools` → explicit wizard tool selection; **dual docker tools without opt-in are
  ignored here**;
- `local_infra_tools` → local-install opt-in: only this moves postgresql/redis/
  mongodb/kafka/grafana/mysql from *optional/docker* into *required*;
- `git_init` / `vscode_config` / `docker` → git / vscode / docker required tools.

The resolved profile must keep required tools strictly separated from
Docker-optional tools exactly as the wizard renders them (requirements list vs.
optional “Run in Docker” section). Mapping order-preservation + dedupe semantics
are regression-tested (`domain::profile::tests::mapping_*`).

### 9.4 Migration rules

1. Adapters may be rewritten internally, but command names, argument shapes,
   response fields, event names and payload fields above are frozen while any
   consumer exists.
2. No new behavior may be implemented twice (once legacy, once canonical): fixes
   go into the canonical engine; adapters project it.
3. Frontend consumers import exclusively through `src/lib/modules/toolchain/compat.ts`;
   direct `api.ts` imports in the create page are not allowed.
4. Secrets remain take-once-only; nothing secret-bearing may appear in drafts,
   snapshots, status payloads, events or logs (wizard restores never re-inflate
   secrets from sessionStorage).

### 9.5 Deprecated APIs & removal criteria

| API | Status | Removal criteria (all must hold) |
|---|---|---|
| `tc_build_install_plan` + `tc_run_install` pair | deprecated, adapter-backed | create page starts installs via `tcx_start_job` with `EngineRequest`; parity tests prove identical task sets/events for all wizard scenarios |
| `tc_get_install_status` / legacy session slot + `jobs.json` | deprecated | job center UI fully replaces wizard status polling; journal readers migrated to engine records |
| `toolchain:task_event` / `toolchain:install_done` bridges (`LegacyTaskEventSink`) | deprecated | wizard consumes `toolchainx:job_event` (+terminal guard) instead of the legacy stream |
| `toolchain:check_progress` | deprecated when Build Environment profile+scan replaces the wizard's ad-hoc check | wizard environment step renders from `tcx_profile_resolve` + snapshot |
| ~~`tc_create_install_plan`, `tc_get_platform_capabilities`~~ | **deleted (acceptance stage)** — were unregistered dead code with no frontend wrappers; removal complete per the cleanup criterion below |

Removal itself is an explicit, separate stage (§7.1): a stage that deletes a
command must also delete its frontend wrappers in `api.ts`/`compat.ts` and update
this document. The acceptance stage performed exactly this deletion for the two
unregistered commands; both had zero frontend references.
