# Contract: standalone Toolchain (canonical `tcx_*` flow)

Status: **authoritative** for the standalone Toolchain page and its backend pipeline.
Created before the detection-semantics refactor; implementation must conform to this file.

Legacy Project Creator contracts are documented in code (`src/lib/modules/toolchain/compat.ts`,
`commands.rs` header) and remain untouched by this contract except where §7 states compatibility
rules explicitly.

---

## 1. Immutable scope boundaries

In scope (this contract governs):

- `src/routes/toolchain/+page.svelte`
- `src/lib/modules/toolchain/**`
- standalone backend flow: `tcx_*` commands in `modules/toolchain/commands.rs`
- `modules/toolchain/domain/**` (`detect`, `models`, `engine`, `probe`, `path_report`,
  `score`, `cache`, `profile`)
- catalog definition (`tools.json` + `defs.rs`) as consumed by the canonical scan
- canonical planner/installer behavior reachable from the Toolchain page
  (`engine/planner.rs`, `EngineRequest` → `CanonicalPlan`)

Out of scope — MUST NOT be modified:

- Project Creator frontend integration (`routes/create/+page.svelte`,
  `lib/modules/project_creator/**`, Project Creator UI contracts)
- legacy `tc_*` command signatures and payload shapes (backward compatible, still used
  by Project Creator). Shared backend primitives may be extracted ONLY if legacy API
  behavior stays byte-compatible and all existing legacy tests pass.
- marketplace functionality (not implemented yet)
- UI redesign beyond what truthfulness of data requires

## 2. Canonical architecture

One domain pipeline produces one canonical result per tool. There is exactly ONE scan path
for the standalone Toolchain page:

```
catalog definition (tools.json)
  → platform context (ScanContext: process PATH, user PATH, managed/dual ids, os_name)
  → detection evidence (detect::detect_detailed → DetailedDetection)
  → version assessment (assess_versions vs VersionRules)
  → health policy (run_health_checks over catalog-declared health_checks)
  → PATH analysis (tool_path_findings)
  → provenance (metadata facts only, no guessing)
  → applicability (classify_applicability)
  → canonical ToolScanResult (detect::scan_tool)
  → snapshot / UI / planner consumers
```

Rules:

- The scan is strictly read-only on the machine. Only catalog-approved probes
  (`version_probes`, `known_paths`, `registry_keys`, `health_checks` from `tools.json`)
  can be executed; arbitrary frontend commands are unrepresentable by types.
- All probes go through `domain/probe.rs::run_probe`: timeout, output cap, secret
  sanitization, stdin disabled.
- The snapshot (`EnvironmentSnapshot`) is the single source of truth for the UI;
  results keep deterministic catalog order regardless of probe completion order.

## 3. Canonical scan lifecycle

`tcx_start_scan` → engine job:
`Queued → Environment → Tools (bounded parallelism, default 4) → Finalizing → Done`.

- Reconnect semantics: starting a scan while one is running returns the running job
  (`AlreadyRunning`); a second parallel scan never exists.
- Cancellation: flag checked after slot acquisition; already-started tools finish,
  unstarted tools stay honestly `ScanPending`; terminal = `Cancelled`/`Partial`.
- Deadline: overall soft deadline; on hit, remaining tools stay `ScanPending`,
  terminal = `Partial`, snapshot carries a `partial_scan` error issue.
- Journal: `scan-journal.json` survives restarts; a `Running` journal entry found at
  startup becomes `Interrupted`. There are no "eternal" running scans.
- Cache: last valid snapshot persists (`scan-snapshot.json`), is served immediately,
  and is honestly marked `stale` when older than the freshness threshold (default 5 min).
  A cache hit never pretends to be live data.
- Events: `toolchainx:scan_progress` / `toolchainx:scan_done`, always carrying
  `job_id` + `scan_id`. Events without operation identity do not exist
  (`ScanProgressEvent::try_new` returns `None`).

## 4. Canonical `ToolScanResult` semantics

`ToolScanResult` preserves ALL of the following, so any verdict can be explained from
the payload alone:

| Field | Meaning |
|---|---|
| `installs` | every detected installation/trace (multiple allowed), each with raw version, parsed version, location, evidence kind, PATH reachability |
| `detection` | `DetectionOutcome`: see below |
| `error` | error information for failed polls (reason string), else null |
| `duration_ms` | wall time of this tool's poll |
| `health` | health outcome incl. per-check results, or None when not reached |
| `path_findings` | per-tool PATH diagnostics |
| `provenance`, `applicability`, `capabilities`, `version_assessment` | independent dimensions |
| `state` | presentation composition of the dimensions |

### 4.1 Detection outcomes (`DetectionOutcome`)

Serialized as internally-tagged `{"kind": ...}` snake_case objects.

- `pending` — not scanned yet (partial snapshots only).
- `not_detected` — clean negative: probes ran, no evidence found anywhere.
- `detected` — positive result: at least one installation/evidence exists
  (`installs` non-empty or silent-on-path evidence present). Evidence lives in
  `installs`; the outcome itself carries no duplicate payload.
- `failed { reason }` — inconclusive poll (timeouts with zero positive evidence /
  launch errors): "unknown", NEVER presented as missing.

Forbidden: returning `not_detected` when evidence exists. Detection verdict must match
the evidence set.

### 4.2 Verdict table (normative)

