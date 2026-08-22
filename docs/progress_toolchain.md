# Progress: standalone Toolchain

## Current phase

Phase 6 — final security, stub/dead-code, contract-validation and
production-hardening pass over the standalone Toolchain refactor.
**Status: COMPLETE** (all matrix items green, limitations listed at the
bottom). Project Creator frontend integration untouched; legacy `tc_*`
signatures/payloads byte-compatible (full legacy suite passes).

Baseline for Phase 6: Phase 5 status (`cargo test` 659 passed / 1
ignored, `npm run test` 171 tests, `svelte-check` 0 errors) + the full
audits described below.

## Phase 6 summary

Three audits were run first (backend engine/planner/installer,
frontend module, legacy core) and every finding was triaged into
fix / documented-limitation / rejected-as-not-applicable. Details:

### Security audit (24 items) — result

| # | Item | Status |
|---|---|---|
| 1 | Frontend cannot provide arbitrary URL | held (deny_unknown_fields + tests) |
| 2 | Frontend cannot provide arbitrary executable | held (planner builds commands from catalog only) |
| 3 | Frontend cannot provide arbitrary installer args | held; **+hardened**: `install_options` for non-Qt tools now rejected by the planner (was silently ignored) |
| 4 | Source IDs validated against catalog | held (validate_requested_source + exec-time revalidation) |
| 5 | Unknown tool IDs rejected | held; **+hardened**: `tcx_run_health_checks` now returns an error for unknown ids (was silently ignoring them) |
| 6 | Unknown install options rejected | Qt whitelist held; non-Qt options now rejected at plan time |
| 7 | Only approved URL schemes | held (https / localhost-only http) |
| 8 | HTTPS enforced | held |
| 9 | Redirects safe and bounded | **+hardened**: explicit `MaxAutomaticRedirections = 5` in the download script (was .NET default 50); https→http downgrade stays blocked by the handler |
| 10 | Download size bounded | **+hardened**: hard 4 GiB per-installer cap enforced inside the download script (was unbounded stream) |
| 11 | SHA-256 verified when present | held |
| 12 | Unverified sources require explicit confirmation | held |
| 13 | Temp files unique + cleaned on success/failure/cancel | held; **+hardened**: `console::sweep_stale_temp_files()` at startup removes crash-orphaned `tc-*` files (>24 h) — panic-safe path covered |
| 14 | Archive traversal rejected fail-closed | held; **+hardened**: production extraction now runs through the single safe layer `archive::extract_zip_safe`/`extract_tar_safe` (the inline duplicate scripts in `installer.rs` were removed) |
| 15 | Symlinks/link-like entries handled safely | held (whole archive rejected) |
| 16 | Installation destinations validated | **+hardened**: Qt `resolve_target` rejects `..` and any escape from `install_dir` (target arg comes from remote Updates.xml); `add_to_user_path` rejects non-absolute entries |
| 17 | PowerShell quoting safe | held (ps_quote / CommandLineToArgvW quoting) |
| 18 | User-controlled strings never interpolated unescaped | held (only ids/options/bools/fingerprint cross the boundary) |
| 19 | `-ExecutionPolicy Bypass` minimized | **+hardened**: removed from all `-Command` invocations (archive scripts deleted, MSIX script, kafka probe in tools.json); stays only where required (`-File` scripts) |
| 20 | Secrets never in logs/events/persistence | held (RedactingSink, secret store, one-shot take, serde-skip) |
| 21 | Redaction before persistence/emission | held (task errors redacted before job record writes) |
| 22 | Path repair cannot write arbitrary paths | held (entries come from catalog only) |
| 23 | Cancellation terminates processes | held (piped_run kill + abort polling) |
| 24 | Restart recovery rebuilds canonical plan | held (`tcx_retry_job` re-runs `build_canonical` from fresh detection; no stale command resume) |

Other security fixes from the audits:

- `core/discovery::run_capture` (shared with legacy health) now caps
  captured output at 64 KiB — a noisy/broken binary cannot exhaust
  memory (was unbounded).
- MSIX winget-bootstrap script documents its integrity model: the
  dependencies zip is a pinned official winget-cli release whose
  contents are Microsoft-signed appx packages (Add-AppxPackage
  validates signatures).
- Legacy `to_legacy_status` documented as a byte-compat-only surface
  (ScanPending/ScanFailed → Missing is unavoidable in the old enum;
  standalone never uses it).

### Stubs / dead definitions — result

Removed: `ToolState::InstallUnavailable` (never produced by any code
path; removed from backend enum, StatusCounts, score, frontend union,
filters, format), `StatusCounts::total`, `PathScope::explanation`,
`Provenance::from_metadata`, `ToolchainController.parseState`,
`ToolchainState::get_definition_merged`, `ToolchainState::legacy_definitions()`,
duplicate archive-extraction scripts in `installer.rs` (superseded by
`archive.rs` safe layer), `ExecutionChoice` frontend type (duplicate of
`ExecutionMode`), duplicated scan-terminal helpers (unified behind
`scanIsTerminal`).

Kept with explicit documentation: legacy `remove_from_user_path` /
`MetadataStore::tool` / `meets_min` (documented future consumers),
`to_legacy_status` (contract §7 total mapping), `NoopReason::DockerManaged`
(old persisted plans), `check.rs` optional-requirements backfill (legacy
caller-owned), `qt_installer` 6.8-branch pin and fallback version list
(data-driven pin, documented), module-level `allow(dead_code)` in
`engine/` and `domain/profile.rs` (documented public-surface growth).

Data truthfulness: terraform `recommended` fixed 1.9 → 1.15 (shipped
zip is 1.15.8; stale advisory would mislabel health); kafka probe no
longer carries a needless `-ExecutionPolicy Bypass`.

### Contract validation (standalone guarantees) — result

- **48-tool blind scan**: held — scan iterates the standalone catalog
  only (`tcx_get_catalog`); engines physically absent (test-guarded).
- **Counts = data**: `ScoreSummary` required-* counters removed
  entirely; `scan_pending` bucket added so a partial report no longer
  inflates `scan_failed` (regression-tested).
- **One source of truth per status**: held (snapshot is authoritative).
- **Version or explicit explanation**: **+fixed** — ToolCard and
  MarketplaceCard now render «версия не определена» with a tooltip for
  footprint-only installs instead of a silent «—»; unparsed markers
  carry tooltips.
