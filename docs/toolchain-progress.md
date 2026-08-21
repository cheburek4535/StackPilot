# Toolchain Control Center — Progress Log

Session: **Stage 0 — audit & contract** (2026-08-21).
Scope of this session: read-only inspection of the repository + creation of the
implementation-contract documentation. No source code was modified.
Companion files: `docs/toolchain-contract.md`, `docs/toolchain-architecture.md`.

---

## Stage 1 — backend domain model (2026-08-21, same day)

Scope: canonical backend domain model for the Toolchain Control Center, built
**inside** the existing toolchain module (`domain/`, `engine/`, additive changes to
`models.rs`/`core/`). No new UI; no new top-level module. Note: parts of this stage
(`engine/` pipeline, scan engine, PATH report) were already present in the working
tree as uncommitted work-in-progress when the session started; this session fixed
the mid-refactor compile breakage and completed the model per the task list.

### 1. Models added / changed

| Area | Change |
|---|---|
| `models.rs::ToolDefinition` | **additive flattened metadata block** `extended: ToolExtendedMetadata` — `aliases`, `dependencies`, `conflicts`, `docs_url`, `source_url`, `platform_availability`, `declared_capabilities { removable?, repairable? }`, `docker { image?, notes? }`. All defaulted → old `tools.json` entries load unchanged; absent capability = "not declared", never guessed |
| `models.rs` enums | explicit `Default` semantics: `PlatformCapabilities` (derive), `InstallSessionStatus` (`#[default] Running` — old journals recover to Interrupted), `HealthState` (`#[default] NotChecked`); `ProjectRequirements` gains `PartialEq/Eq` |
| `mod.rs::ToolchainState` | new fields `journal: Arc<JobJournal>` + `secrets: Arc<Mutex<SecretStore>>` with accessors (fixes missing `state.journal()`/`state.secrets()`); startup wiring: legacy plaintext secrets migrate state.json → SecretStore; install journal recovers Running → Interrupted on boot; `EnvironmentInfo.capabilities` now actually populated |
| `domain/models.rs` | **serialization contract normalized**: all data-carrying domain enums are internally tagged `{"kind": …}` snake_case (`ToolState`, `DetectionOutcome`, `EvidenceKind`, `VersionAssessment`, `Provenance`, `PlatformApplicability`, `HealthState`) — no more mixed string/object encodings; `Provenance` extended to 7 kinds (+`package_manager`, `bundled_with{tool}`, `docker`); `HealthState` extended to the 9 contract states (+`not_checked/checking/degraded/unhealthy/unsupported/failed_to_run`; coarse `Checked` replaced by real verdicts); `VersionAssessment` += `policy_violation` (self-contradictory min>recommended catalog policy); new `ToolPlatformCapabilities` (8 independent flags, explicit-declaration-only for removable/repairable); snapshot extensions: `arch`, `disk[]`, `admin`, `summary: StatusCounts` (per-state counts), `warnings[]`/`errors[]` (`SnapshotIssue{code,message}`), `active_jobs[]` — all serde-defaulted so old cached snapshots still deserialize |
| `domain/profile.rs` | **new** — Build Environment profile (`EnvironmentProfile`): groups required/recommended(reserved-empty)/optional/docker_managed/local_alternatives/manual/unsupported, declared-only conflicts, dependency closure (bundled hosts + declared deps, cycle-safe), size estimate, admin warnings, readiness with `apply_snapshot()`. Resolution delegates to existing `core::requirements::resolve` — Project Creator compatibility by construction, zero logic duplication |
| `domain/detect.rs` | health verdicts: Healthy/Degraded/Unhealthy/**FailedToRun** (process-failure ≠ condition-failure); Degraded = checks pass but version below recommended/min; provenance facts only (BundledWith names host, dual-tool-not-found=Docker, PackageManager never guessed); capabilities attached to every `ToolScanResult` |
| `domain/engine.rs` | snapshot assembly fills arch/disk/admin/summary/warnings/errors/active_jobs; disk probe failure becomes a warning, not a failed scan; deadline/cancel produce typed issues |
| `core/installer.rs` | archive prevalidation (zip-slip fail-closed via shared `archive::prevalidate_archive`) wired into `try_install_source` before any unpack script runs — contract rule §5.5 now holds on the main install path too (qt path was already safe) |

### 2. Compatibility adapters

- Legacy `ToolStatus` ↔ new `ToolState`: `domain/detect.rs::to_legacy_status`
  (+ existing compose rules); table-tested both ways; old `tc_*` payloads unchanged.
- Old cached snapshots (`scan-snapshot.json`) without new fields deserialize with
  honest defaults (test: `snapshot_new_fields_survive_round_trip_and_default_for_old_files`).
- Old jobs journals without `status` default to Running and are recovered as
  Interrupted at startup (never fake success).
- `ProjectRequirements` payload untouched; profile engine consumes it through the
  legacy resolver.

### 3. Tests added / updated (all in-module `#[cfg(test)]`)

- Serialization round trips & tagged-form assertions: `ToolState`, all 7
  provenance kinds (incl. bundled payload), `DetectionOutcome`, 9 health states,
  `policy_violation`, `PlatformApplicability`, snapshot legacy-field tolerance,
  profile round trip.
- Status mapping: legacy coverage table (detect.rs) + composition order tests.
- Version states: min/recommended boundaries, unparseable, **policy violation**,
  degraded(update-available) presentation.
- Health semantics: no-checks ≠ unhealthy, FailedToRun ≠ Unhealthy (unknown, not
  broken), degraded on passing checks + stale version, process-failure flag.
- Provenance: bundled-with host naming, docker provenance for dual tools not found
  locally, managed/system/external precedence.
- Unsupported capabilities: empty catalog → all-false; explicit-only
  removable/repairable; manual-only ⇒ not installable + instructions available;
  execution-backend gate for installable/updatable.
- Dependency closure: bundled host pull, declared deps, cycle safety, stable order.
- Docker/local infra: dual tool defaults to optional+docker_managed, opt-in moves
  it to required while keeping the docker view; Project Creator resolve parity
  guard (every legacy-resolved id is classified somewhere).
- Readiness: null until snapshot applied, working states counted, replacement on
  re-apply.

### 4. Known limitations

1. `recommended` profile group is deliberately empty until the catalog gains
   recommendation metadata (honest absence, not guessing).
2. `Provenance::PackageManager` exists in the model but detection cannot yet prove
   a winget/apt origin during scans — such installs stay `external`.
3. Engine-job progress is task-granular inside `CanonicalPlan` (no separate
   percentage field); UI computes percent from task counts.
4. `extract_zip_safe` remains used only by tests; installer zip unpacking uses its
   own script but is now pre-gated by the shared validator (behavioral parity kept
   intentionally; full migration of the unpack scripts is a later cleanup).
5. Profile resolution does not consult live machine state by design; readiness
   requires applying a scan snapshot explicitly.
6. Pre-existing dead-code/formatting warnings outside this stage's scope were left
   untouched (see Stage 0 log).

### 5. Verification results (exact)

| Command (workdir `src-tauri`) | Result |
|---|---|
| `cargo check --message-format=short` | **pass** — 0 errors (fixed the 20 pre-existing mid-refactor errors first) |
| `cargo test --quiet` | **pass** — 617 passed, 0 failed, 1 ignored (live-network Qt test, ignored by design); up from 603 baseline after adding 14 new tests |
| `cargo clippy --all-targets --message-format=short` | **pass, 0 errors** — remaining warnings are pre-existing dead-code of deliberate compatibility shims (`build_plan`, `remove_from_user_path`, `extract_zip_safe`, legacy tc commands) + doc-comment whitespace; none introduced by this stage except intentional future-API allows marked `#[allow(dead_code)]` with reasons |
| `rustfmt` (12 touched files only) | **applied + clean `--check`** on those files; repo-wide fmt violations outside scope intentionally left (Stage 0 note stands) |

Not run: frontend checks (no UI changes in this stage by design); `npm run check`
unaffected since TS mirrors are updated in the next stage together with the new UI.

### 6. Handoff notes for next stage

- Wire `tcx_profile_resolve`, expose `declared_capabilities` through
  `tcx_get_tool_definitions` (already serialized automatically), surface
  `active_jobs` union of scan+engine jobs in `tcx_get_environment_snapshot`
  responses.
- Migrate `InstallSession.running` consumers off the boolean onto
  `InstallSessionStatus`.
- Frontend TS mirrors should be generated against contract §8 (kind-tagged).

---

## 1. What was inspected (verified by reading sources)

- **Toolchain backend** (`src-tauri/src/modules/toolchain/`): `mod.rs`, `models.rs`,
  `defs.rs`, `tools.json` (48 tools), `commands.rs`, all of `core/` (version,
  discovery, requirements, check, planner, installer, console, path_service,
  metadata, health, qt_installer, disk), all of `platforms/` (mod, windows, linux,
  macos, unix_rc).
- **Command registration & startup**: `src-tauri/src/lib.rs` (11 tc_* commands,
  ToolchainState managed, PATH sync on boot); `src-tauri/src/core/settings.rs`
  (settings.json persistence).
- **Project Creator backend**: `commands.rs` (events `project_creator:step_event`,
  `project_creator:execution_done`), `models.rs` (WizardContext incl.
  `local_infra_tools`), engine usage of local_infra (readme.rs, mod.rs),
  knowledge/wizard_tree.json as the data bridge consumed by toolchain requirements.
- **Frontend**: `src/lib/modules/toolchain/types.ts` + `api.ts`;
  `src/routes/environment/+page.svelte` (standalone read-only page);
  `src/routes/create/+page.svelte` phase-5 environment/install integration;
  `src/lib/core/navigation.ts`; `src/lib/components/ui/AppShell.svelte`; full UI kit
  inventory in `src/lib/components/ui/`.
- **Persistence**: state.json via MetadataStore; sessionStorage create session key.
- **Tests**: inline `#[cfg(test)]` modules across toolchain core/platforms and
  project_creator; no frontend test tooling exists.
- Prior art: `docs/stackpilot-frontend-contract.md` / `-progress.md` conventions reused.

Findings are recorded in `docs/toolchain-architecture.md` §1–§6, including 30 numbered
issues (safety violations, security weaknesses, bugs, stubs, risks).

## 2. What was created (changed/new files)

| File | Change |
|---|---|
| `docs/toolchain-contract.md` | **new** — target product contract: two modes (Build Environment / Manage Everything), nine independent dimensions, 13-state conceptual status model, target UX structure, non-negotiable safety rules, target backend/event/persistence shapes, compatibility constraints |
| `docs/toolchain-architecture.md` | **new** — verified as-is inventory (files/commands/events/models/persistence), Project Creator integration map, platform limitations, migration matrix (current → target with strategy/compatibility/verification), known bugs/security issues/stubs/risks (#1–#30), tests inventory + gaps |
| `docs/toolchain-progress.md` | **new** — this log |

This session touched nothing else. Note: the working tree contains **pre-existing
uncommitted modifications** (e.g. `src-tauri/src/modules/devlauncher/*`,
`workspace/*`, `project_creator/*`, `src/routes/create/+page.svelte`, relocated
helper scripts) that were already present before this session started and were left
untouched; they are not part of this change set.

## 3. Commands run and results

| Command (workdir) | Result |
|---|---|
| `npm run check` (repo root) | **pass** — svelte-check: 0 errors, 17 warnings. All warnings pre-existing a11y/CSS items in `create/+page.svelte`, `devlauncher/*` pages (files not touched by this session) |
| `cargo fmt --check` (src-tauri) | **fail (pre-existing)** — formatting diffs only in out-of-scope files: `modules/devlauncher/{analyzer,commands,launch_engine}.rs`, `modules/project_creator/engine/content.rs`, `modules/workspace/process_manager.rs`. No toolchain file flagged. Left unformatted intentionally (out-of-scope diff avoidance) |
| `cargo test` (src-tauri) | **pass** — 430 passed, 0 failed, 1 ignored (`qt_installer::tests::live_repo_resolution_and_parsing`, live-network test ignored by design). Full toolchain suite green incl. Windows end-to-end install echo tests |
| `cargo clippy --all-targets` (src-tauri) | **pass with warnings** — 97 warnings (78 duplicates), 0 errors; e.g. `too_many_arguments` on `console::piped_run`. Pre-existing style debt; nothing fixed in this session |

Not run / not available:

- ESLint/Prettier/Vitest/Playwright — **not configured** in this repo (no config files,
  no npm scripts beyond dev/build/check/tauri). Frontend has zero tests today.
- `npm run build` / `tauri build` — skipped as heavyweight and not required for a
  docs-only change; typecheck already covers compile-level validation of the frontend.
- Manual install smoke tests (real winget/UAC runs) — deliberately not executed from a
  documentation session.

## 4. Unresolved risks carried forward (summary; details in architecture doc §5)

1. Scan-path machine mutation (`ensure_maui_workload` inside environment check) —
   violates contract safety rule #1; must be first implementation fix.
2. Secrets exposed via `tc_get_metadata` / `tc_get_install_status` payloads.
3. No download integrity validation; archive extraction lacks traversal protection.
4. Weak DB-password generator (timestamp entropy).
5. Partial-scan reports indistinguishable from complete ones after 90 s deadline.
6. In-memory-only job session: no history/recovery across restarts.
7. Frontend/backend type drift in TS mirrors (`manual_install`, source fields missing);
   `/toolchain` nav match without an existing route file.
8. ~3 700-line monolithic `create/+page.svelte` hosting wizard+env UX — refactor risk.
9. Linux/macOS installs stubbed (`Skipped`); PkgManager builder hardcodes winget.
10. cargo fmt violations exist outside toolchain scope (unfixed here by design).

## 5. Next stage entry criteria (not started)

Per `docs/toolchain-contract.md` §6–§7: additive `tcx_*` scan/snapshot/job APIs behind
the safety rules, secret isolation in metadata reads, integrity + traversal guards in
the installer layer, then the control-center UI on canonical `/toolchain` with the
legacy `/environment` alias preserved. Old commands/events stay until consumers migrate.

---

# Stage 1 — read-only discovery & diagnostic engine (2026-08-21)

Scope of this session: implement the new **read-only scan/diagnostic engine**
(`domain/`) in the existing Rust Toolchain backend per contract §5–§6, plus the
minimal compile fixes required by the unfinished working tree. No installation
behavior changed beyond what the new job model required to compile.

## 1.1 Pre-existing breakage fixed first (working tree did not compile)

The inherited uncommitted refactor left 16 compile errors; fixed minimally:

- `mod.rs`: dangling `pub mod domain;` → the module now exists (the new engine).
- `commands.rs`: `secrets_arc.set_secret(...)` called without locking the mutex.
- `core/archive.rs`: `extract_zip_safe`/`extract_tar_safe` missing `session_id`
  argument for `run_tool_script` (event identity restored end-to-end).
- `core/installer.rs`: `try_install_source` call-site argument order; four
  `&Arc<RedactingSink>` → `&Arc<dyn EventSink>` coercions; `configure_php_ini`
  test calls updated to current signature.
- `core/planner.rs`: `_ => &[]` slice-type mismatches; test initializer missing
  `complete`/`scan_timed_out`.
- `core/discovery.rs` test: borrow-after-move in glob assertion.
- Pre-existing failing tests repaired: `official_msi_runs_via_msiexec` (asserts the
  old predictable MSI-log name; now asserts unique `tc-*-msi-*.log` naming from the
  temp-squatting fix), `merge_is_case_sensitive` (missing non-Windows cfg gate).

## 1.2 New engine: `src-tauri/src/modules/toolchain/domain/`

| File | Responsibility |
|---|---|
| `models.rs` | Dimension model + presentation states (`ToolState`, 13 variants per contract §3), `DetectedInstall` (multiple installations), `DetectionOutcome` (**scan-failed ≠ missing**), `VersionAssessment`, `Provenance`, `PlatformApplicability`, health V2 (`NoChecksDefined` is explicit), PATH finding/report types, `ScanJobSnapshot`, `ScanProgressEvent` (identity-required constructor), `ScoreSummary`, `EnvironmentSnapshot`, `ToolScanResult` |
| `probe.rs` | The only process launcher of the layer: catalog-approved commands only, per-probe timeout (10 s), stdout/stderr captured with hard 8 KiB/stream cap (deadlock-safe parallel readers), sanitization strips values of secret-looking env vars (`SECRET/PASSWORD/TOKEN/KEY/...`), stdin disabled, `kill_on_drop`; robust version extraction from noisy output |
| `detect.rs` | Per-tool live detection: PATH probes → known paths (**all** glob matches → multiple installations) → registry footprints; PATH visibility per install; silent-binary-in-PATH → PathBroken evidence; inconclusive probes (timeout/launch error) with zero evidence → `ScanFailed`, never `Missing`; catalog-declared health checks with process-failure vs assertion-failure distinction; applicability classification (installable / manual-only / docker-default / built-in / unsupported-on-platform); state composition incl. manual normalization parity with check.rs |
| `path_report.rs` | Read-only PATH analysis: exact duplicates, case/slash-normalization duplicates, stale entries, `%VAR%` requiring expansion, unverifiable entries; per-tool findings (binary-not-on-path, expected entry missing). **No mutation whatsoever** |
| `score.rs` | Documented formula: denominator = required+applicable tools only; healthy=100, degraded(update-available)=50, missing/broken/unhealthy=0; unchecked, scan-failed/pending, optional/docker/manual/built-in and not-applicable are **excluded** (never penalize); half-up rounding |
| `cache.rs` | Honest snapshot cache: last valid snapshot returned immediately from memory→disk (`scan-snapshot.json`, atomic tmp+rename); `stale` flag + `age_seconds` recomputed on every read against a freshness threshold (default 5 min); broken file = "no cache" |
| `engine.rs` | Scan job lifecycle: job_id + scan_id, phases Queued→Environment→Tools→Finalizing→Done, total/completed/current tool, cancel flag, terminal states Running/Completed/Partial/Cancelled/Failed/**Interrupted**; bounded concurrency (Semaphore, default 4); deterministic result ordering = catalog order regardless of completion order; overall soft deadline (90 s) → honest Partial with remaining tools `ScanPending`; typed events only with operation identity; journal `scan-journal.json` with startup recovery Running→Interrupted; reconnect semantics (second start returns the running job) |

Safety properties (enforced by construction, guarded by tests): scans never install,
update, delete, touch PATH, touch project files, run package-manager mutations or
workload installs, write secrets, or change preferences. `ensure_maui_workload`
remains only inside explicit approved install jobs (installer.rs).

## 1.3 Backend commands

New (registered in `lib.rs`):

| Command | Purpose |
|---|---|
| `tcx_get_environment_snapshot` | Last valid snapshot immediately; `stale`/`age_seconds`/`from_cache` flags; None before first scan |
| `tcx_start_scan` | Start read-only scan; if one is running → reconnect (`AlreadyRunning`) instead of a second scan |
| `tcx_get_scan_job(job_id)` | Job snapshot by id (running, terminal, or recovered Interrupted) |
| `tcx_get_latest_scan_job` | Current/latest job snapshot |
| `tcx_cancel_scan(job_id)` | Cooperative cancel: no new tools start, running probes finish, terminal Cancelled/Partial |
| `tcx_get_tool_details(tool_id)` | Live single-tool state: detection + installs + health + PATH findings |
| `tcx_run_health_checks(tool_ids)` | Health-with-live-state for selected tools; ids validated against catalog; bounded concurrency (4); deterministic order |

Compatibility adapters: all eleven legacy commands (`ping_toolchain`,
`tc_get_tool_definitions`, `tc_get_environment_info`, `tc_check_environment`,
`tc_build_install_plan`, `tc_run_install`, `tc_get_install_status`,
`tc_abort_install`, `tc_take_new_secrets`, `tc_get_metadata`, `tc_get_health_report`)
remain registered and byte-compatible for Project Creator and `/environment` until
their migration (contract §7).

## 1.4 Events

New (typed, always carrying operation identity):

| Event | Payload |
|---|---|
| `toolchainx:scan_progress` | `{job_id, scan_id, completed_count, total_count, tool_id, display_name, icon, tool_state, timestamp, error}` — constructor refuses empty identity |
| `toolchainx:scan_done` | `{job_id, scan_id, terminal, completed, total}` |

Legacy events (`toolchain:check_progress`, `toolchain:task_event`,
`toolchain:install_done`) continue unchanged.

## 1.5 Frontend types

`src/lib/modules/toolchain/types.ts`: mirrors for the full tcx surface
(`ToolState`, `DetectedInstall`, `HealthOutcomeV2`, `PathReport`,
`ScanJobSnapshot`, `EnvironmentSnapshot`, `ScoreSummary`, events) plus helpers
(`toolStateIsOk`, `toolStateName`). `api.ts`: wrappers for all seven tcx commands
and both event listeners.

## 1.6 Tests added (all passing)

- **No mutation during scan**: engine writes nothing but its own journal/snapshot;
  state.json/secrets.bin never appear (engine test).
- **Timeouts**: probe reports `timed_out` quickly and kills the process.
- **Malformed version output**: garbage probe output still detects the tool as
  installed with `Unparseable` assessment (never crashes, never "missing").
- **Duplicate paths**: exact and case/normalization duplicates detected.
- **Missing PATH entries**: binary-found-but-not-on-PATH and expected-entry-missing.
- **No-health-check tools**: `NoChecksDefined` → `InstalledHealthUnknown`, excluded
  from score denominator (never unhealthy).
- **Unsupported platform**: foreign-OS sources → `UnsupportedOnPlatform`/
  `UnsupportedPlatform`, excluded from score even when missing.
- **Stale cache**: old snapshot still returned, flagged `stale` with age; fresh
  snapshot not flagged; disk reload works; broken file tolerated.
- **Cancellation**: pending tools stay `ScanPending`, terminal Cancelled/Partial,
  repeated cancel is a no-op.
- **Stale event isolation**: every event carries its job/scan identity; two runs'
  identities differ; `ScanProgressEvent::try_new` rejects empty identity.
- **Bounded concurrency**: `bounded_map` gauge stays ≤ limit with deterministic
  output order; engine uses Semaphore(4).
- **Sanitized output**: secret env-var values replaced with `***`; output capped.
- Plus: deterministic catalog-order snapshots, reconnect-to-running, journal
  recovery Interrupted, score table (degraded=half, rounding half-up, bundled-missing
  not a failure), glob-all multi-install detection, legacy status mapping.

## 1.7 Commands run and results

| Command (workdir) | Result |
|---|---|
| `cargo fmt` then `cargo fmt --check` (src-tauri) | **pass** — tree formatted, no diffs. Per stage instructions formatting was applied repo-wide; this also normalized the five out-of-scope files Stage 0 had deliberately left unformatted (`devlauncher/{analyzer,commands,launch_engine}.rs`, `project_creator/engine/content.rs`, `workspace/process_manager.rs`) — formatting only, no semantic changes |
| `cargo test` (src-tauri) | **pass** — 528 passed, 0 failed, 1 ignored (live-network Qt test, ignored by design) |
| `cargo check --tests` (src-tauri) | **pass** — 0 errors (was 16 errors at session start) |
| `npm run check` (repo root) | **pass** — svelte-check: 0 errors, 17 warnings (all pre-existing a11y/CSS items in untouched files) |

Note: an untracked `src/modules/toolchain/engine/` directory (`events.rs`,
`plan.rs`, `request.rs`) appeared in the working tree during this session from
parallel work on the unified job registry. It is not declared in `mod.rs`, does
not compile into the crate, and was left untouched — it is not part of this
change set.

## 1.8 Remaining installer limitations (unchanged by design this session)

1. **Windows-first execution**: `run_task` marks non-Windows tasks `Skipped`;
   Linux/macOS sources exist in the catalog but no executor (sudo/apt/brew wrappers)
   exists yet.
2. **PkgManager builder hardcodes `winget`** as program name — latent bug when
   Linux/macOS execution is enabled (apt/dnf/brew sources would invoke winget).
3. **Qt pipeline** remains Windows/msvc2022_64-only, pinned to the Qt 6.8 branch by
   prefix check `"68"`, with hand-rolled Updates.xml parsing and HTML listing scrape.
4. **Update flows are display-only**: `UpdateAvailable` renders a label; there is no
   dedicated update command (re-install relies on winget upgrade semantics).
5. **No uninstall / fix-PATH action / per-tool recheck command yet** — fix-PATH
   mutations are designed (path_service has add/remove) but not exposed as audited
   jobs; PATH diagnostics are read-only in this stage.
6. **Elevation (`run_elevated`)** is Windows-UAC only; argument quoting for exotic
   args remains fragile (architecture doc issue #9).
7. **Sources without sha256** download as explicitly *unverified* (honest warning);
   integrity enforcement for those is still open (issue #3/#6).
8. **Single install-session slot** preserved for compatibility: second concurrent
   `tc_run_install` is rejected; the unified multi-job registry is a later stage.
9. **Tool-specific post-install hooks** (MAUI workload, php.ini rewrite, composer
   shim) remain imperative special cases inside installer.rs pending the declarative
   hook mechanism (issue #25).
10. **PATH race window**: whole-user-PATH replace on write can race external edits
    between read and write (issue #28); unchanged.

## 1.9 Next stage entry criteria

Per contract §4/§6: control-center UI on canonical `/toolchain` consuming
`tcx_*` + `toolchainx:*` (cached-snapshot-first rendering, per-tool progress rows,
cancel, stale/cancelled states), then plan fingerprinting/re-validation, unified job
registry, and legacy `tc_*` removal after consumer migration.

---

## Stage 3 — canonical installation & environment-management pipeline (2026-08-21)

Scope: backend-authoritative job pipeline (`engine/`) implementing
install / update / repair-PATH / health-check operations with restricted
requests, canonical plans, persistence-before-execution, typed phases,
integrity enforcement, audited PATH changes, cancellation, restart recovery,
typed events, and Project Creator compatibility through an adapter.
The parallel read-only scan stage (`domain/`, `tcx_*scan*` commands) was
already present in the working tree and was left intact.

### What was created (new files)

| File | Content |
|---|---|
| `src-tauri/src/modules/toolchain/engine/mod.rs` | Module wiring, public API re-exports, end-to-end pipeline test |
| `engine/request.rs` | Restricted request model (`EngineRequest`, `ToolRequest`, `OperationKind`, `ExecutionChoice`, `VersionChannel`). `deny_unknown_fields` makes URLs/paths/args/versions/sizes/task-states/dependency lists physically unrepresentable. `force_reinstall` is the explicit reinstall confirmation |
| `engine/plan.rs` | Canonical plan types: `CanonicalPlan` (fingerprint, disk/capability facts, warnings), `PlanTask` (backend-resolved action/source/deps/path entries), typed `Phase` (12 variants incl. validating/preparing/configuring/checking_health/completed/failed/cancelled/interrupted), `JobStatus`, `EngineTaskStatus`, `NoopReason`, `SelectedSource` (catalog integrity metadata), `PathChangeRecord` |
| `engine/planner.rs` | Canonical planner: definition resolution, platform gate, conflict table, fresh detection (injectable `Detector`; `DiscoveryDetector` over discovery), dependency closure (bundled-with parents + winget bootstrap for PkgManager sources), source selection (explicit user source must be catalog-allowed), admin validation (elevation capability + explicit confirmation), integrity validation (digest-less sources require confirmation), disk requirements, idempotency decisions (`NoOp(AlreadyInstalled)` / `NoOp(UpdateUnavailable)` / docker-managed default), topological ordering (winget first), plan/task ids + input fingerprint |
| `engine/jobs.rs` | Job registry: one active mutating job at a time, record persisted BEFORE execution (`jobs/<job_id>.json`), bounded ring (20), startup recovery Running→Interrupted (unfinished tasks marked Interrupted), watch-based status, pluggable event sinks with per-job monotonic seq counters, audited path changes, error list. No secret field exists in job state |
| `engine/exec.rs` | Executor: per-task validating→preparing phases, no-op short-circuit, execution-time source revalidation against catalog, injectable `TaskRunner` (`RealRunner` reuses installer machinery via single-task adapter; `BridgeSink` translates legacy installer events into typed engine events), RepairPath runner (idempotent user-PATH add + before/after diff + process sync), HealthCheck runner (read-only), cancellation (kills current process, remaining tasks Cancelled), terminal status computation (Succeeded/Partial/Failed/Cancelled), temp cleanup on all terminal states, secrets returned out-of-band |
| `engine/events.rs` | `JobEvent {job_id, task_id, tool_id, seq, timestamp, payload}` — constructor rejects identity-less events; `belongs_to` filter for stale-event dropping; `MemorySink` test buffer |

### What was changed

| File | Change |
|---|---|
| `toolchain/mod.rs` | `pub mod engine;` + `job_engine: Arc<JobEngine>` in `ToolchainState` (loads + recovers on startup) + accessor |
| `toolchain/models.rs` | `PlatformCapabilities` derives `PartialEq/Eq`; `ToolchainMetadata(.View).adopted` map (additive, serde-default) for explicit adopt/track marks |
| `core/metadata.rs` | `record_adoption` / `is_adopted` |
| `commands.rs` | New commands: `tcx_build_plan`, `tcx_start_job`, `tcx_get_job`, `tcx_list_jobs`, `tcx_cancel_job`, `tcx_retry_job`, `tcx_adopt_tool`; global `TauriJobEventSink` (`toolchainx:job_event`) registered once in setup; `tc_run_install` rewritten as ADAPTER over the engine (legacy plan → restricted request with `force_reinstall=true` reproducing wizard semantics; backend rebuilds canonical plan; execution via engine; legacy `toolchain:task_event`/`toolchain:install_done` bridged per-run; legacy session slot + jobs.json maintained); `tc_abort_install` also cancels the active engine job; dead `AppEventSink` removed; PC-compat mapping tests added |
| `lib.rs` | Registered 7 new commands + `register_tcx_event_sink` in setup |

### Guarantees implemented (pipeline prompt mapping)

1. Request model — enforced by serde `deny_unknown_fields` (tests prove URL/version/state/dep injection fails deserialization).
2. Canonical plan — planner builds everything server-side; client sends selections/intents only; plan persisted before execution; fingerprint recorded; sources revalidated at execution time.
3. Idempotency — installed-at-acceptable-version → truthful NoOp unless `force_reinstall`; update-unavailable → truthful NoOp; adopt/track is a separate explicit command that never claims StackPilot provenance.
4. Phases — typed 12-value `Phase` enum emitted through events.
5. Integrity/safety — reuses Prompt-2 guarantees (unique temp files, traversal-safe archives, sha256 gate in download, https-only URLs, redacting sink); adds digest-less-source confirmation requirement and execution-time source revalidation.
6. PATH — planned explicitly per task (`path_entries`), applied only inside approved jobs, idempotent/normalized via `path_service`, diffed before/after, reported in `path_changes` + `PathUpdated` events; scans never touch PATH.
7. Updates — `UpdateAvailable` (catalog min/recommended policy) → `Update{current,target}` tasks with size/admin/source/warnings; health-check/diagnose operations are strictly read-only.
8. Persistence/recovery — full job records survive restarts; Running→Interrupted on load; `tcx_retry_job` re-plans from the stored request facts; secrets excluded by construction.
9. Events — every event carries job/task/tool ids, per-job seq, timestamp, typed payload; consumers filter with `belongs_to`.
10. Project Creator — legacy commands/events/session-slot/journal preserved; adapter covered by mapping tests; docker/local infra semantics untouched (dual tools stay docker-managed unless Host chosen explicitly).
11. Tests — 56 engine tests + 4 compat tests covering: invalid tool ids, invalid source ids, dependency closure (+dedupe), conflicts (table), idempotent install, force reinstall, update plans, update-noop, update-of-missing, read-only health plan, disk rejection, admin confirmation, unverified-source confirmation, manual-only exclusion, broken-install reinstall warning, repair-path plan shape, ids/fingerprint assignment, empty selection, request-model rejections, phase/status serde, persistence-before-execution, single-active-job, restart recovery (interrupted), terminal survival, event identity/monotonic seq, stale-event filtering, path audit, secret-free job state, bounded history, cancel flag, no-op bypass, failure/partial/cancel terminal math, foreign-source revalidation, legacy state mappings, PC payload roundtrip.

### Commands run and results

| Command (workdir) | Result |
|---|---|
| `cargo test --lib` (src-tauri) | **pass** — 588 passed, 0 failed, 1 ignored (live-network Qt test, by design) |
| `cargo clippy --lib` (src-tauri) | **pass** — no warnings in `engine/`; only two pre-existing dead-code notes in `commands.rs` (`tc_get_platform_capabilities`, `tc_create_install_plan` — registered earlier, not yet consumed by any route) |
| `cargo fmt --check` (src-tauri) | **pass** (touched files rustfmt'd; tree fully clean) |
| `npm run check` (repo root) | **pass** — svelte-check: 0 errors, 17 warnings (same pre-existing a11y/CSS items as baseline) |

### Remaining unsupported platform operations (exact list)

Windows-first execution is preserved; the following remain unimplemented:

1. **Linux/macOS automatic install execution** — `Install`/`Update` operations are rejected at planning (`PlatformUnsupported`); catalog sources exist for apt/dnf/pacman/brew but no executor exists (legacy `run_task` skip retained for direct callers).
2. **Elevation off-Windows** — UAC-only; admin-requiring plans are rejected with `ElevationUnsupported` when `elevation_supported=false`.
3. **Qt online pipeline** — Windows/msvc2022_64 only; other platforms get no Qt task.
4. **PkgManager bootstrap** — winget auto-dependency is Windows-only; no brew/apt equivalent bootstrap logic yet.
5. **Downloads/probes via PowerShell 5.1 / curl.exe** — Windows-oriented transport; Linux/macOS would need native HTTP/tar paths.
6. **RepairPath on Linux/macOS** — rc-file block writer exists (`unix_rc.rs`) but the repair-path job has not been exercised/tested there.
7. **True mid-task resume** — interrupted jobs recover to a deterministic Interrupted state and can be retried (fresh plan), but a partially-downloaded task does not resume in place.
8. **Uninstall operation** — not part of the operation set yet (PATH removal primitive `remove_from_user_path` exists and is reversible-by-design).
9. ~~Frontend TS bindings for the new tcx job API~~ — **done in Stage 4** (see below); Control Center UI consumption is the next stage.

---

# Stage 4 — frontend foundation of the Control Center (2026-08-21)

Scope: typed frontend foundation for the Toolchain Control Center per contract
§6–§8 — canonical TS models, typed API layer, identity-aware event layer,
Svelte 5 runes controller, formatting/filter helpers, Project Creator
compatibility layer, first frontend test suite. No UI pages built yet
(deliberately out of scope).

## 4.1 What was created / changed

| File | Content |
|---|---|
| `src/lib/modules/toolchain/types.ts` | Rewritten. Section A: legacy `tc_*` mirrors kept byte-compatible (`ToolStatus`, `InstallPlan`, `ToolchainEvent`, helpers `statusIsOk/statusLabel/statusKind/taskStateKind/taskStateLabel`), drift fixed: `InstallSource` gains `file_name/install_dir/needs_admin/execution/sha256`, `ToolDefinition` gains `manual_install` + flattened extended metadata (aliases/dependencies/conflicts/docs_url/source_url/platform_availability/declared_capabilities/docker), `EnvironmentInfo.capabilities`, `EnvironmentCheck.complete/scan_timed_out`, `InstallPlan.session_id`, `InstallSession.status` (**secrets field removed** — backend serializes them with `#[serde(skip)]`), `ToolchainEvent.session_id`, `CheckProgressEvent.scan_id`, `ToolHealth.state`, `ToolchainMetadata` = sanitized View (+`adopted`). Section B: canonical domain per contract §8 as **explicit kind-tagged discriminated unions** — `ToolState` (13 variants + `TOOL_STATE_KINDS` + safe parser `parseToolState` returning null on unknown kind), `EvidenceKind`, `DetectedInstall`, `DetectionOutcome`, `VersionAssessment` (+`policy_violation`), `Provenance` (7 kinds), `PlatformApplicability`, `HealthState` (9 kinds) + `healthIsVerdict`, `ToolPlatformCapabilities` (8 flags), PATH report types, `ScanJobSnapshot`/`ScanStartOutcome` (+extractor)/events, `ScoreSummary`, `StatusCounts`, `DiskSpaceInfo`, `AdminCapability`, `SnapshotIssue`, full `EnvironmentSnapshot`. Section C: engine job family mirroring the REAL serde encoding (externally tagged snake_case): `EngineRequest`/`ToolRequest`/`OperationKind`, `Phase`(12), `JobStatus`, `TaskAction`/`NoopReason`, `PlanTask`/`SelectedSource`/`PlanWarning`/`PathChangeRecord`/`CanonicalPlan`/`PersistedJob`, `JobEvent`/`JobEventPayload` + `jobEventBelongsTo`. Section D: `EnvironmentProfile` group (Build Environment). Section E: `CatalogFilters`/`PageRequest`/`PageMeta`/`Paged<T>`. |
| `src/lib/modules/toolchain/api.ts` | Typed wrappers for every registered `tcx_*` command: snapshot, start/cancel/get/latest scan, tool details, health checks, `tcx_build_plan`, `tcx_start_job`, `tcx_get_job`, `tcx_list_jobs`, `tcx_cancel_job`, `tcx_retry_job`, `tcx_adopt_tool`, convenience `repairPath()` and `mutationRequest()`. All legacy `tc_*` functions and thin legacy listeners preserved verbatim. |
| `src/lib/modules/toolchain/events.ts` | **New** — event layer: `EventChannel` with ref-counted subscriptions (ONE physical Tauri listener per event name regardless of subscriber count; auto-detach at zero subscribers; idempotent unsubscribe; handler exceptions isolated); module-singleton channels for all six events (survive route navigation → no duplicate listeners after reconnect); `TerminalGuard` (terminal exactly once per id); scoped `trackScan(jobId)` / `trackJob(jobId)` with identity filters (stale foreign-job events dropped before handlers run). Injectable `AttachFn` keeps it unit-testable without Tauri. |
| `src/lib/modules/toolchain/stateLogic.ts` | **New** — pure state-transition rules (no Svelte/Tauri deps): incremental `applyScanProgress` (string state name → validated canonical variant; unknown names ignored; payload preserved when variant repeats), `mergeLiveSnapshot` (cache renders as-is; scan never erases prior tools; summary recomputed only under full live coverage), `snapshotFreshness`, `gateMutation` (conflicting mutation blocked while one is active; read-only ops always allowed; `SCAN_DURING_MUTATION_ALLOWED=true` — scan engine is strictly read-only), `applyJobEvent` (identity + monotonic seq guard; status never moves backwards from terminal), `isJobSucceeded` (success only from backend terminal facts), `upsertJobHistory` (bounded ring). |
| `src/lib/modules/toolchain/state.svelte.ts` | **New** — Svelte 5 runes controller (module singleton class with `$state`/`$derived`): cached snapshot + live overlay + freshness, loading/error (failed calls keep previous valid data), current scan job + cancel flow, current mutation job + bounded session history, catalog/profile loading, mode/selected-tool/drawer/log-panel/filters UI state, toast integration via `$lib/core/toasts`, persisted lightweight prefs (`stackpilot:toolchain:prefs:v1` via `core/storage`; mode/log panel/search/categories/states only). `ensureInitialized()` is idempotent: attaches listeners once, restores running scan/job after navigation (reconnect via `getLatestScanJob`/`listJobs`). Stale payloads never touch current state (identity+seq filtered in `stateLogic`). Secrets have no representation anywhere in state types. |
| `src/lib/modules/toolchain/format.ts` | **New** — pure formatters: `formatSizeMb`/`formatBytes`, `formatRelativeTime`/`formatAgeSeconds`, info tables with Badge-tone mapping for `ToolState`/version assessment/health/provenance/job status/phase, capability labels, platform names, install-source descriptions (incl. unverified-source marker), `sanitizeErrorMessage` (secret-like tokens masked, paths shortened, length-capped). Unknown kinds render "Неизвестное состояние", never throw. |
| `src/lib/modules/toolchain/filters.ts` | **New** — pure predicates over snapshot tools: search/category/state-kind/provenance/capability/execution-mode, `applyCatalogFilters` (catalog order preserved), `availableCategories`, definition-level search incl. aliases. |
| `src/lib/modules/toolchain/compat.ts` | **New** — explicit Project Creator compatibility surface: re-exports the exact legacy functions/listeners/types/helpers the wizard uses; documented as the single allowed import point for the create page. |
| `src/routes/create/+page.svelte` | Imports switched from `toolchain/api`+`types` to `toolchain/compat` (2 import blocks, no logic changes). One dead branch removed that read `session.secrets` — the backend stopped serializing session secrets (`#[serde(skip)]`) long ago, so the branch could never execute; secrets still arrive exclusively via one-shot `getNewSecrets()`. User-facing behavior unchanged. |
| `package.json`, `vitest.config.ts` | First frontend test tooling: `vitest` devDependency, `test`/`test:watch` scripts, isolated config (node env, pure-TS tests only, no SvelteKit plugin needed). |
| `src/lib/modules/toolchain/*.test.ts` | **63 unit tests** covering: discriminated-union discrimination by `kind` incl. all 13 ToolState kinds + unknown/garbage rejection (`parseToolState` returns null for legacy encodings/bare strings), legacy `statusKind`/`taskStateKind` tables, event-channel deduplication (N subscribers → 1 physical listener, detach-on-zero, idempotent cleanup, filter-before-handler, handler isolation), stale-event dropping (foreign `job_id`), terminal-exactly-once (duplicate backend terminals, `Running` not terminal), incremental scan progress (unknown state ignored, prior tools untouched, `scan_failed` ≠ missing), cache/live merge rules, freshness, mutation gating (active mutation blocks mutations; health_check passes; terminal releases), seq/duplicate/out-of-order job-event guards, success-only-from-backend, history dedupe/limit, filter predicates, all formatters incl. sanitization masking. |

## 4.2 Backend contract mismatches (recorded explicitly)

1. **`tcx_profile_resolve` is NOT registered** — `domain/profile.rs` ships the full
   `EnvironmentProfile` model, but no command exposes it yet (matches the Stage 1
   handoff note). Frontend `resolveEnvironmentProfile()` invokes the planned command
   name and will surface a backend error until the command layer wires it.
2. **No dedicated tcx catalog command** — `getCatalog()` uses legacy
   `tc_get_tool_definitions`, which already carries the flattened extended metadata,
   so this is lossless today; a `tcx_*` replacement remains desirable.
3. **Engine enums deviate from §8.1's blanket rule** — contract §8.1 says every new
   data-carrying enum uses internal `{"kind": …}` tagging, but lists only the seven
   domain enums. The actual `engine/*` serialization is EXTERNALLY tagged snake_case
   (`{"already_installed": {…}}` / `"docker_managed"`), verified against
   `engine/{plan,request,events,jobs}.rs`. TS mirrors the implemented encoding;
   either the doc blanket rule or the engine encoding should be reconciled later.
4. **`ScanPhase`/`ScanTerminal` serialize as PascalCase strings** (`"Queued"`,
   `"Partial"`, … — no `rename_all` in `domain/models.rs`). TS mirrors this; noted
   because §8's prose ("snake_case names") could suggest otherwise.
5. **Dead command surface** — `tc_get_platform_capabilities` and
   `tc_create_install_plan` exist in `commands.rs` but are NOT registered in `lib.rs`;
   not exposed by the frontend.
6. **Secrets hygiene already fixed backend-side** — `tc_get_metadata` returns the
   sanitized View and `InstallSession.secrets` is `#[serde(skip)]`; the TS types now
   enforce this (no `secrets` field), and the create page's dead `session.secrets`
   branch was removed.

## 4.3 Commands run and results

| Command (workdir) | Result |
|---|---|
| `npm test` (root) | **pass** — vitest: 5 files, 63 passed, 0 failed |
| `npm run check` (root) | **pass** — svelte-check: 0 errors, 17 warnings (all pre-existing a11y/CSS items in untouched `create/+page.svelte`/`devlauncher/*` sections; baseline unchanged) |
| ESLint/Prettier | not configured in this repo (unchanged finding from Stage 0) |
| `cargo check/test/clippy` (src-tauri) | **not run — no Rust/shared-contract file changed**; this stage aligned the frontend to the already-implemented backend serialization |

## 4.4 Handoff notes for the UI stage

- Build `/toolchain` on `toolchain.ensureInitialized()` + `toolchain.liveSnapshot`
  (cached-first rendering is already correct); drawer/log panel/mode state is ready.
- Auto-scan on entry: call `ensureScanRunning()` from the page mount (contract §4.2).
- Plan review screen consumes `buildCanonicalPlan` + `mutationRequest`; respect
  `mutationGate` for disabled-button explanations.
- When the backend registers `tcx_profile_resolve`, only the invoke target needs to
  change (types are final per §8.6).
- Remaining type-drift watch item: none known; TS now matches implemented serde for
  both protocol families (legacy externally-tagged PascalCase, canonical kind-tagged).

---

# Stage 5 — Toolchain Control Center UI (2026-08-21)

Scope: the standalone read-only `/environment` page is replaced by the full
**Toolchain Control Center** on the canonical `/toolchain` route (contract §4),
built entirely on the Stage 4 frontend foundation (controller/events/filters/format)
and the existing design system (`PageContainer/PageHeader/Card/Button/Badge/Tabs/
Modal/Progress/LoadingState/EmptyState/ErrorState/Icon/IconButton/TechIcon`,
`--sp-*` tokens). No static fake data: every fact comes from backend commands.

## 5.1 Backend (small additive wiring)

| File | Change |
|---|---|
| `toolchain/commands.rs` | **new command `tcx_profile_resolve(requirements)`** — closes the Stage 1 handoff gap. Pure `(selection, catalog, OS)` resolution via `domain::profile::build_profile`, then readiness computed **backend-side** by applying the latest cached scan snapshot (`apply_snapshot`) — until the first scan `satisfied_count` stays `None` (honest "no data"). No machine mutation |
| `toolchain/domain/profile.rs` | `ProfileTool.reason` field (serde-default) + `inclusion_reasons()`: every profile entry carries a truthful inclusion reason — «Выбрано в требованиях», «Базовый менеджер пакетов» (winget), «В комплекте с X» (bundled host), «Требуется для X» (declared dependency). UI never guesses reasons. Tests: `inclusion_reasons_distinguish_direct_dependency_and_bundled`, `profile_reasons_are_filled_from_catalog_facts` |
| `lib.rs` | registered `tcx_profile_resolve` |

## 5.2 Frontend foundation extensions

| File | Change |
|---|---|
| `state.svelte.ts` | controller additions used by the UI: `definitions` map + `ensureDefinitions()`/`definitionFor()` (catalog metadata for descriptions/sources/deps, loaded once via legacy `tc_get_tool_definitions` which already carries extended metadata); per-job bounded **log buffer** (`jobLogs`, 300 entries/job) built from typed `JobEvent`s via `jobEventLogText()`; `retryJob(jobId)` (backend re-plans from stored request facts; adopted job becomes current); `ensureDefinitions` wired into `ensureInitialized` |
| `filters.ts` | `CatalogFilters` extended: `health[]` (actual HealthState kind incl. honest `not_checked` for null health), `admin_only` (fact **only** from catalog metadata; no metadata → excluded, never guessed), `update_only`, `manual_only`; predicate takes `CatalogFilterContext {definitions}`; new `sortCatalogTools` (`catalog/name_asc/name_desc/category/status` with a documented severity order); existing tests untouched, 4 new tests |
| `types.ts` | `ProfileTool.reason?`; `CatalogFilters` extension; `CatalogSort` |
| `format.ts` | kind-based helpers for filter rails: `toolStateKindInfo`, `allToolStateKinds`, `provenanceKindInfo`, `allProvenanceKinds` |
| `api.ts` | `mutationRequest` widened to all `OperationKind` (repair_path plans from manage mode/profile use it too) |

## 5.3 New components (`src/lib/modules/toolchain/components/`)

| Component | Responsibility |
|---|---|
| `ScoreRing.svelte` | SVG score donut; tone lime/amber/red/**neutral** by verdict; dashed ring for partial scans; neutral when no data — never fake green |
| `StateBadge.svelte` | `ToolState` → Badge tone/label (+optional version) via `format.toolStateInfo` |
| `EnvironmentHero.svelte` | health hero: score ring, verdict (Данных ещё нет / Требует ремонта / Внимание / Всё готово — stale/partial/missing/unchecked force amber-or-worse), required-tools summary, updates/broken/missing counts, disk headroom, admin capability, last-scan age; actions Scan now / Build environment / Review updates (disabled truthfully when zero updates) |
| `ActivityStrip.svelte` | non-intrusive strip for active scan (phase, n/m, cancel) and active mutation job (status, task progress, running phase, cancel, sanitized first error); terminal scan summary with re-scan |
| `JobCenter.svelte` | operations card: active scan/job rows with typed phases and progress; session history with status badges, recovered flag («после перезапуска»), retry for failed/interrupted/cancelled, expandable logs (raw lines live only in these collapsed areas); empty state |
| `ToolCard.svelte` | marketplace-style card: TechIcon anchor, name/category, catalog description, StateBadge, version (with → target on update), provenance, size, admin/PATH/health flags, truthful primary action derived from capabilities+state (install/update/repath → plan review; installed+health-checkable → direct read-only recheck; manual → instructions/details), details overflow |
| `ToolDetailDrawer.svelte` | side dialog (full-screen on narrow): hero, description/notes, version comparison + assessment badge, detection outcome & per-install evidence/path/PATH-reachability, health checks with durations, PATH findings, dependencies/conflicts/**dependents** (reverse deps computed from catalog), per-OS sources with integrity marker (sha256 present/absent), Docker image/notes, admin/size/installability facts, notes/manual instructions, docs/source links, tool-filtered job log; footer actions mirror card rules; Escape/backdrop close, focus moved into panel |
| `RequirementPicker.svelte` | Build Environment selection fed by `getWizardTree()` (backend data, nothing hardcoded): project-type presets (apply mapped languages/tools via wizard maps), language chips, searchable frameworks grouped by side (incompatible ones disabled with reason in tooltip), infra tools grouped by category (database/cache/messaging/observability/container/testing/BaaS/orchestration/ETL/infra/tooling) with local-install opt-in toggles for dual docker tools, git/vscode/docker switches, reset + picked count |
| `ProfileResult.svelte` | resolved profile groups: Required (icon, name, **reason**, how-it-installs, size, admin, live status from snapshot or honest «нет данных»), Recommended (reserved-empty note — honest absence), Optional docker tools with local opt-in toggle (re-resolves), collapsed Docker-managed / Unsupported groups, Manual group, conflicts + warnings alert list; summary bar with size estimate, readiness badge (`satisfied_count` unknown without scan) and Review-plan action |
| `BuildEnvironmentMode.svelte` | two-column layout (stacks ≤980px): picker left, resolver result right; debounced (400 ms) auto-resolve through `tcx_profile_resolve`; loading/error(+retry)/stale-profile-warning/empty states; review button builds plan request for required tools only |
| `ManageEverythingMode.svelte` | prominent search (focus ring via tokens), sort select, collapsible filter rail (≤980px toggle): states (only kinds present, with counts), health, categories (counts), provenance, platform capabilities, execution mode host/docker, manual-only, update-only, admin-only; results count with inline reset; responsive `auto-fill minmax(19rem,1fr)` card grid keyed by id (no giant table); empty/no-data/error/loading states; «часть инструментов ещё проверяется» note while scan_pending exists |
| `PlanReviewModal.svelte` | approval gate over `tcx_build_plan`: preview tasks (action label, source description, deps order, size, UAC, noop rows muted), totals + enough-space badge, warning list (unverified sources / admin / reinstall-on-broken), explicit checkboxes for unverified-source risk and elevation consent, blocked-reason panel (no space, elevation unsupported, mutation conflict via `mutationGate`, nothing-to-do), Start → `startMutation` (job appears in strip/center) |

## 5.4 Routes & navigation

| File | Change |
|---|---|
| `src/routes/toolchain/+page.svelte` | **new canonical Control Center page**: PageHeader (title/description, OS·arch badge, freshness badge, active-operation indicator chip → opens job center, Scan button with spinner, activity toggle); ActivityStrip; EnvironmentHero; snapshot issues banner (typed `warnings[]`/`errors[]` surfaced honestly); Tabs mode switch with hint line; both mode panes stay mounted (`hidden` toggling) so a half-built stack selection survives mode switches; JobCenter (persisted `logPanelOpen` pref); ToolDetailDrawer (live refresh on open, dependents + tool log lines passed in); PlanReviewModal; `onMount` → `ensureInitialized()` + `ensureScanRunning()` (§4.2: cache renders immediately, background read-only scan, incremental merge never erases cards) |
| `src/routes/environment/+page.svelte` | **deleted** (old hardcoded-color standalone page) |
| `src/routes/environment/+page.ts` | **new** — `redirect(308, "/toolchain")`: legacy alias keeps working |
| `src/lib/core/navigation.ts` | nav href `/environment` → `/toolchain` (match rule already covered both; fixes architecture-doc issue #16 where `/toolchain` had no route file) |

## 5.5 UX state matrix (verified in code)

first-run loading · no-scan-data (EmptyState + start-scan action) · stale data (header + hero badges, amber verdict) · partial/cancelled scan (hero note + typed issue banner + terminal strip) · empty search/filter results (compact EmptyState + reset) · unsupported platform (card badge, drawer section, plan gating) · manual-only (badge + instructions in drawer/profile) · permission warnings (cards/profile/plan + explicit consent checkbox) · insufficient disk (plan blocked reason + red badge) · failed/cancelled/interrupted jobs (badges, sanitized errors, retry, «после перезапуска») · source-unavailable (`install_unavailable` badge) · health unknown ≠ healthy everywhere. Raw output lives only in expandable log areas; toasts carry sanitized messages.

Settings entry point deliberately omitted: the app has no toolchain-specific
settings destination yet (lightweight prefs persist automatically via
`stackpilot:toolchain:prefs`).

## 5.6 Commands run and results

| Command (workdir) | Result |
|---|---|
| `cargo test --quiet` (src-tauri) | **pass** — 619 passed, 0 failed, 1 ignored (live-network Qt test, by design) |
| `cargo clippy --lib` (src-tauri) | **pass** — only pre-existing dead-code notes (`tc_get_platform_capabilities`, `tc_create_install_plan`); nothing new |
| `cargo fmt --check` (src-tauri) | **pass** (touched files formatted) |
| `npm run check` (root) | **pass** — svelte-check: 0 errors, 17 warnings = exact pre-existing baseline (a11y/CSS items in untouched `create/+page.svelte` / `devlauncher/*` files); zero new warnings |
| `npm test` (root) | **pass** — vitest: 5 files, 67 tests (63 baseline + 4 new filter/sort tests) |
| `npm run build` (root) | **pass** — adapter-static SPA build succeeds |

## 5.7 Known limitations carried forward

1. Update flow executes as engine `update` operation (re-plan/reinstall semantics);
   dedicated incremental updaters remain a later stage (architecture doc #19/#20).
2. `recommended` profile group stays reserved-empty until the catalog gains
   recommendation metadata (honest absence rendered as an explanatory note).
3. Drawer "adopt" (`tcx_adopt_tool`) is not surfaced yet (rarely needed action;
   command remains available).
4. Project-type presets translate into concrete languages/tools at selection time
   because `ProjectRequirements` has no project-type field (wizard-compatible by
   construction; backend resolve unchanged).
5. Job history logs are available only for jobs observed this app session;
   records restored from disk show an explanatory note instead of fake logs.

---

# Stage 6 — Project Creator integration over the canonical engine (2026-08-22)

Scope: close the Project Creator side of the refactor without breaking the working
wizard. Explicit requirements mapping for Build Environment, stale-event protection
for the wizard's event streams, secret-hygiene hardening, regression tests for all
listed wizard scenarios, and the normative compatibility appendix in the contract.
No wizard UI redesign; no new commands.

## 6.1 What changed

| File | Change |
|---|---|
| `domain/profile.rs` | **New canonical request type `EnvironmentProfileRequest`** with explicit `from_project_requirements()` mapping (order-preserving dedupe) and lossless inverse used solely to feed the single-source resolver `core::requirements::resolve` — zero logic duplication. `build_profile()` now takes the canonical request; `EnvironmentProfile.requirements` echoes it (wire shape identical to the legacy payload — same seven snake_case fields). Semantics of languages/frameworks/tools/local_infra_tools/git_init/vscode_config/docker pinned by tests |
| `commands.rs` | `tcx_profile_resolve` maps the legacy wizard payload into the canonical request explicitly (wire payload unchanged). Completion-summary math extracted from finalize into testable `legacy_completion_summary`; one-shot secret delivery extracted into `take_pending_secrets`. 4 new PC-compat adapter tests (interrupted→Skipped-with-restart-reason mapping, completion summary counts/names, take-once secrets, order+Qt-options preservation in legacy plans) |
| `create/+page.svelte` | Stale-event protection: after `tc_run_install` resolves, the wizard adopts the authoritative `session.plan.session_id` (= engine job_id) and filters foreign `task_event`/`install_done` payloads before they touch task-state maps; check progress filters late tails by per-run `scan_id`. Dead `newSecrets` snapshot-restore branch removed (secrets never persisted nor restored from sessionStorage) |
| `stateLogic.ts` / `compat.ts` | New pure rule `identityMatches(current, incoming)` shared by both event streams; exported through compat as part of the PC surface. 5 unit tests |
| `docs/toolchain-contract.md` | **§9 normative compatibility appendix**: command adapters table, event-mapping rules, requirements mapping semantics, migration rules, deprecated APIs + removal criteria |

## 6.2 Regression coverage vs. required scenario list

| Scenario | Covered by |
|---|---|
| Python/FastAPI-like requirements | `profile::python_fastapi_requirements_map_into_required_groups` (+ ~25 pre-existing resolve tests) |
| Node/React-like requirements | `profile::node_react_requirements_map_into_required_groups` |
| Docker required | `profile::docker_flag_adds_docker_tool_as_required` |
| PostgreSQL/Redis Docker optional | `profile::postgresql_redis_default_to_optional_docker_not_required`, `dual_docker_tool_defaults_to_optional_docker_managed` |
| Local infrastructure opt-in | `profile::local_infra_opt_in_for_both_tools_moves_them_to_required`, `local_infra_opt_in_moves_tool_to_required` |
| Manual-only tools | `profile::manual_only_tool_never_enters_required_group`, planner `manual_install_tools_never_scheduled`, canonical-plan skip tests |
| Already-installed tools | planner `installed_tools_are_not_scheduled`, `selected_ids_never_include_installed` |
| Update-available tools | planner `missing_and_outdated_become_tasks` |
| Insufficient disk | check.rs space-math tests (`enough_space=false`) |
| Admin warning | check `needs_admin_any` tests + `admin_warning_emitted_when_required_needs_admin` (elevation on/off texts) |
| Failed task | pc_compat `cancelled_and_failed_map_to_legacy_states`, engine terminal-math suite |
| Cancelled installation | pc_compat session-status mapping (Cancelled), engine cancellation tests |
| Route navigation during install | frontend `reSyncLiveSessions()` reconnect over `tc_get_install_status`; backend journal recovery keeps truthful state while away |
| App restart during install | jobs `running_session_recovers_as_interrupted`, pc_compat `interrupted_task_maps_to_skipped_with_restart_reason`, engine restart-recovery suite |
| Final install completion | pc_compat `completion_summary_counts_success_and_names_failures`, `legacy_plan_preserves_ids_and_maps_states`, `legacy_plan_preserves_order_and_install_options` |
| Secret retrieval | pc_compat `take_new_secrets_is_one_shot`, metadata `view_excludes_secrets_and_save_strips_them`, serde-skip on `InstallSession.secrets` |
| Stale job ids corrupting wizard state | stateLogic `identityMatches` suite (5 tests); wizard adopts authoritative session_id post-start |

## 6.3 Commands run and results

| Command (workdir) | Result |
|---|---|
| `cargo test --quiet` (src-tauri) | **pass** — 632 passed, 0 failed, 1 ignored (live-network Qt test, by design); up from 619 after adding 13 backend tests |
| `cargo clippy --all-targets` (src-tauri) | **pass, 0 errors** — remaining warnings are the documented pre-existing dead-code compat shims + minor pre-existing items; nothing new introduced |
| `rustfmt` + `cargo fmt --check` (src-tauri) | **pass** — touched files formatted; tree clean |
| `npm run check` (root) | **pass** — svelte-check: 0 errors, 17 warnings = exact pre-existing baseline |
| `npm test` (root) | **pass** — vitest: 5 files, 72 tests (67 baseline + 5 new identity-guard tests) |

Single-implementation audit: all `tc_*`/`tcx_` invokes live only in
`src/lib/modules/toolchain/api.ts`; `create/+page.svelte` imports toolchain
exclusively through `compat.ts` (3 import blocks); backend has exactly one
requirements resolver (`core::requirements`), one plan builder pair (legacy
canonicalize → engine planner) and one executor (engine pipeline) — the wizard
reaches them only through the §9 adapters.

## 6.4 Known limitations carried forward

1. Wizard still speaks the legacy protocol (`tc_*` + bridged events); migration to
   native `tcx_start_job`/`toolchainx:job_event` remains a separate stage per §9.5.
2. Check-progress filtering accepts events lacking `scan_id` unconditionally
   (legacy-backend tolerance) — acceptable because the backend always sets it now.
3. `tc_check_environment` still runs its own parallel detection rather than reusing
   scan-engine snapshots; honest-but-slower path kept until wizard migrates.
4. Engine enum serialization still deviates from §8.1's blanket rule (Stage 4
   note #3 stands).

---

---

# Stage 7 — final acceptance & hardening (2026-08-22)

Scope: end-to-end verification of the whole Toolchain Control Center (backend,
frontend, docs), removal of every production-relevant placeholder found, and the
final report. No feature was assumed from previous session logs — everything was
re-verified against the actual sources and re-run commands.

## 7.1 Verification results (exact, all re-run in this session)

| Command | Result |
|---|---|
| `cargo fmt --check` (src-tauri) | **pass** — tree fully formatted |
| `cargo check --tests` (src-tauri) | **pass** — 0 errors |
| `cargo test` (src-tauri) | **pass** — 624 passed, 0 failed, 1 ignored (live-network Qt resolution test, by design); 8 legacy-planner tests removed together with their dead function |
| `cargo clippy --all-targets` (src-tauri) | **pass, 0 errors**; toolchain scope: zero dead-code warnings remaining; rest are pre-existing style notes (too_many_arguments on process-launcher signatures etc.) outside hardening scope |
| `npm run check` (root) | **pass** — svelte-check: 0 errors, 17 warnings = exact pre-existing baseline (`create/+page.svelte`, `devlauncher/*` files untouched by this stage) |
| `npm test` (root) | **pass** — vitest 5 files, 75 tests (72 baseline + 3 new scan-label tests) |
| `npm run build` (root) | **pass** — adapter-static SPA build succeeds |

## 7.2 Files changed in this stage

| File | Change |
|---|---|
| `src-tauri/.../toolchain/commands.rs` | **mojibake repaired**: 124 double-encoded comment/log lines restored to proper UTF-8 (incl. user-visible `eprintln!` diagnostics and an error message of `tcx_get_tool_details` that rendered as garbage); deleted dead unregistered commands `tc_get_platform_capabilities` + `tc_create_install_plan` (contract §9.5 cleanup criterion met: no registration, zero frontend references); stale doc reference fixed |
| `src-tauri/.../core/planner.rs` | deleted dead legacy `build_plan(check, selected)` + its 8 tests (semantics remain covered by engine planner + canonicalize_plan tests) |
| `src-tauri/.../domain/detect.rs` | removed unused `managed_ids`; `rules_have_any_evidence` gated `#[cfg(test)]`; `to_legacy_status` kept with documented reason (contract §8.8 compat surface, table-tested); imports pruned |
| `src-tauri/.../core/secrets.rs` | removed vestigial never-set/never-read `degraded_reason` field+method; `get_secret`/`keys` annotated as test/future-generation accessors with reasons |
| `src-tauri/.../mod.rs` | legacy secrets now migrate through `MetadataStore::take_legacy_secrets()` (single legal extraction path instead of reaching into internals); explicit startup warning when DPAPI is unavailable and secrets would sit unencrypted |
| `src-tauri/.../core/metadata.rs` | removed unused `is_adopted` (adopted set is exposed truthfully via the metadata View) |
| `src-tauri/.../domain/mod.rs` | `ConcurrencyGauge` gated `#[cfg(test)]` (test-only utility) |
| `src-tauri/.../core/path_service.rs` | `remove_from_user_path` kept with reason: reversibility primitive required by contract §5.7 for future uninstall/rollback jobs |
| `src-tauri/.../core/archive.rs` | `extract_zip_safe` kept with reason (reference traversal-safe extractor until unpack-script migration); lint cleanups (doc-comment merge, `rsplit`) |
| `src-tauri/.../domain/path_report.rs` | removed dead `collect_process_report` (per-tool PATH findings cover the shipped UI); `_index` rename |
| `src-tauri/.../domain/engine.rs` | scan-journal rename failure now logged instead of silently ignored |
| `src-tauri/.../models.rs`, `core/check.rs`, `core/disk.rs`, `domain/profile.rs` | lint cleanups: unused accessor removed, `is_err()`, `is_some_and`, underscored test locals |
| `src/lib/modules/toolchain/format.ts` | new `scanPhaseLabel()` / `scanTerminalLabel()` — raw PascalCase backend values no longer leak into the Russian UI |
| `src/lib/modules/toolchain/components/ActivityStrip.svelte`, `JobCenter.svelte` | use the new scan phase/terminal labels |
| `src/lib/modules/toolchain/components/ToolDetailDrawer.svelte` | focus restoration to the triggering element on close (was focus-in only); global `.ok/.fail` class leakage replaced with `.checks :global(…)` scoping; new honest «отслеживается» badge + «Отслеживать» action wired to `tcx_adopt_tool` (offered only for external/unknown provenance) |
| `src/lib/modules/toolchain/components/ProfileResult.svelte` | scoped `.admin-icon` global selector |
| `src/lib/modules/toolchain/state.svelte.ts` | adoption state + `adoptTool()`/`refreshAdopted()`; job-log memory bounded to jobs visible in history/current (pruned at finalize); prefs persistence debounced while typing search (no localStorage write per keystroke) |
| `src/routes/toolchain/+page.svelte` | passes adopted/onadopt into the drawer |
| `src/lib/modules/toolchain/format.test.ts` | 3 new tests for the scan labels |
| `docs/toolchain-contract.md` | status final v3; normative engine-enum external-tagging exception; adoption surface documented; §9.5 marks the two dead commands deleted |
| `docs/toolchain-architecture.md` | status final v2; new §0 shipped-architecture inventory (exact command/event/persistence list); §5.0 resolution table for all 30 audit issues; §6.2 rewritten to actual test coverage |
| `docs/toolchain-acceptance.md` | **new** — concise manual acceptance checklist |

## 7.3 Placeholders / stubs audit (final state)

Searches performed over toolchain scope for TODO/FIXME/unimplemented/todo!/
"coming soon"/placeholder/mock/fake/hardcoded/demo and for empty catch blocks:

- All matches are legitimate: test fixtures named `fake_*`, HTML input
  `placeholder` attributes, a CSS class named `.placeholder` for an empty-state.
- Zero TODO/FIXME/unimplemented markers in production paths.
- Silent error swallowing: event-emission failures (`let _ = app.emit(...)`)
  are deliberate Tauri practice; the one silent disk failure (scan journal
  rename) now logs. Mutex `expect("... poisoned")` guards are intentional.
- Genuinely unsupported platform operations are represented honestly through
  capabilities/planner rejection (see §7.6), not stubs.

## 7.4 Features completed (system-level)

1. Two modes on one canonical route `/toolchain`: Manage Everything (search,
   filters, sorting, per-tool actions) and Build Environment (wizard-tree
   selection → backend profile resolution with per-tool inclusion reasons →
   plan review → engine job).
2. Cached-snapshot-first rendering with automatic read-only diagnostic scan on
   entry; incremental per-tool merge that never erases cards; cancel support;
   honest stale/partial/cancelled states everywhere.
3. Backend-authoritative install/update/repair-PATH/health-check jobs:
   restricted requests (deny_unknown_fields), fresh canonical plans persisted
   before execution, execution-time source revalidation, integrity gates
   (sha256 or explicit confirmation), audited PATH changes, cancellation that
   kills the running task, restart recovery to Interrupted, typed events with
   monotonic seq, bounded history ring.
4. Truthful presentation model: 13-state composition, scan-failed ≠ missing,
   health unknown ≠ healthy, manual/docker/built-in/unsupported states,
   provenance facts, 8 independent platform capability flags.
5. Secrets: generated CSPRNG, DPAPI-isolated store, take-once delivery channel,
   absent from every read model/job record/log; plaintext-degradation warning.
6. Project Creator keeps working unchanged over the compat layer (§9): same
   engine underneath, bridged legacy events with stale-event protection.
7. Accessibility & UX quality: labelled controls, keyboard-accessible filter
   rail, aria-selected tabs, dialog semantics, focus moved into drawer/modal
   and restored on close, reduced-motion handled globally by theme tokens,
   sanitized error messages.

## 7.5 Compatibility status

- All 11 legacy `tc_*` commands registered and byte-compatible; wizard flows
  regression-covered by adapter mapping tests.
- Legacy events flow alongside `toolchainx:*`.
- `state.json` additive-only; old snapshots/journals deserialize with defaults;
  Running records recover to Interrupted (never fake success).
- `/environment` redirects (308) to `/toolchain`; nav item covers both.

## 7.6 Remaining platform limitations (honest, user-visible)

1. Install/update **execution** is Windows-first; Linux/macOS plans are
   rejected at planning (`PlatformUnsupported`), sources exist in catalog but
   no executor exists. Detection/health work cross-platform.
2. Elevation (UAC) is Windows-only; admin-needing plans are rejected where
   elevation is unsupported.
3. Qt online pipeline is Windows/msvc2022_64, pinned to the Qt 6.8 branch.
4. Update operation = re-plan/reinstall (winget upgrade semantics); no
   incremental updaters yet.
5. No uninstall operation yet (PATH removal primitive exists and is tested).
6. True mid-task resume: interrupted jobs recover to a truthful Interrupted
   state and are retryable, but a partially downloaded artifact restarts.
7. `recommended` profile group stays reserved-empty until catalog gains
   recommendation metadata; `Provenance::PackageManager` exists in the model
   but scans cannot prove a winget/apt origin (stays external).

## 7.7 Remaining non-critical technical debt

1. `create/+page.svelte` (~3 700 lines) still hosts the whole wizard; extract
   before further UI work there.
2. Tool-specific post-install hooks (MAUI/php.ini/composer shim) remain
   imperative special cases pending a declarative hook mechanism.
3. Qt listing/Updates.xml hand-parsing; pinned "68" prefix check.
4. Whole-user-PATH replace race window (§5 issue #28).
5. eprintln-based logging without levels/sinks; structured events exist only
   for the job/scan engines.
6. Engine enum encoding deviates from the domain blanket rule (now documented
   as a normative exception in contract §8.1).
7. `defs::validate` warnings beyond duplicate ids are computed but not surfaced.
8. Component-level frontend tests (rendering/a11y) not configured.

## 7.8 Security limitations

1. Secrets at rest rely on DPAPI (Windows) / file mode 0600 (Unix); no OS
   keyring integration (would require external crates). Degradation is loud.
2. Sources without sha256 can be installed after an explicit per-plan
   confirmation; the risk is surfaced in plan review and drawer badges.
3. `run_elevated` argument quoting remains fragile for exotic args (UAC path).
4. Threat model is local single-user: anyone who can read
   `<app_data>/toolchain/secrets.bin` as the same OS user can decrypt it.

## 7.9 Exact manual verification steps

See `docs/toolchain-acceptance.md` (kept concise and checklist-shaped).

---

