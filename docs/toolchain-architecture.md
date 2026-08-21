# Toolchain Control Center — Architecture & Migration

Status: **final v2** (acceptance stage — inventory updated to the shipped state;
§5 issues annotated with their resolution; §6 test inventory reflects reality).
Companion files: `docs/toolchain-contract.md` (normative contract),
`docs/toolchain-progress.md` (session log incl. final report),
`docs/toolchain-acceptance.md` (manual acceptance checklist).

Everything in §1–§4 was **verified by reading the sources** in this repository
(2026-08-21, re-verified 2026-08-22 at the acceptance stage). Line references are
approximate anchors, not guarantees.

---

## 0. Shipped architecture (what actually exists now)

Backend module `src-tauri/src/modules/toolchain/`:

```
mod.rs                 ToolchainState: definitions, legacy install_session slot,
                       abort flag, metadata store, jobs journal, SecretStore,
                       pending one-shot secrets, ScanEngine (domain/), JobEngine (engine/);
                       startup: duplicate-id assert, plaintext-secrets migration into
                       SecretStore, Running→Interrupted recovery for all three journals
models.rs              legacy wire models + flattened ToolExtendedMetadata + metadata View (no secrets)
defs.rs / tools.json   versioned catalog (48 tools), include_str!, duplicate-id validation
commands.rs            26 registered commands = 11 legacy tc_* adapters + 15 tcx_*
core/                  discovery, requirements (wizard_tree-driven), check (read-only),
                       planner::canonicalize_plan, installer (integrity gates, traversal-safe
                       prevalidation, MAUI workload ONLY inside approved installs), console
                       (download sha256 gate, redacting sink), path_service, metadata,
                       health, qt_installer, disk, archive (validate_entry_name +
                       extract_zip_safe reference implementation), crypto (CSPRNG passwords,
                       sha256 helpers), secrets (DPAPI store), jobs (legacy journal)
domain/                read-only scan engine: models (13-state ToolState, kind-tagged),
                       probe (sanitized, capped, timeout), detect (multi-install, provenance,
                       capabilities, health v2), path_report (read-only PATH diagnostics),
                       score, cache (honest stale flags), engine (scan job lifecycle,
                       bounded concurrency 4, 90 s deadline → honest Partial)
engine/                canonical mutation pipeline: request (deny_unknown_fields restricted
                       model), plan (CanonicalPlan, typed phases/statuses), planner
                       (backend-authoritative plan builder), jobs (persist-before-execute,
                       bounded ring 20, restart recovery), exec (cancellation kills current
                       task, terminal math, out-of-band secrets), events (identity+seq)
platforms/             windows/linux/macos adapters + unix_rc managed PATH block
```

Frontend module `src/lib/modules/toolchain/`:

```
types.ts       legacy mirrors + canonical kind-tagged unions + engine external-tagged mirrors
api.ts         typed wrappers for every registered command + thin listeners
events.ts      ref-counted event channels (1 physical listener per event name),
               identity filters, TerminalGuard, trackScan/trackJob
stateLogic.ts  pure transition rules (applyScanProgress, mergeLiveSnapshot, gateMutation,
               applyJobEvent seq guard, isJobSucceeded, upsertJobHistory, identityMatches)
state.svelte.ts Svelte 5 runes controller singleton (cache-first snapshot, reconnect,
               adoption set, debounced prefs persistence, bounded per-job logs)
format.ts      human labels/sanitizers (incl. scanPhaseLabel/scanTerminalLabel)
filters.ts     pure catalog predicates/sorting
compat.ts      the single Project Creator import surface
components/    ScoreRing, StateBadge, EnvironmentHero, ActivityStrip, JobCenter,
               ToolCard, ToolDetailDrawer (focus restore, adopt action), RequirementPicker,
               ProfileResult, BuildEnvironmentMode, ManageEverythingMode, PlanReviewModal
routes:        /toolchain (canonical page) · /environment (+page.ts redirect 308)
tests:         vitest suites for types/events/stateLogic/filters/format (75 tests)
```