- **Health truthful**: held (verdicts only Healthy/Degraded/Unhealthy).
- **Docker never silently substitutes local**: held (planner rejects
  `execution: docker`; UI label «локальная установка» now gated on
  `execution_mode === "host"`).
- **Marketplace without snapshot**: held; **+fixed** — runtime filter
  groups (state/health/provenance/update) are inert without scan facts
  (persisted filters from Manage Everything could previously hide the
  whole marketplace with no visible control), and platform-resolution
  failure now shows a retryable error instead of an eternal
  «Определяем платформу…».
- **Filters work**: held (+regression tests; `admin_only` etc.).
- **Score explainable**: held; **+fixed** — live-scan placeholder
  states (empty version) no longer count as healthy 100 mid-scan;
  summary reads are NaN-safe.
- **Plan preview/execution terminate**: held (backend 45 s planner
  deadline); **+fixed** — frontend preview flight has a 90 s timeout
  and a request-generation guard (stale responses from a previous
  modal session can no longer overwrite the current plan).
- **Malformed IPC cannot crash UI**: **+fixed** — `jobEventLogText`
  (moved to stateLogic, total function) and `applyJobEvent` now guard
  every inner payload (`task_phase.phase`, `task_completed.status`,
  `job_finished.status`, `path_updated.record`, `job_started`); the
  last surviving `"running" in null` render crashes in ActivityStrip
  and JobCenter are guarded; `recomputeSummary`/`classifyScoreTool`/
  `scoreBreakdown` are NaN- and malformed-proof. This closes the
  `already_installed in undefined` family for good (regression-tested).
- **Startup non-blocking**: held (audit verified; no sync work).
- **Project Creator compatible**: held (legacy suite green).

## Files changed (exact) — Phase 6

Backend (`src-tauri/src/modules/toolchain/`):

- `core/console.rs` — hard download size cap (4 GiB) + explicit
  redirect cap (5) inside the extracted `download_script`; startup
  `sweep_stale_temp_files` (crash-orphan cleanup); tests.
- `core/discovery.rs` — 64 KiB output cap in `run_capture`; test.
- `core/qt_installer.rs` — `resolve_target` rejects `..`/escapes from
  `install_dir` (remote-`Updates.xml`-controlled target); tests.
- `core/installer.rs` — archive extraction now exclusively via
  `archive::extract_zip_safe`/`extract_tar_safe` (duplicate inline
  scripts + `-Command -ExecutionPolicy Bypass` removed); MSIX script
  Bypass removed + integrity note; tests updated/added.
- `core/path_service.rs` — `add_to_user_path` rejects non-absolute
  entries; tests.
- `domain/models.rs` — `ScoreSummary` without required-* counters,
  `scan_pending` added; `InstallUnavailable` removed; `ToolScanResult::
  pending` computes real applicability/capabilities from the catalog;
  dead helpers removed; tests.
- `domain/score.rs` — pending ≠ scan_failed bucket; field renames;
  tests.
- `domain/cache.rs` — future-dated snapshots are stale, not fresh;
  test.
- `domain/detect.rs` — «?» recommended-version placeholder replaced
  with honest empty; legacy-mapping limitation documented.
- `domain/engine.rs` — partial-snapshot `pending` call passes real
  platform facts.
- `engine/planner.rs` — non-Qt `install_options` rejected; test.
- `commands.rs` — `tcx_run_health_checks` fails closed on unknown ids.
- `mod.rs` — startup temp sweep; dead accessors removed.
- `tools.json` — kafka probe `Bypass` removed; terraform `recommended`
  → 1.15.

Frontend (`src/lib/modules/toolchain/`):

- `stateLogic.ts` — guarded `jobEventLogText` (moved here), guarded
  `applyJobEvent`, NaN-proof `recomputeSummary`, honest live placeholder
  (`applicability: unknown`, not installable).
- `state.svelte.ts` — dead `parseState` removed; marketplace platform
  failure surfaces `marketplaceOsError`.
- `types.ts` — ScoreSummary/StatusCounts/ToolState cleanup,
  `PlatformApplicability` += `unknown` (live placeholders only),
  `ExecutionChoice` removed.
- `format.ts` — state labels cleanup; `planWarningInfo` returns
  machine-readable `kind` (no text matching for confirmations).
- `filters.ts` — known-state/severity tables cleaned.
- `score.ts` — placeholder-version guard (no mid-scan score
  inflation), malformed-tool guard.
- `events.ts` / `startup.ts` — duplicated scan-terminal helpers
  unified behind `scanIsTerminal`.
- `marketplace.ts` — runtime filter groups inert without scan facts
  (states/provenance/health/update_only).
- Components: `PlanReviewModal.svelte` (generation guard + 90 s
  timeout + kind-based confirmation counts + host-label gating),
  `ToolCard.svelte` / `MarketplaceCard.svelte` («версия не
  определена» + tooltips + `def?.versions?.` guards),
  `ToolDetailDrawer.svelte` (`versions?.` guard),
  `ActivityStrip.svelte` / `JobCenter.svelte` (`null`-status guards),
  `MarketplaceMode.svelte` (platform-error retry),
  `EnvironmentHero.svelte` (NaN-safe summary sums).
- Tests: `stateLogic.test.ts` (+11), `score.test.ts` (+3),
  `marketplace.test.ts` (+2, 1 re-asserted), `types.test.ts`,
  `format.test.ts`, `filters.test.ts` (fixtures updated).

NOT touched (verified via `git status`): `routes/create/+page.svelte`,
`lib/modules/project_creator/**`, project_creator backend, legacy `tc_*`
signatures/payloads, `MarketplaceCard.svelte` layout/visual language,
legacy catalogs (`legacy_compat_tools.json`).

## Tests run (final matrix)

- `cargo fmt --check` → PASS.
- `cargo clippy --lib` → no errors (124 pre-existing warnings, none
  introduced by this phase).
- `cargo test` (workspace lib) → **667 passed; 0 failed; 1 ignored**
  (was 659/1 before this phase; +8 new toolchain tests).
- `npm run check` (svelte-check) → **0 errors**; same 17 pre-existing
  a11y warnings (all in unrelated files).
- `npm run test` (vitest) → **10 files, 181 tests passed** (was
  171).
- `npm run build` (vite build) → PASS.

New backend regression tests (this phase):