| Situation | `detection` | installs evidence | state |
|---|---|---|---|
| No installation anywhere | `not_detected` | — | `Missing` (or `ManualInstall`/`DockerManaged`/`BuiltInSystem`/`UnsupportedPlatform` per applicability) |
| Probe via PATH responds with parseable version | `detected` | ≥1 install, `VersionProbe` | `InstalledHealthy`/`InstalledHealthUnknown`/`InstalledUnhealthy`/`UpdateAvailable` per health+assessment |
| Known-path binary responds directly | `detected` | install, `KnownPath`, `reachable_via_path` reflects dir visibility | same as above |
| Executable exists in PATH but command fails / outputs nothing | `detected` | footprint install, PATH-visible | `PathBroken { reason }` |
| Known-path dir contains expected binary that fails to run (nonzero exit / empty output / timeout / launch error) | `detected` | footprint install, `KnownPath` | `PathBroken { reason }` — explicit executable-broken evidence |
| Installation footprint WITHOUT working executable and WITHOUT broken-executable/path evidence (registry key only, leftover dir without binaries) | `detected` | footprint install | `InstalledHealthUnknown { version: "" }` — honest neutral; **NOT** `PathBroken` |
| Command timeout, no other evidence | `failed` | — | `ScanFailed { reason }` ("unknown ≠ missing") |
| Launch failure, no other evidence | `failed` | — | `ScanFailed { reason }` |
| Timeout/launch error BUT evidence exists elsewhere | `detected` | evidence wins over inconclusive | per evidence rows above |
| Unsupported platform (sources exist, none for this OS), nothing installed | `not_detected` | — | `UnsupportedPlatform` |
| Manual-only tool, nothing installed | `not_detected` | — | `ManualInstall { reason }` |
| Manual-only/bundled tool WITH footprint | `detected` | footprint | per footprint rows (footprint beats the "install it manually" claim) |
| Bundled tool (e.g. npm) responding | `detected` | working install | normal installed states + `Provenance::BundledWith { host }` |

### 4.3 Health policy

- Health checks run whenever installation evidence exists — **independently of version
  parsing success**. An unparseable/garbage version must NOT suppress health checks.
- Empty `health_checks` list ⇒ `NoChecksDefined` ("never checked"), not "unhealthy".
- Process-level failure (launch error / timeout / binary absent) ⇒ `FailedToRun`
  ("could not check"), distinct from assertion failure ⇒ `Unhealthy`.
- All checks passed + assessment below recommended/min ⇒ `Degraded`.
- No installation evidence and clean negative ⇒ `Unavailable` (nothing to check);
  failed poll ⇒ `health: None` (pending).
- Only `Healthy`/`Degraded`/`Unhealthy` are verdicts; everything else is "no data".

### 4.4 Provenance (facts only, never guessed)

`state.json` record ⇒ `StackPilotManaged`; OS built-in ⇒ `System`; responding tool with
`bundled_with` ⇒ `BundledWith`; dual docker tool absent locally ⇒ `Docker`; responding
standalone ⇒ `External`; otherwise `Unknown`. PackageManager provenance is NOT inferred.

## 5. State and health semantics (frontend view)

- Frontend discriminates unions ONLY by `kind`; unknown kinds render as "unknown state"
  but never crash the UI (`parseToolState`, drawer fallbacks).
- `state` is a composition, not a replacement of dimensions; UI must keep showing
  installs/PATH/health evidence alongside the composed state.
- Snapshot freshness: cached snapshots render instantly with stale badge; live slice
  overlays incrementally and never erases previous tools.

## 6. Planner/installer invariants

- Plans are built by the BACKEND from the catalog (`build_plan`); the frontend submits
  only an `EngineRequest` (operation + tool ids + explicit choices).
  `deny_unknown_fields`: URLs/paths/args/versions are physically unexpressible.
- Plan preview (`tcx_build_plan`) never mutates; execution always re-canonicalizes the
  plan from fresh detection right before running.
- Jobs persist BEFORE execution; restart mid-job ⇒ `Interrupted`. Secrets go to the
  isolated store only, surfaced once via one-shot retrieval; never in plans/status/events.

## 7. Frontend/backend compatibility rules

- Canonical enums: internally tagged `{"kind": ...}` snake_case; unit variants are
  one-field objects. Engine enums are externally tagged snake_case (documented deviation).
- Legacy section A types (`tc_*`) stay byte-compatible.
- Additive changes are allowed when old payloads keep deserializing (serde defaults)
  and unknown kinds don't crash the frontend. Adding `DetectionOutcome::Detected` is
  such an additive change; old caches without it stay valid.
- Legacy mapping `to_legacy_status` must remain total over `ToolState`.

## 8. Security invariants

- Scan/mutation separation: scans never write machine state (guarded by regression test
  `scan_never_writes_machine_state_files`).
- Probe output sanitized (secret-like env values masked, control chars flattened, capped).
- Frontend is never authoritative for plan content; tool ids validated against catalog.
- Snapshots/journals contain no secrets by construction.

## 9. Test acceptance criteria

Rust (domain):

1. successful detection ⇒ `DetectionOutcome::Detected` + installed state (no false
   `NotDetected`);
2. detection failure (missing tool) ⇒ `NotDetected` + `Missing`;
3. known-path installation detected with `KnownPath` evidence;
4. PATH-visible executable failing its command ⇒ `PathBroken` with explanatory reason;
5. footprint-only evidence WITHOUT broken-executable evidence ⇒ NOT `PathBroken`
   (honest neutral state), footprint preserved in `installs`;
6. health checks execute even when version parsing fails (results non-empty,
   independent gating);
7. bundled tool semantics (provenance names host);
8. unsupported platform classification;
9. timeout-only poll ⇒ `Failed` (ScanFailed), never `NotDetected`/`Missing`;
10. serialization round-trips incl. new variant; legacy status mapping stays total.

Frontend: existing vitest suites keep passing; typecheck (`svelte-check`) passes.

Commands: `cargo fmt --check`, `cargo test`, `npm run test`, `npm run check`.