Registered commands (`lib.rs`, exact list): `ping_toolchain`,
`tc_get_tool_definitions`, `tc_get_environment_info`, `tc_check_environment`,
`tc_build_install_plan`, `tc_run_install`, `tc_get_install_status`,
`tc_abort_install`, `tc_take_new_secrets`, `tc_get_metadata`,
`tc_get_health_report`; `tcx_get_environment_snapshot`, `tcx_start_scan`,
`tcx_get_scan_job`, `tcx_get_latest_scan_job`, `tcx_cancel_scan`,
`tcx_get_tool_details`, `tcx_run_health_checks`, `tcx_profile_resolve`,
`tcx_build_plan`, `tcx_start_job`, `tcx_get_job`, `tcx_list_jobs`,
`tcx_cancel_job`, `tcx_retry_job`, `tcx_adopt_tool`.

Events: legacy `toolchain:check_progress` / `toolchain:task_event` /
`toolchain:install_done` (wizard compat) and new `toolchainx:scan_progress` /
`toolchainx:scan_done` / `toolchainx:job_event` (identity-bearing).

Persistence under `<app_data>/toolchain/`: `state.json` (**secrets stripped on
every save**, sanitized View for reads), `secrets.bin` (DPAPI-encrypted on
Windows, 0600 on Unix), `jobs.json` (legacy journal), `jobs/*.json` (engine
records, bounded ring), `scan-journal.json`, `scan-snapshot.json`.

---

## 1. Verified as-is inventory (backend)

### 1.1 Files (actual paths)

```
src-tauri/src/modules/toolchain/
├── mod.rs                 ToolchainState: definitions, install_session
│                          (Arc<Mutex<Option<InstallSession>>>), abort_install
│                          (Arc<AtomicBool>), metadata (MetadataStore),
│                          pending_secrets (one-shot secrets display)
├── models.rs              ALL data models (serde; snake_case JSON)
├── defs.rs                tools.json loader (include_str!) + validate()
├── tools.json             48 tool definitions (compiled into binary)
├── commands.rs            11 Tauri commands (tc_* / ping_toolchain)
├── core/
│   ├── mod.rs             service map; rule: no cfg!(target_os) in core
│   ├── version.rs         parse_version / compare / meets_min
│   ├── discovery.rs       detection: version probes → known paths → registry;
│                          PROBE_TIMEOUT_SECS = 10; glob_first(); installed_path()
│   ├── requirements.rs    ProjectRequirements → tool id list; reads wizard_tree.json
│   │                      (required_tools, languages, requires_docker); dual-tool
│   │                      logic; qt UI options → install_options
│   ├── check.rs           run_check(): parallel detect, CHECK_DEADLINE=90s partial
│   │                      results, ManualInstall normalization, disk space,
│   │                      ensure_maui_workload() ⚠ (mutates machine during check)
│   ├── planner.rs         build_plan(check, selected): skips Installed/Manual/
│                          RunInDocker; winget forced first; task_id == tool_id
│   ├── installer.rs       execute_plan/run_task: source fallback chains;
│                          resolve_execution (GitClone/Phar/Script/Archive/Exe);
│                          msiexec/msix/Appx/Expand-Archive+tar scripts; UAC via
│                          run_elevated; PATH phase; verify by re-detection;
│                          tool-specific post-steps (MAUI workload, php.ini,
│                          composer.bat shim); generate_db_password()
│   ├── console.rs         EventSink trait; piped_run (streaming, abort-kill);
│                          run_elevated (Start-Process -Verb RunAs, file logs);
│                          download (PowerShell HttpClient chunked, tc:dl lines,
│                          DOWNLOAD_TIMEOUT=30min); ps_quote
│   ├── path_service.rs    merge_dirs, expand_env_vars, add_to_user_path,
│                          sync_process_path (process env update)
│   ├── metadata.rs        MetadataStore: app_data_dir/toolchain/state.json,
│                          atomic write (tmp+rename), broken file → default
│   ├── health.rs          check_tool/run_health_report: serial per tool; ok only if
│                          all checks pass; score over tools that have checks
│   ├── qt_installer.rs    QtOnline pipeline: repo listing → Updates.xml (naive XML
│                          parsing) → per-package download (curl.exe) → tar extract;
│                          Windows msvc2022_64 only; hardcoded Qt 6.8 branch ("68")
│   └── disk.rs            install_root(), free_space_mb facade, DriveInfo/df parsers
└── platforms/
    ├── mod.rs             PlatformAdapter trait (+async_trait); current_platform()
    │                      OnceLock; resolve_command (.cmd/.bat handling)
    ├── windows.rs         HKCU/HKLM PATH via [Environment]; Win32_OperatingSystem;
    │                      DriveInfo free space; package managers [winget, choco]
    ├── linux.rs           apt/dnf/pacman list; rc-file PATH; df -Pk
    ├── macos.rs           brew list; same rc-file approach (mirror of linux.rs)
    └── unix_rc.rs         ~/.bashrc|~/.zshrc|~/.profile "# StackPilot:begin/end"
                           block read/write; strips " $ `