| Test | Guards |
|---|---|
| `qt_installer::target_dir_rejects_traversal_from_remote_xml` | `..` / backslash traversal from remote Updates.xml rejected fail-closed |
| `qt_installer::target_dir_allows_safe_subdirectories` | legitimate subdirs still work |
| `discovery::capture_output_is_capped` | noisy binary cannot exhaust memory |
| `console::download_script_carries_size_and_redirect_bounds` | 4 GiB cap + 5-redirect cap in the actual script |
| `console::sweep_removes_only_old_orphaned_temp_files` | crash-orphan cleanup at startup, fresh files untouched |
| `path_service::add_to_user_path_rejects_relative_entries` (+ `absolute_entry_detection_matches_platform`) | no junk PATH writes |
| `installer::archive_source_rejects_command_building` | extraction lives only in archive.rs |
| `score::scan_failed_and_pending_never_count_as_failure` | partial report no longer inflates scan_failed |
| `models::pending_result_carries_real_applicability_from_catalog` | partial-snapshot pending is not fake "Installable" |
| `cache::future_dated_snapshot_is_stale_not_fresh` | clock-skewed snapshot cannot masquerade as fresh |
| `planner::install_options_for_non_qt_tools_are_rejected` | unknown install options fail closed |

New frontend regression tests (this phase):

| File / case | Guards |
|---|---|
| `stateLogic.test.ts` — malformed nested job-event payloads (`task_phase: null`, `status: null`, `job_finished: null`, non-string status) don't crash `applyJobEvent` | `already_installed in undefined` family, IPC boundary |
| `stateLogic.test.ts` — `jobEventLogText` total over 20+ malformed payload shapes | journal line never throws |
| `stateLogic.test.ts` — `recomputeSummary` no NaN on unknown/null kinds | summary counts stay numbers |
| `stateLogic.test.ts` — live placeholder carries `applicability: unknown` | no fake installable claim mid-scan |
| `score.test.ts` — empty-version live placeholders not counted as healthy 100; malformed tools don't crash breakdown | no mid-scan score inflation; no crash |
| `marketplace.test.ts` — runtime filter groups inert without scan data (and `update_only`), definition facts still filter | marketplace visibility trap |
| `filters.test.ts`/`types.test.ts`/`format.test.ts`/`score.test.ts` fixtures | removed required-*/install_unavailable fields |

## Remaining limitations (honest)

1. **Catalog integrity coverage**: only the Swift Windows source ships
   a sha256 in `tools.json`. All other direct-download sources are
   honestly marked `unverified` and require the explicit confirmation
   checkbox. Filling the rest requires live verification of official
   hashes (not guessed; documented as future work).
2. **Qt package downloads** have no checksums (Qt's Updates.xml ships
   none) — honest `unverified`; the Qt 6.8 branch pin and fallback
   version list are hardcoded data that must be bumped with the
   catalog `recommended`.
3. **Legacy `tc_*` surfaces** still run on the merged catalog and use
   `ToolStatus`-level semantics (`to_legacy_status` compresses
   ScanPending/ScanFailed into Missing — impossible to express
   otherwise in the old enum; standalone never uses it).
4. **Legacy health/discovery** (`core/health.rs`, `core/discovery.rs`)
   keep their coarser verdicts for Project Creator; the standalone
   page uses the domain pipeline only.
5. **Malformed-snapshot coverage**: the hardened guards cover the
   aggregation and event paths plus the known render sites; a
   wholesale-validated snapshot renderer remains future work if the
   backend ever becomes a hostile boundary.
6. **`-ExecutionPolicy Bypass` remains for `-File` script execution**
   (downloaded unsigned scripts: dotnet-install.ps1 etc.) — required
   by the OS policy, kept only there.

## Decisions made

- One extraction implementation (`archive.rs`) is now the only path
  for zip/tar installs; the installer's inline scripts were deleted
  rather than kept as a second copy — traversal policy cannot drift.
- Hardening that touches wire formats is additive or
  frontend-unused: `ScoreSummary` field changes are safe because the
  UI only reads `score`/`counted_tools`; old cached snapshots still
  deserialize (unknown fields ignored, new fields defaulted).
- Frontend plan flights get an explicit timeout and a generation
  guard because «terminates» must hold on the client side too, not
  just in the backend planner.
- Runtime marketplace filters are inert (not reset) without scan
  data: resetting user prefs on mode switch would be data loss; a
  silent hide was the bug.

## Notes for next session

- Baseline after Phase 6: `cargo test` 667/1, `npm run test` 181,
  `svelte-check` 0 errors, build green.
- If a new direct-download source is added to tools.json, prefer
  shipping an official sha256; otherwise the confirmation checkbox
  is mandatory and honest.
- If a canonical OS-only command is added later, swap it in
  `ensureMarketplacePlatform` (see Phase 4 notes).
- Qt 6.8 pin (`qt_installer.rs` `pick_latest_version_dir` branch
  filter + fallback list) must be updated when the catalog moves off
  6.8.x.

## Phase 5 summary

The page becomes interactive immediately: the shell (header, tabs,
hero, drawer, actions) renders on first paint; cached snapshot,
definitions, metadata, scan reconnect and job history load in
PARALLEL fire-and-forget flights; nothing blocks navigation. Every
async entry point now has an in-flight guard (request coalescing) or
an identity/staleness guard, so duplicate IPC calls, duplicate scan
starts, duplicate listeners and stale responses cannot happen.

### Startup orchestration (`state.svelte.ts`)

- `ensureInitialized` — `OnceInitializer` guard: listeners and startup
  flights run exactly once per app; page remount is a no-op (no
  duplicate subscriptions). Prefs load synchronously; then 4 parallel
  fire-and-forget flights: snapshot, reconnect jobs, definitions,
  adopted metadata.
- `refreshSnapshot` — single in-flight flight for `tcx_get_environment_snapshot`.
  Foreground callers (initial load, retry button) drive
  `snapshotLoading`/`snapshotError`; background callers (scan done,
  job finished) join the same flight silently. The loading flag always
  resets (counter-based release) — no stale spinner; a failed request
  keeps the previous valid snapshot and surfaces the error to any
  foreground waiter.
- `ensureScanRunning` — `ScanStartCoordinator`: reconnect
  (`tcx_get_latest_scan_job`) always completes BEFORE the start
  decision, so a stale terminal scan from reconnect can never
  overwrite a freshly started scan; `tcx_start_scan` runs at most once
  per flight; concurrent callers (page mount + header button +
  ActivityStrip retry) share one promise. Reconnect is itself
  coalesced (`#scanReconnectOnce`, shared with `reconnectJobs`).
- `reconnectJobs` — coalesced; `getLatestScanJob` shared with the
  scan chain (one IPC total); `tcx_list_jobs` once; active job adopted
  without re-subscribing the tracker when the same job_id is adopted
  again (`#adoptJob` same-id guard — no duplicate listeners).
- `ensureDefinitions` / `refreshAdopted` — `CoalescingCall` guards;
  failure leaves `definitionsError` and allows retry.
- `loadCatalog` — now reads through the shared snapshot flight instead
  of issuing a second `tcx_get_environment_snapshot` (dead code path,
  kept API-compatible).

### Scan lifecycle

- Cached snapshot renders instantly (one IPC, backend serves from
  disk cache); scan progress overlays incrementally; final snapshot is
  refreshed exactly once per terminal event (TerminalGuard) and the
  partial overlay is cleared at `scan_done` — partial result fields
  are never rendered "as final" on top of the authoritative snapshot.
- Late progress events for a completed scan are dropped
  (`scanProgressAppliesTo`: identity + terminal check) — a done scan
  cannot be "revived" and old events cannot update it.
- Terminal states (Completed/Partial/Cancelled/Failed/Interrupted) are
  shown as before; duplicate terminal notifications prevented by the
  shared TerminalGuard.
- Scan button shows «Подключение…» while reconnecting
  (`scanReconnecting`), «Сканирование…» while running.

### Mutation lifecycle

Unchanged semantics (queued → running → phases → logs → cancel →
partial → retry, final snapshot refresh after completion, targeted
recheck only after success) plus:

- `runHealthChecks` — per-tool in-flight guard: the same tool is
  never probed twice concurrently (drawer recheck, plan-refresh stage
  and post-job recheck share flights).
- `#adoptJob` same-id re-adoption no longer re-subscribes `trackJob`
  (duplicate-listener bug fixed).

### Detail requests (drawer)

- `refreshToolDetails` — `LatestRequestGuard`: only the response for
  the CURRENTLY selected tool is applied; a slow response from a
  previously selected tool cannot overwrite the drawer.
- Per-tool in-flight coalescing: repeated requests for the same tool
  (effect re-fire on remount) share one IPC.
- New `detailsLoading` / `detailsError` state: the drawer shows
  «Обновляем данные инструмента…» while in flight and a retry line
  on failure (recoverable error state, sanitized message).

## Files changed (exact) — Phase 5

Frontend only (backend untouched in Phase 5):

- `src/lib/modules/toolchain/startup.ts` (new) — pure primitives:
  `CoalescingCall` (in-flight guard), `OnceInitializer`,
  `LatestRequestGuard`, `ScanStartCoordinator`, `scanJobIsActive`.
- `src/lib/modules/toolchain/stateLogic.ts` — event-identity guards
  `scanProgressAppliesTo` / `scanDoneAppliesTo` / `jobEventAppliesTo`
  (stale-скан/stale-задание rules on the listener boundary).
- `src/lib/modules/toolchain/state.svelte.ts` — startup orchestration,
  coalescing guards, detail staleness, liveTools lifecycle at
  `scan_done`, same-id job adoption guard, `scanReconnecting` /
  `jobsReconnecting` / `detailsLoading` / `detailsError` state.
- `src/routes/toolchain/+page.svelte` — scan button reconnect label +
  loading, drawer `busyDetails`/`detailsError`/`detailsRetry` wiring.
- `src/lib/modules/toolchain/components/ToolDetailDrawer.svelte` —
  details in-flight indicator and retryable error line.
- `src/lib/modules/toolchain/startup.test.ts` (new, 17 cases).
- `src/lib/modules/toolchain/stateLogic.test.ts` (+9 cases).

NOT touched (verified via `git status`): `routes/create/+page.svelte`,
`lib/modules/project_creator/**`, project_creator backend, legacy
`tc_*` signatures/payloads, `src-tauri/**` (Phase 5 has no backend
changes).

## Tests run

- `npm run test` (vitest) → **10 files, 171 tests passed** (was
  9/145).
- `npm run check` (svelte-check) → **0 errors**; same 17 pre-existing
  a11y warnings (all in unrelated files).
- `cargo test` — not re-run: Phase 5 contains no Rust changes; Phase 3
  baseline (659 passed / 1 ignored) still applies.

New frontend tests:

| File | Guards |
|---|---|
| `startup.test.ts` (17 cases) | duplicate calls share one flight (snapshot/scan start); run-once init (page remount does not duplicate subscriptions); always-reset loading (failed request resets loading state); error reaches ALL waiters; stale detail response dropped when the selected tool changed; reconnect always completes BEFORE start (stale terminal can't overwrite a fresh scan); reconnect-ошибка не блокирует старт; reconnect-revealed running scan suppresses start; active-scan re-entry doesn't start twice |
| `stateLogic.test.ts` (+9) | stale scan progress (other job / after terminal) dropped; stale scan done (other job) dropped; stale job events (other job_id) dropped; current-scan/current-job events apply |

## Performance validation

Reasoned bounds (per first mount of `/toolchain`):

- **Time to interactive**: page shell renders on first paint — all
  startup flights are fire-and-forget; nothing awaits them before
  render. Cached snapshot appears after one IPC
  (`tcx_get_environment_snapshot`, served from backend disk cache).
- **IPC calls on first mount**: exactly one per command —
  `tcx_get_environment_snapshot` (1), `tcx_get_latest_scan_job` (1,
  shared by reconnect + scan chain), `tcx_list_jobs` (1),
  `tcx_get_catalog` (1), `tc_get_metadata` (1), plus `tcx_start_scan`
  (1, only if reconnect revealed no running scan) and
  `tc_get_environment_info` (1, only when marketplace mounts without a
  snapshot). No command can be invoked twice concurrently.
- **Duplicate call count**: 0 by construction (coalescing guards on
  snapshot/scan-start/reconnect/definitions/adopted/details/
  health-checks).
- **Scan start count**: ≤1 per flight; a second `ensureScanRunning`
  joins the in-flight promise.
- **Listener count**: one physical Tauri listener per event name
  (EventChannel ref-counting) + one subscription per channel from the
  controller (OnceInitializer); job trackers are 1:1 with the current
  job and never duplicated for the same job_id.
- **Plan preview latency**: unchanged flow; `runHealthChecks` for the
  stale-snapshot refresh stage now skips tools already being probed
  (no queueing behind a duplicate IPC).
- **Detail drawer latency**: one `tcx_get_tool_details` per open;
  repeated opens of the same tool share the in-flight flight; stale
  responses are dropped.

## Decisions made

- Coalescing lives in pure primitives (`startup.ts`) so the race rules
  are unit-testable without the Svelte-rune controller; the controller
  delegates to them.
- `liveTools` (scan overlay) is cleared at `scan_done` — the final
  snapshot is authoritative; partial fields must never be shown "as
  final". Detail/health refreshes after a scan are unaffected (they
  write after the clear).