```

### 1.2 Registered Tauri commands (src-tauri/src/lib.rs)

`ping_toolchain`, `tc_get_tool_definitions`, `tc_get_environment_info`,
`tc_check_environment`, `tc_build_install_plan`, `tc_run_install`,
`tc_get_install_status`, `tc_abort_install`, `tc_take_new_secrets`,
`tc_get_metadata`, `tc_get_health_report`.

State registered: `ToolchainState` (`app.manage`, lib.rs). On startup a background
task calls `path_service::sync_process_path()`.

### 1.3 Events emitted by the backend (verified emit sites)

| Event | Emit site | Payload |
|---|---|---|
| `toolchain:check_progress` | commands.rs (tc_check_environment) | `CheckProgressEvent {done,total,tool_id,display,icon,status}` |
| `toolchain:task_event` | commands.rs `AppEventSink` | `ToolchainEvent {event_type, task_index, total_tasks, task_id, tool_id, timestamp}` |
| `toolchain:install_done` | commands.rs (after execute_plan) | final `InstallPlan` |

Related non-toolchain events consumed elsewhere: `process-status` (workspace, listened
in lib.rs), `process-output` (workspace), `project_creator:step_event`,
`project_creator:execution_done` (project_creator/commands.rs).

### 1.4 Models (models.rs; TS mirrors in src/lib/modules/toolchain/types.ts)

- **Catalog**: `ToolDefinition` (id, category, display, description, icon,
  `DetectionRules{version_probes,known_paths,registry_keys}`, `VersionRules{min,recommended}`,
  `InstallSources{windows,linux,macos}` of `InstallSource{kind,id,url,file_name,args,
  extra_args,dynamic_args,install_dir,needs_admin,execution}`, size_mb, needs_admin,
  path_entries, bundled_with, health_checks, notes, manual_install).
  `InstallSourceKind`: PkgManager | Official | Script | QtOnline.
  `ExecutionKind`: GitClone | Exe | Script | Phar | Archive | Auto.
- **Runtime**: `ToolStatus` = Missing | Installed{version} | UpdateAvailable{installed,recommended}
  | PathBroken{reason} | ManualInstall{reason} | RunInDocker.
  `ProjectRequirements` {languages, frameworks, tools, local_infra_tools, git_init,
  vscode_config, docker}. `ToolRequirement`. `EnvironmentCheck` {os, requirements,
  optional_requirements, total_size_mb, free_space_mb, enough_space, needs_admin_any,
  all_ready}. `EnvironmentInfo`.
- **Plan/job**: `InstallTask`, `TaskState` = Pending | Running{phase} | Success{version}
  | Failed{error} | Skipped{reason}; `TaskPhase` = Downloading | Installing | Verifying |
  UpdatingPath; `InstallPlan`; `InstallSession`; `ToolchainEvent(+Type)`; `CheckProgressEvent`.
- **Health**: `HealthCheckResult`, `ToolHealth`, `HealthReport{tools,score,scanned_at}`.
- **Persistence**: `ToolchainMetadata{last_scan, tools:Map<String,InstalledToolInfo>,
  secrets:Map, prefs:Map}`, `InstalledToolInfo{path,version,installed_at,path_entries}`.

Serde format for enums is default (externally tagged): unit variants are strings
(`"Missing"`), data variants objects (`{"Installed":{"version":...}}`) — the frontend
mirrors this exactly.

### 1.5 Catalog contents (tools.json ids, 48)

winget, node, npm, python, pip, rust, rustc, cargo, tauri-cli, go, java, kotlin, kafka,
grafana, terraform, firebase, dotnet, csharprepl, msvc-build-tools, zig, dart, php,
composer, swift, erlang, elixir, gleam, flutter, maven, gradle, postgresql, redis,
mongodb, mysql, sqlite, docker, git, vscode, cmake, make, curl, tar, unity, unreal,
godot, android, qt, xcodebuild.

Categories today: language/package_manager/compiler/database/container/vcs/editor/utility.
Dual docker tools (requires_docker in wizard_tree AND local sources): postgresql,
redis, mongodb, kafka, grafana, mysql. Pure docker-only wizard tools (no local
sources, unmapped): clickhouse, airflow, mailpit. Manual-install engines/SDKs:
unity, unreal, godot, qt, xcodebuild (+ flutter installs via git clone).

### 1.6 State persistence (verified)

- Backend: `<app_data_dir>/toolchain/state.json` (lib.rs `data_dir.join("toolchain")`;
  MetadataStore atomic tmp+rename). Contains installed tools, **plaintext secrets**,
  prefs, last_scan timestamp. Written after successful installs and after every
  environment check (`touch_last_scan`). App settings live separately in
  `<app_data_dir>/settings.json` (core/settings.rs).
- Frontend: only `sessionStorage["stackpilot:create:session:v1"]`
  (project_creator/createSession.ts) — whitelisted light fields of the create session.
  No localStorage anywhere in src/.

---

## 2. Current Project Creator integration (verified; do not break)

Backend:

- `requirements.rs` reads `project_creator/knowledge/wizard_tree.json` (include_str!)
  for `frameworks[].required_tools`, `frameworks[].languages` (requires-language
  inference), `tools[].requires_docker`. Toolchain stays code-independent from
  project_creator; coupling is via this data file only.
- `WizardContext.local_infra_tools` (project_creator/models.rs) drives compose-file
  exclusion and LOCAL_INFRA.md generation (engine/readme.rs, engine/mod.rs).

Frontend (single integration point): `src/routes/create/+page.svelte` (~3700 lines),
wizard phase 5 “Environment”:

- `buildRequirements()` builds `ProjectRequirements` from wizard state.
- `goToEnvironment()` → subscribes `listenCheckProgress`, refreshes installed set from
  `tc_get_metadata`, runs `tc_check_environment`.
- Selection UI: checkboxes over non-ok/non-manual requirements; optional section lists
  docker tools with “Run in Docker / Use Host Machine” toggle (`optInLocalInfra` /
  `revertLocalInfra` re-run the check).
- `startInstall()`: `tc_build_install_plan(envCheck, selectedIds)` then listeners +
  `tc_run_install(plan)`; streams `toolchain:task_event` into per-task state maps,
  parses `tc:dl` progress lines, collects `tc:error` lines; `handleInstallDone`
  applies final plan, refreshes metadata, fetches one-time secrets
  (`getNewSecrets` → copy-to-clipboard UI).
- `cancelInstall()` → `tc_abort_install`. Listeners unregistered in onDestroy.
- Restart hint after install (“restart terminals to pick up PATH”).

The standalone Toolchain route consumes only `tc_get_environment_info`,
`tc_get_health_report`, `tc_get_metadata` (read-only page).

---

## 3. Current compatibility constraints & platform limitations

### 3.1 Platform

- Installs are **Windows-first**: `run_task` returns `Skipped{"Установка на этой ОС
  появится позже"}` for non-Windows; `run_elevated` errors off-Windows; Qt pipeline is
  windows/msvc2022_64-only; PkgManager command builder hardcodes program `"winget"`
  regardless of OS (unreachable off-Windows because tasks skip first).
- Detection works cross-platform (probes/paths; registry checks effectively
  Windows-only via reg.exe failing elsewhere).
- PATH persistence: Windows `[Environment]::SetEnvironmentVariable('Path',...,'User')`
  (whole-value replace after merge); Unix: managed block in one rc file chosen at
  first touch ($SHELL preference), values stripped of `` " $ ``.
- Linux/macOS package manager *sources exist* in tools.json but are never executed
  today (skipped before reaching the runner).
- Downloads run through PowerShell 5.1 scripts (`System.Net.Http.HttpClient`);
  Qt listing/HEAD requests use curl.exe (mirror redirect handling). Both are
  Windows-oriented choices.

### 3.2 Behavioral constraints to preserve

- Probe timeout 10 s/tool-probe; whole-check deadline 90 s producing *partial*
  reports (dropped tools are silently absent from requirements).
- `resolve()` always prepends `"winget"` even on non-Windows.
- Missing+bundled tools (npm→node etc.) are dropped from requirements when missing.
- Manual-install tools never enter plans; they don't count into size/admin sums.
- Planner forces winget first; `selected` cannot resurrect Installed/Manual/Docker rows.
- Installer fallback chain: sources tried in order, intermediate failures logged only
  under `DEVLAUNCHER_DEBUG=1`; single aggregated `tc:error` when all fail.
- winget codes `-1978335189/-1978335193` treated as already-installed → go to verify.
- Verify step = re-run detection; success requires `Installed{..}`.
- One install session slot; second concurrent `tc_run_install` rejected with error.
- Abort flag reset at each `tc_run_install` start; running task killed mid-stream,
  remaining marked Skipped.

---

## 4. Migration matrix

Legend — Strategy: **Keep** (reuse as-is), **Adapt** (extend/refactor in place),
**Wrap** (new API over existing core), **Replace** (new implementation), **Move**
(relocate responsibility). Verification column names the concrete check to run.

| Current component | Target responsibility | Strategy | Compatibility requirements | Verification |
|---|---|---|---|---|
| `tools.json` + `defs.rs` loader/validate | Versioned catalog incl. checksums, execution-mode tags, deprecations | Adapt | include_str! loading stays; duplicate-id assert stays; additive fields with serde defaults | cargo test `defs`/catalog tests; catalog validation test |
| `models.rs::ToolStatus` (6 variants) | Dimension model (§2 of contract) + derived presentation status | Replace (new types alongside old) | Old enum keeps serializing for legacy commands until consumers migrate | Serde roundtrip tests old↔new mapping |
| `discovery.rs` | Read-only scan engine feeding dimension model; adds scan-pending/failed outcomes | Adapt | probe order, timeouts, glob behavior unchanged; pure functions keep signatures | Existing discovery tests + new scan-failure-path tests |
| `check.rs::run_check` | Profile resolution + read-only diagnostic scan (no machine mutation) | Adapt | Same output shape for legacy `tc_check_environment`; MAUI workload moved OUT to explicit job | Existing check tests; new test asserting no side-effect calls during scan |
| `ensure_maui_workload` (in check.rs) | Explicit user-approved job step | Move | Only runs from approved plan/job, never from scans | Code-path audit + test that scan path never calls it |
| `requirements.rs` (wizard_tree-driven) | Canonical profile resolver for both modes | Keep/Adapt | wizard_tree.json contract unchanged; Project Creator payloads unchanged | All ~25 existing requirement tests must stay green |
| `planner.rs::build_plan` | Backend-canonical plan builder (+ fingerprint/re-validation) | Adapt | Legacy signature kept as wrapper; winget-first preserved | Existing planner tests green; new tampered-plan rejection tests |
| `installer.rs::execute_plan/run_task` | Job executor under unified job registry | Wrap | Event stream shape compatible (or dual-emitted); abort semantics unchanged | installer tests green; manual smoke install of echo-def |
| `console.rs` (piped_run/run_elevated/download) | Process/download layer + integrity hooks | Adapt | tc:* line protocol preserved during migration; add checksum verification gate | console tests green; new checksum-failure test |
| Archive extraction scripts (Expand-Archive/tar in installer.rs, qt_installer.rs) | Traversal-safe unpacking service | Replace | Behavior for well-formed archives unchanged | New zip-slip fixture test (malicious archive rejected) |
| `generate_db_password` (nanosecond hex) | CSPRNG secret generator | Replace | Output still delivered via take-once channel only | Unit test length/uniqueness; audit no logging |
| `metadata.rs` MetadataStore/state.json | Versioned store + provenance + snapshot + PATH audit + job history; secret isolation | Adapt | Existing files load without migration failure (defaults); secrets excluded from new read APIs | metadata roundtrip tests + new backward-compat load test (old file) |
| `health.rs` report | Health dimension provider; parallelizable, cacheable | Adapt | Report shape for legacy `tc_get_health_report` unchanged | Existing health tests green |
| `qt_installer.rs` | Qt pipeline as a job; robust manifest parsing | Adapt (later) | Windows behavior unchanged; explicit unsupported elsewhere | Existing parsing tests; manual smoke optional |
| `commands.rs` tc_* (11 cmds) | Legacy façade over new core; new tcx_* commands added | Wrap | Signatures/payloads byte-compatible until removal stage | svelte-check + create-page flow regression (manual) + cargo test |
| Single install_session slot | Job registry (multi-job, history, recovery) | Replace | `tc_get_install_status` reads primary install job for compat | New registry unit tests; restart-recovery test |
| Events `toolchain:*` (3) | New `toolchainx:*` events; legacy kept during migration | Wrap | Create page keeps working untouched | Listener smoke via create flow |
| `/environment` standalone page | Control Center overview (canonical `/toolchain`) | Replace (new route) | `/environment` alias keeps resolving; nav match already covers both | svelte-check; manual nav check both URLs |
| `create/+page.svelte` env step (phase 5) | Consumer of same domain APIs (no fork) | Wrap | No behavioral change to wizard UX in this refactor | Manual wizard pass + cargo requirement tests |
| UI kit (`src/lib/components/ui/*`) | Styling foundation for control center | Keep | Use tokens; dark/light parity | svelte-check; visual pass both themes |

---

## 5. Known bugs, stubs, security issues, architectural risks (found during inspection)

### 5.0 Resolution status (acceptance stage, 2026-08-22)

| # | Issue | Status |
|---|---|---|
| 1 | Scan mutates machine (`ensure_maui_workload` in check) | **FIXED** — workload install lives only in installer.rs (approved jobs); check.rs is read-only (guarded by engine tests) |
| 2 | Secrets in `tc_get_metadata` / status payloads | **FIXED** — sanitized View + `#[serde(skip)]`; take-once channel only; DPAPI SecretStore |
| 3 | No download integrity | **FIXED** — sha256 gate in console::download; digest-less sources require explicit confirmation and are surfaced as warnings in plan review |
| 4 | Archive traversal | **FIXED** — archive.rs prevalidation (fail-closed) wired into installer + qt paths; `validate_entry_name` rejects absolute/UNC/drive/`..`/NUL entries |
| 7 | Weak DB password | **FIXED** — CSPRNG via crypto.rs (BCrypt/rand fallback), tests for length/uniqueness |
| 8 | Predictable temp paths | **FIXED** — unique temp names (`tc-*-msi-*.log` etc.), cleanup on all terminal states |
| 11 | Partial scan ≡ complete | **FIXED** — `complete`/`scan_timed_out` flags, typed snapshot issues, honest Partial terminal |
| 15 | Frontend type drift | **FIXED** — types.ts mirrors implemented serde exactly (both protocol families) |
| 16 | `/toolchain` route missing | **FIXED** — canonical page exists; `/environment` redirects 308 |
| 19–22 | Update-only flows / no fix-PATH / no job persistence | **FIXED** — engine operations install/update/repair_path/health_check with persistence+recovery; update = re-plan/reinstall semantics |
| 23 | `prefs` map unused | **FIXED (superseded)** — UI prefs live in localStorage `stackpilot:toolchain:prefs:v1`; metadata prefs remain unused-but-typed |
| 6 | Plaintext secrets at rest on Unix | **MITIGATED/DOKUMENTED** — isolated file 0600, plaintext-mode warning logged at startup on Windows if DPAPI unavailable; OS keyring needs external crates (documented limitation) |
| 9,10 | run_elevated quoting / dynamic args | **UNCHANGED** (Windows-UAC fragility remains; see §7) |
| 12–14,17,18 | winget-on-non-Windows noise, glob_first first-match, dropped validate warnings, slow serial health report | **PARTIAL**: #12/#13 unreachable off-Windows (planner rejects non-Windows installs); #14 superseded by glob-all multi-install detection in domain layer; #18 superseded by bounded parallel scans with progress; #17 open (validate warnings beyond duplicates still not surfaced) |
| 24 | 3 700-line create page | **UNCHANGED** (out of refactor scope by design) |
| 25–27,29,30 | tool-specific installer hooks, Qt pipeline scraping, dual event protocols, process-PATH sync, eprintln logging | **ACCEPTED AS DOCUMENTED DEBT** — single mapping module exists (detect.rs to_legacy_status + commands.rs bridges, table-tested); structured events shipped alongside legacy |
| 28 | Whole-user-PATH replace race | **UNCHANGED** (small window; removal primitive kept for future rollback jobs) |

### 5.1 Safety-rule violations (original findings)

1. **Scan mutates the machine** — `check.rs::ensure_maui_workload()` runs
   `dotnet workload install maui` (up to 20 min timeout) inside `run_check` whenever
   dotnet appears installed; reached via `tc_check_environment`. Violates “scans never
   install”. Also silently changes the developer's .NET setup outside any approved plan.
2. **Secrets in normal responses** — `tc_get_metadata` returns the full
   `ToolchainMetadata` including the `secrets` map; `tc_get_install_status` returns
   `InstallSession.secrets`. Any caller/log/dump leaks persisted DB passwords.
   Contract §5.4 requires take-once-only delivery.
3. **No download integrity validation** — `console::download` writes bytes to temp with
   no checksum/signature; corrupted/MITM'd installers would be executed (many via UAC).
4. **Archive extraction not traversal-protected** — Expand-Archive/tar invocations
   (installer.rs, qt_installer.rs) don't validate entry paths (zip-slip class).
5. **PATH mutations implicit** — PATH entries added automatically inside install tasks
   (not an explicit user-approved action), and there is no remove/rollback command;
   audit trail limited to `InstalledToolInfo.path_entries`.

### 5.2 Security weaknesses / risks

6. Weak generated DB password: hex of nanosecond timestamp truncated to 16 chars
   (guessable entropy window) — `generate_db_password`.
7. Plaintext secrets at rest in state.json (acceptable only with OS user-scope threat
   model; should at least be documented + isolated from read APIs).
8. Predictable temp paths with PID (`tc-{tool}-{pid}.ps1`, elevated logs
   `tc-{tool}-{pid}-out.log`, downloads `tc-{tool}-{name}`) — temp-file squatting/link
   attacks by local users; also stale files accumulate (elevated logs removed on
   success path only).
9. `run_elevated` argument quoting (`"{}"` with backtick-escaped inner quotes joined
   into a single ArgumentList string) is fragile for exotic args — potential
   mis-quoting/injection edge cases.
10. PS scripts built by string interpolation of expanded env values; mitigations exist
    (ps_quote) but coverage is uneven (e.g., dynamic `--superpassword` goes through
    winget `--override` single-string).

### 5.3 Bugs / correctness issues

11. Partial scan reports are indistinguishable from complete ones: after the 90 s
    deadline, missing tools simply don't appear in `requirements` — UI shows “Ready”.
12. Non-Windows: `resolve()` always injects `winget` as first requirement (noise in
    reports on Linux/macOS).
13. PkgManager builder hardcodes `winget` binary — latent bug when Linux/macOS
    execution gets enabled (apt/dnf/brew sources would run winget).
14. `glob_first` returns the *first* directory entry matching the pattern (filesystem
    order), not the newest — e.g. `PostgreSQL/*/bin` may pick 10 over 17.
15. Frontend type drift vs backend models: TS `InstallSource` lacks
    `file_name/install_dir/needs_admin/execution`; TS `ToolDefinition` lacks
    `manual_install`; `EnvironmentCheck.optional_requirements` typed required while
    backend uses `#[serde(default)]` (page guards with `?? []`).
16. Nav item matches `/toolchain` but no such route file exists — direct navigation to
    `/toolchain` yields SvelteKit's error/fallback instead of the page.
17. `defs::validate` warnings other than duplicate ids are computed and dropped
    (never surfaced or asserted).
18. Health score counts only tools that have health_checks; combined with serial full
    scans, `tc_get_health_report` can take minutes (48 tools × up to 10 s probes ×
    detection+checks) and blocks with no progress event.

### 5.4 Stubs / incomplete features

19. Update flows are display-only: `UpdateAvailable` renders a label; there is no
    update command (installing again relies on winget upgrade semantics by accident).
20. No uninstall, no fix-PATH action, no per-tool recheck (only whole-catalog health).
21. Linux/macOS install execution stubbed out (`Skipped`), despite sources present.
22. Job model has no persistence/history: `InstallSession` lives only in memory; app
    restart mid-install loses state (state.json records successes only); no resume.
23. `prefs` map in metadata exists but nothing reads/writes it.

### 5.5 Architectural risks

24. `create/+page.svelte` ≈ 3 700 lines hosting the entire wizard + environment +
    install UX inline — high regression risk during migration; extract before/while
    touching.
25. Tool-specific hacks inside the generic installer (erlang NSIS flags, php.ini
    rewriting, composer shim, MAUI workload) — each new ecosystem grows special cases;
    target design needs a declarative post-install hook mechanism.
26. Qt pipeline depends on scraping HTML listings and regex-free hand parsing of
    Updates.xml; pinned to the Qt 6.8 branch by prefix check `"68"`.
27. Dual emission risk during migration: two event protocols and two status vocabularies
    coexisting — need one mapping module tested both ways.
28. Whole-user-PATH replace on write (`SetEnvironmentVariable('Path', merged, 'User')`)
    races external modifications between read and write; window is small but real.
29. `sync_process_path` mutates the process-wide `PATH` env var — affects every module
    implicitly; acceptable but must remain centralized.
30. eprintln-based logging (some gated by DEVLAUNCHER_DEBUG) — no levels, no sink;
    activity panel will need structured events rather than scraped strings long-term.

---

## 6. Tests — what exists, what's missing

### 6.1 Existing Rust tests (inline `#[cfg(test)]`, run via `cargo test`)

Toolchain:

- `core/version.rs` — parse/compare/meets_min.
- `core/discovery.rs` — version rules application, %VAR% expansion, glob_first,
  detect smoke (machine-dependent assertions kept tolerant).
- `core/requirements.rs` — ~25 mapping/regression tests incl. exhaustive guards:
  every resolved id exists in tools.json; every framework yields ≥1 requirement;
  dual-docker opt-in semantics; ordering (winget first, erlang-before-elixir,
  php-before-composer).
- `core/check.rs` — fake-def based: unknown ids ignored, installable/manual/bundled
  handling, space math, progress events.
- `core/planner.rs` — scheduling rules, selection restriction, winget-first,
  manual/docker never scheduled.
- `core/installer.rs` — command shapes (winget/postgres override/phar/ps1/sh/msi/
  msixbundle/zip/tgz/git clone targets), php.ini canonicalization + idempotence,
  end-to-end echo install (Windows), fallback quietness, single aggregated error,
  abort marking, unknown-tool skipping.
- `core/console.rs` — piped_run streaming/error-line capture, abort kills process,
  ps_quote.
- `core/path_service.rs` — merge/expand purity tests.
- `core/metadata.rs` — load-missing/broken defaults, save/reload roundtrip, overwrite.
- `core/health.rs` — check aggregation rules, score semantics.
- `core/qt_installer.rs` — Updates.xml parsing/version-suffix helpers (pure parts).
- `core/disk.rs` — parsers, install_root shape.
- `platforms/mod.rs`, `platforms/windows.rs`, `platforms/unix_rc.rs` — split_path,
  quoting script, rc-block read/write/sanitize.

Elsewhere (context): project_creator knowledge/validate/normalize/engine*/generators/
packs test modules; workspace/devlauncher have none relevant here.

### 6.2 Test coverage now (acceptance stage)

Backend (inline `#[cfg(test)]`, `cargo test`): **624 passed, 0 failed, 1 ignored**
(live-network Qt resolution test, ignored by design). Highlights beyond the
legacy core/platforms suites:

- scan purity (no machine mutation), bounded concurrency, deterministic order,
  deadline→Partial, cancel semantics, journal recovery, stale-event isolation,
  sanitized probe output;
- serialization contracts for both protocol families (kind-tagged domain,
  externally tagged engine) incl. old-file tolerance;
- engine planner/executor: request-model injection rejections, dependency
  closure, conflicts, idempotency/no-op truth, integrity/admin confirmations,
  persistence-before-execution, restart recovery, terminal math, path audit,
  secret-free job records, bounded history ring;
- Project Creator compat mappings (legacy states/phases/session status,
  completion summary, take-once secrets, order+options preservation).

Frontend (`npm test`, vitest): **75 tests** across types/events/stateLogic/
filters/format — union discrimination incl. unknown-kind rejection, channel
ref-counting and handler isolation, identity/seq stale guards, cache/live merge,
mutation gating, formatters and sanitization.

Still absent: component-level UI tests (no Svelte test renderer configured),
end-to-end Playwright, real-install smoke automation (manual checklist lives in
`docs/toolchain-acceptance.md`).