- Background snapshot refreshes join the shared flight silently;
  foreground waiters drive loading/error. A foreground joiner during a
  background flight gets loading + error surfaced.
- Reconnect-before-start is enforced INSIDE the scan coordinator, not
  by the controller: any future caller of `ensureScanRunning` inherits
  the ordering guarantee.

## Notes for next session

- Phase 3 baseline (backend tests) remains valid; run `cargo test`
  after any future backend change.
- If a canonical OS-only command is added later, swap it in
  `ensureMarketplacePlatform` (see Phase 4 notes) — the rest of the
  flow is independent.
- `scanReconnecting` / `jobsReconnecting` / `detailsLoading` /
  `detailsError` are new state flags with minimal UI wiring; extend
  their consumers if the design needs more granular loading displays.

## Phase 4 summary

New top-level modes on `/toolchain`: **Управлять всем · Собрать окружение
· Витрина инструментов** (third tab added, same `Tabs` design language).

### Marketplace (definition-driven, works without a scan)

- New module `marketplace.ts` (pure, unit-tested):
  - `buildMarketplaceItems(definitions, snapshot, os)` — builds the
    marketplace ONLY from the standalone catalog (`tcx_get_catalog`);
    removed tools (unity/unreal/godot) physically cannot enter the UI.
  - Works with `snapshot = null`: platform comes from `liveSnapshot.os`
    or a light read-only `tc_get_environment_info` fallback
    (`state.svelte.ts::ensureMarketplacePlatform`); scan facts are
    honestly `null` («нет данных скана») until a scan exists.
  - `platformsOfDefinition` (declared `platform_availability` ∪ actual
    per-OS sources), `sourcesForPlatform`, `capabilitiesOfDefinition`
    (mirror of backend `ToolPlatformCapabilities::for_definition`),
    `checksumStatus` (all/partial/none/no-sources per current OS).
  - Search across **display, id, aliases, category, description, notes,
    docs_url, source_url** (`matchesMarketplaceSearch` →
    extended `matchesDefinitionSearch`, defensive vs missing fields).
- `MarketplaceCard.svelte`: every required field shown — icon, display,
  category, description, aliases, platform availability, installability,
  detected state (if scan), installed version, recommended version,
  source types, checksum verification, size, admin requirement,
  dependencies, conflicts, Docker recommendation (metadata only, badge
  «альтернатива: image»), manual instructions, docs/source links,
  «Установить локально» + «Подробнее» actions.
- Install locally → `openPlan("install", [id])` → the SAME canonical
  `PlanReviewModal`: standalone `EngineRequest` (never Project Creator
  Docker optionality), backend-built plan with exact local tasks, source
  data and confirmation requirements (unverified/admin checkboxes)
  preserved. After success the state controller refreshes the snapshot in
  background AND runs a targeted read-only recheck of the requested tool
  ids (`#finalizeCurrentJob`), so cards/marketplace update immediately.

### Filters (all functional, count-backed, persisted)

- `CatalogFilters` extended with **`installable`** and
  **`has_docker_alternative`**.
- `validateCatalogFilters` (new): runtime validation of persisted prefs —
  unknown state/provenance/capability/execution/health kinds, bad types,
  duplicates and junk are discarded; never throws. Used by
  `loadPrefs()`; **all** filter groups now persist (provenance,
  capabilities, execution_modes, health, admin_only, update_only,
  manual_only, installable, has_docker_alternative), not just
  search/categories/states.
- Both Manage Everything and Marketplace rails show **live counts per
  option from the current dataset**; zero-candidate options are disabled
  with a visible «0» (or hidden, as before, for states in Manage
  Everything); runtime-only groups (state/health/provenance/update) are
  hidden in the marketplace with a clear note when no scan exists.
  OR semantics within a group, AND between groups (tested).
- `reviewUpdates()` («Обзор обновлений») now resets ALL filter groups
  except `update_only` (previously it could silently hide tools).

### Score — transparent environment health

- New module `score.ts`: `scoreBreakdown(snapshot)` mirrors the backend
  formula (counted = applicable tools only; healthy 100 / degraded 50 /
  broken·unhealthy·missing 0; unchecked, not_applicable and
  bundled-children excluded without penalty; docker recommendation does
  not create exclusions — a missing-but-installable tool is honestly
  counted in standalone). No «required» counter exists in the breakdown.
- `EnvironmentHero` statistics now use truthful labels: **Применимые,
  Здоровы, Обновления, Проблемы, Отсутствуют, Не проверено, Диск,
  Права** («Обязательные готовы N/M» removed).
- New explanation popover next to the score ring (help button, Escape +
  focus return): formula text, per-bucket counts (incl. «Неприменимо
  здесь» and «Исключено (bundled-дети)»), and the per-tool contribution
  list of every counted tool (100/50/0).

### Detail drawer

- Added «Каталог» section: aliases and platform availability badges
  (current platform highlighted). Existing usability features (full logs,
  expandable sections, copy buttons, local vs Docker distinction,
  malformed-data guards, keyboard/focus, mobile layout) were already in
  place and are unchanged.

## Files changed (exact) — Phase 4

Frontend only (backend untouched in Phase 4):

- `src/lib/components/ui/icons.ts` — added `store` icon (additive).
- `src/lib/modules/toolchain/types.ts` — `CatalogFilters` +=
  `installable`, `has_docker_alternative`.
- `src/lib/modules/toolchain/filters.ts` — `validateCatalogFilters`,
  extended `matchesDefinitionSearch` (description/notes/docs/source),
  new predicate branches for `installable`/`has_docker_alternative`,
  defensive search against malformed definitions.
- `src/lib/modules/toolchain/marketplace.ts` (new) — items, search,
  predicate, counts, capability mirror, platform/checksum derivation.
- `src/lib/modules/toolchain/score.ts` (new) — score breakdown +
  formula text.
- `src/lib/modules/toolchain/state.svelte.ts` — full filter persistence
  with `validateCatalogFilters` on load; `tool_marketplace` mode;
  `ensureMarketplacePlatform()`; post-success targeted recheck of
  requested tools.
- `src/lib/modules/toolchain/components/MarketplaceMode.svelte` (new) —
  search + rail + grid; zero-candidate filters disabled with counts.
- `src/lib/modules/toolchain/components/MarketplaceCard.svelte` (new) —
  definition-driven card with all required fields.
- `src/lib/modules/toolchain/components/EnvironmentHero.svelte` —
  truthful stats labels + score explanation popover.
- `src/lib/modules/toolchain/components/ManageEverythingMode.svelte` —
  counts + disabled zero-candidates for every group; two new quick
  filters; typed filter option constants.
- `src/lib/modules/toolchain/components/ToolDetailDrawer.svelte` —
  «Каталог» section (aliases, platform availability).
- `src/routes/toolchain/+page.svelte` — third mode tab + pane;
  `reviewUpdates()` full reset.

NOT touched (verified via `git status`): `routes/create/+page.svelte`,
`lib/modules/project_creator/**`, project_creator backend, legacy `tc_*`
signatures/payloads, `src-tauri/**` (Phase 4 has no backend changes).

## Tests run

- `npm run test` (vitest) → **9 files, 145 tests passed** (was 7/102).
- `npm run check` (svelte-check) → **0 errors**; same 17 pre-existing
  a11y warnings (all in unrelated files).
- `cargo test` — not re-run: Phase 4 contains no Rust changes; Phase 3
  baseline (659 passed / 1 ignored) still applies. `git diff` confirms
  no backend edits in this phase.

New frontend tests:

| File | Guards |
|---|---|
| `marketplace.test.ts` (23 cases) | definition-only search over display/id/aliases/category/description/notes/docs/source; no-scan marketplace; per-OS installability; availability union; checksum honesty; docker-as-metadata; **removed tools absent from the REAL tools.json** and from items; OR-within/AND-across filter semantics; state/health/provenance/update are runtime filters; counts match the dataset; predicate covers every filter group |
| `score.test.ts` (11 cases) | health score, not required-set: proportional missing penalty, half-credit degraded, unchecked/unsupported/manual/docker/bundled never penalize, docker-recommendation metadata doesn't exclude host requirement, no «required» field, total classification over all 13 states |
| `filters.test.ts` (+12 cases) | extended definition search fields; `validateCatalogFilters` discards junk/duplicates/unknown kinds and keeps valid payloads; `installable`/`has_docker_alternative` predicates |

## Failures

None outstanding. Intermediate failures during development were test
fixture issues (xcode fixture inherited default sources; android scan
fixture carried default installable capabilities; vitest `toBe` message
argument; `node:fs`/`process` typing — replaced with a typed JSON import
of the real `tools.json`) and one real defensive gap found by tests:
`matchesDefinitionSearch` crashed on definitions missing optional fields
(fixed with `?? ""`).

## Decisions made

- Marketplace platform without a snapshot: `liveSnapshot.os` first,
  then the light read-only legacy `tc_get_environment_info` — read-only,
  byte-compatible, no Project Creator integration touched.
- Filter counts: computed per render from the CURRENT dataset (items or
  snapshot tools), never static; runtime-only groups hide entirely when
  no scan exists (dead filters never rendered).
- Score: backend `ScoreSummary` stays the authoritative ring value; the
  frontend `scoreBreakdown` recomputes the identical formula for the
  transparent explanation (tested against the same rule, not duplicated
  payloads).
- Docker stays metadata-only everywhere in standalone (marketplace badge,
  plan review recommendation); `execution:"docker"` remains unproducible
  and rejected by the planner (Phase 3 invariant, unchanged).

## Notes for next session

- Phase 3 baseline (backend tests) remains valid; run `cargo test` after
  any future backend change.
- The marketplace uses `tc_get_environment_info` only as an OS fallback
  (no snapshot); if a canonical OS-only command is added later, swap it
  in `ensureMarketplacePlatform` — the rest of the flow is independent.
- `MarketplaceMode`/`ManageEverythingMode` share the same rail pattern;
  keep counts and disabled-zero behavior when adding future filters.

## Files changed (exact)

Backend (`src-tauri/src/modules/toolchain/**`):

- `tools.json`
  - **Removed `unity`, `unreal`, `godot`** from the standalone catalog.
    They no longer appear in catalog definitions returned to standalone
    UI (`tcx_get_catalog`), standalone scan results, filters, score,
    marketplace inputs, planner, cards or drawer.
  - **Swift**: official Windows distribution added — winget
    `Swift.Toolchain` + Official Burn installer
    (`https://download.swift.org/swift-6.3.3-release/windows10/swift-6.3.3-RELEASE/swift-6.3.3-RELEASE-windows10.exe`,
    silent `-quiet`, user scope/no UAC) with **explicit SHA-256 integrity**
    taken from the official Swift-maintained winget manifest
    (`23562654…ede94b7`). URL verified live (HTTP 200); nothing invented.
    `manual_install` removed; recommended version → 6.3; known_paths gained
    `%LOCALAPPDATA%/Swift`; notes name Git/VCRedist prerequisites.
  - **Xcode** (`xcodebuild`): declares `platform_availability: ["macos"]`.
    On Windows/Linux it now classifies as *UnsupportedPlatform* («не
    поддерживается здесь»), never «manual install», and can never be
    auto-installed anywhere (no sources on any OS).
  - Docker-related tools (`postgresql`, `redis`, `mongodb`, `kafka`,
    `grafana`, `mysql`) keep their real local sources (MySQL CDN MSI URL
    re-verified live) and each declares a factual **docker alternative as
    metadata only** (`extended.docker`: image + note). MySQL remains fully
    locally installable — it must not become DockerManaged/Docker-no-op in
    standalone mode, and it does not anymore.
- `legacy_compat_tools.json` (new): the three removed engine definitions
  verbatim, preserved **exclusively** for Project Creator legacy data
  (wizard_tree frameworks unity/unreal/godot must still produce honest
  manual-install requirements instead of silently disappearing).
- `defs.rs`
  - `load_legacy_definitions()` + `load_merged_definitions()` (asserts no
    id collisions between catalogs). Tests: legacy engines absent from
    standalone catalog / present only in legacy catalog / swift source
    carries explicit sha256 / xcode macos-only availability / docker-
    related tools locally installable with docker metadata.
- `mod.rs`
  - `ToolchainState` splits `definitions` (standalone) vs
    `legacy_definitions` (+ `merged_definitions()`,
    `get_definition_merged()`). `environment_info` (legacy surface) counts
    over the merged catalog — behavior preserved byte-for-byte.
- `commands.rs`
  - New **`tcx_get_catalog`**: standalone-only catalog command; frontend
    switched to it. Legacy `tc_get_tool_definitions` unchanged (merged set).
  - Legacy surfaces (`tc_check_environment`, `tc_build_install_plan`,
    `tc_get_health_report`) run over `merged_definitions()` so Project
    Creator keeps resolving its own legacy data.
  - **Standalone scan de-dockerized**: `domain_scan_context` passes an
    EMPTY `dual_tools` set — PostgreSQL/Redis/MongoDB/Kafka/Grafana/MySQL
    scan as ordinary local tools; no `DockerManaged` states, no Docker
    provenance, no `DockerDefault` applicability in standalone snapshots.
  - **Planner termination**: `build_canonical` wrapped in a finite
    `PLAN_BUILD_TIMEOUT` (45 s) producing structured `PlanError::
    PlannerTimeout { seconds }` with actionable text — plan building always
    terminates (success | structured error | timeout).
- `domain/engine.rs` — `run_scan_job` no longer derives dual tools via
  `is_dual_tool`; standalone scans are docker-classification-free.
- `domain/detect.rs`
  - `classify_applicability`: **declared `platform_availability` excludes
    an OS before any other rule** (rule order documented). Xcode on Windows
    → UnsupportedOnPlatform; on macOS → ManualOnly. Regression tests:
    `declared_platform_availability_excludes_os_before_manual`.
- `engine/request.rs` — new optional `expected_plan_fingerprint`
  (deny_unknown_fields preserved): the fingerprint of the approved preview;
  safe scalar choice, URLs/paths/args remain unexpressible.
- `engine/planner.rs`
  - **Docker no-op branch removed**: dual tools no longer produce
    `NoOp(DockerManaged)` tasks. Every tool installs on host when the
    current platform has a catalog source; explicit
    `execution: "docker"` is rejected (`NotInstallable`) instead of being
    silently converted to host. `NoopReason::DockerManaged` remains in the
    enum only so old persisted plans keep deserializing.
  - **Stale-plan guard**: if the request carries
    `expected_plan_fingerprint` and the freshly rebuilt plan differs →
    structured `PlanError::PlanChanged { expected, actual }` («окружение
    изменилось… пересмотрите план»). Preview and execution both rebuild
    server-side; a changed plan is NEVER silently executed.
  - New `PlanError::PlannerTimeout` variant.
  - Tests: mysql host-install (never docker noop), postgresql truthful
    noop + redis local install, explicit docker choice rejected, update
    task only when actually available, broken-path reinstall-with-warning
    vs separate RepairPath, deterministic dependency closure + stable
    fingerprint, planner timeout finite, stale preview fingerprint aborts,
    single-tool plan probes ≤ requested+dependency hosts (never full
    catalog).
- `core/requirements.rs`
  - `resolve()` refactored over `resolve_inner`; behavior byte-compatible.
  - New **`resolve_standalone()`**: wizard docker tools are local
    requirements without `local_infra_tools` opt-in; pure-docker tools
    (clickhouse/airflow/mailpit) still excluded by mapping. PC's
    `docker_optional_requirements` semantics NOT reused in standalone.
  - Tests: standalone resolves all six infra tools locally without
    opt-in (deterministic order), pure-docker still skipped, legacy
    `resolve()` gating unchanged.
  - `every_resolved_id_exists_in_tools_json` now validates against the
    MERGED catalog (standalone + legacy compat file).
- `domain/profile.rs`
  - Build Environment profile uses `resolve_standalone`; docker groups
    (`optional`, `docker_managed`) stay empty by construction — Docker is
    only a recommendation/metadata capability
    (`docker_alternative_available`). Fields remain serialized for old
    caches. Tests rewritten: postgresql/redis required without opt-in;
    docker groups empty; local_alternatives echo preserved; unity resolves
    through merged catalog into manual (legacy), godot unknown in
    standalone (`unknown_tool` warning); android replaces unity as the
    standalone manual fixture.

Frontend (`src/lib/modules/toolchain/**`):

- `types.ts` — `EngineRequest` mirror gains optional
  `expected_plan_fingerprint`.
- `api.ts` — `getCatalog()` → `invoke("tcx_get_catalog")`;
  `mutationRequest` accepts/passes `expected_plan_fingerprint`.
- `components/PlanReviewModal.svelte` (rewritten flow)
  - Distinct plan states instead of indefinite «Строим план…»:
    **validating → refreshing facts (only if snapshot stale) → preparing
    plan → ready | failed**, each with its own label; failed shows the
    structured backend error with retry.
  - **Snapshot-first planner flow**: opening a plan never starts a full
    48-tool scan. Latest valid snapshot is used as-is while fresh; only a
    stale snapshot triggers a TARGETED recheck of just the requested tools
    (`tcx_run_health_checks`, bounded parallelism).
  - Start sends `expected_plan_fingerprint` from the approved preview;
    backend rejects drift with PlanChanged (rendered as failed state).
  - Task list truthfulness: per-task no-op REASON always visible
    (total formatters); local infra tools explicitly labeled
    «локальная установка на этой машине»; Docker shown separately as a
    recommendation line (`image + notes` from catalog metadata) — never as
    a task mode or action.
  - Malformed actions/tasks render «Неизвестное действие» (guards reused);
    summary counts actionable vs no-op honestly.
  - On start success the operations panel opens immediately — progress is
    visible right away.
- `planUi.test.ts` (new, 8 cases): mysql/postgresql local install labels,
  installed no-op truthfulness, update-only-with-target, path-repair vs
  reinstall distinction, malformed noop reasons, malformed task actions
  (never crash), unknown-ID error surfacing.

Docs: this file. NOT touched (verified via `git status`):
`routes/create/+page.svelte`, `lib/modules/project_creator/**`,
project_creator backend, legacy `tc_*` signatures/payloads, engines
deletion, marketplace, UI redesign beyond truthfulness.

## Decisions made

- Engine removal strategy: definitions moved OUT of tools.json INTO a
  dedicated `legacy_compat_tools.json` consumed only by legacy tc_*
  commands. This satisfies both constraints at once — standalone surfaces
  physically cannot see the engines (catalog/scan/planner read
  `state.definitions()` alone, so their IDs fail validation as unknown),
  while Project Creator's own wizard_tree data keeps resolving them.
- Swift kept installable rather than manual-only: the official Burn
  installer exists, is verified live, supports `-quiet`, installs per-user,
  and has an authoritative SHA-256 (official winget manifest maintained by
  the Swift release manager). Integrity is therefore explicit, satisfying
  «source integrity must be explicit» without inventing anything.
- Xcode uses the existing-but-unused `platform_availability` declaration
  instead of new code paths: one classification rule, data-driven.
- Standalone de-dockerization touches only consumers (scan ctx, planner,
  profile), not `is_dual_tool` itself — Project Creator's Docker behavior
  (optional_requirements, docker-compose default) stays intact.
- Fingerprint guard lives inside `planner::build_plan` (the single plan
  authority) so preview, execution and retry all share it; the commands
  layer adds only the finite timeout wrapper.
- The modal's staged states reflect real work (snapshot read → targeted
  recheck → server-side build); they never fabricate progress data.

## Tests run

- `cargo fmt --check` → PASS.
- `cargo test` (workspace lib) → **659 passed; 0 failed; 1 ignored**
  (pre-existing ignore).
- `npm run test` (vitest) → **7 files, 102 tests passed**.
- `npm run check` (svelte-check/tsconfig) → **0 errors**; same 17
  pre-existing a11y warnings.

New Rust tests:

| Test | Guards |
|---|---|
| `defs::tests::legacy_engines_are_absent_from_standalone_catalog` | godot/unreal/unity cannot leak into standalone |
| `defs::tests::legacy_engines_live_only_in_legacy_catalog` | compat preserved exactly where PC needs it; no id collisions |
| `defs::tests::swift_windows_official_source_carries_explicit_integrity` | real download.swift.org source + 64-hex sha256; not manual-only |
| `defs::tests::xcode_declares_macos_only_availability` | availability declared, drives classification |
| `defs::tests::docker_related_tools_are_locally_installable_with_docker_metadata` | six infra tools installable on Windows; docker = metadata |
| `detect::tests::declared_platform_availability_excludes_os_before_manual` | Xcode: unsupported on Windows, manual on macOS |
| `requirements::tests::standalone_resolution_installs_docker_tools_locally_without_opt_in` | resolve_standalone deterministic, no opt-in gate |
| `requirements::tests::standalone_resolution_still_skips_pure_docker_tools` | clickhouse/airflow/mailpit never local |
| `requirements::tests::legacy_resolve_keeps_opt_in_gating_for_wizard` | PC `resolve()` byte-compatible |
| `profile::tests::postgresql_redis_are_required_locally_without_opt_in` | profile standalone grouping |
| `profile::tests::standalone_docker_tool_is_required_and_docker_groups_stay_empty` | docker groups empty; alternative = metadata |
| `profile::tests::local_alternatives_echo_preserved_in_profile` | user choice echoed |
| `profile::tests::legacy_catalog_manual_engine_is_classified_as_manual` | unity via merged catalog → manual (PC) |
| `profile::tests::standalone_catalog_does_not_know_legacy_engines` | unknown_tool warning; never required |
| `planner::tests::mysql_missing_produces_host_install_task_never_docker_noop` | MySQL local install (with and without explicit host choice) |
| `planner::tests::postgresql_installed_is_truthful_noop_missing_installs_locally` | installed → AlreadyInstalled noop; missing → host install |
| `planner::tests::explicit_docker_execution_choice_is_rejected` | no silent docker→host conversion |
| `planner::tests::update_task_only_when_update_actually_available` | update vs UpdateUnavailable noop |
| `planner::tests::broken_path_is_reinstall_with_warning_not_silent_repair` | reinstall≠repair; RepairPath downloads nothing |
| `planner::tests::dependency_resolution_order_is_deterministic` | stable order + stable fingerprint |
| `planner::tests::planner_timeout_is_finite_and_structured` | PlannerTimeout message |
| `planner::tests::stale_preview_fingerprint_aborts_execution` | PlanChanged on drift; matching passes |
| `planner::tests::single_tool_plan_does_not_scan_full_catalog` | detector called only for requested + dep hosts |

New frontend tests (`planUi.test.ts`, 8 cases): see Files changed above.

## Failures

None outstanding. Intermediate fixes during development: two type errors
after splitting catalogs (`&Vec` borrow, owned-vs-ref lookup), three test
updates where fixtures referenced the removed engine ids (replaced with
android/merged-catalog equivalents) and the terraform fixture needed
`confirm_unverified_sources` (real zip ships no digest by design).

## Remaining work (explicitly out of scope)

- Marketplace: not implemented (per instructions).
- UI redesign beyond truthfulness: not started (per instructions).
- Engines deletion from Project Creator knowledge base: not done (legacy
  compatibility requires them).
- Optional future hardening: non-Windows probe-test equivalents;
  auto-refresh of pinned Swift installer URL/sha256 on new releases
  (currently pinned 6.3.3 with live-verified values).

## Notes for next session

- Contract §2 pipeline order unchanged; every backend addition was either
  additive with serde defaults (`expected_plan_fingerprint`,
  `platform_availability` consumers) or confined to standalone-only
  consumers (scan ctx, tcx planner, profile) — old caches/snapshots and
  all legacy payloads deserialize cleanly.
- `NoopReason::DockerManaged` and `ExecutionMode::Docker` survive ONLY for
  deserialization of old persisted jobs; the planner can no longer produce
  them, and `execution:"docker"` requests are rejected.
- When Swift releases a new version, update BOTH url/file_name and sha256
  in tools.json (values must come from the official winget manifest or
  swift.org — never guessed); `swift_windows_official_source_carries_
  explicit_integrity` guards the shape but not the currency of the pin.
