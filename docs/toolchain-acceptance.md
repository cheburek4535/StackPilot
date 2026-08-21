# Toolchain Control Center — Manual Acceptance Checklist

Concise pass/fail checklist for a human session on Windows (the supported
install platform). Every item maps to implemented, test-guarded behavior; this
document exists because some qualities (visuals, focus flow, real installs)
cannot be proven by automated tests alone.

Start: `npm run tauri dev`, sign in to the app shell, open **Toolchain** in the
sidebar (or navigate to `/toolchain`; `/environment` must redirect here).

---

## 1. Entry & diagnostics

- [ ] Page opens instantly showing cached state (or an honest "no data yet"
      empty state on true first run) — never a blank screen.
- [ ] A read-only diagnostic scan starts automatically; progress strip shows
      phase + `n/m` and can be cancelled mid-run.
- [ ] Cards appear/fill incrementally while scanning; nothing disappears.
- [ ] After the scan: hero shows score, verdict, required-tools summary,
      disk headroom, admin capability, last-scan age. Partial/cancelled scans
      are labeled honestly (never rendered as success).
- [ ] Header freshness badge flips to «устарели» after the snapshot ages out.

## 2. Manage Everything mode

- [ ] Search matches name/id/category; clear button works.
- [ ] Filters: state kinds (with counts), health, categories, provenance,
      capabilities, host/docker execution, manual-only, update-only,
      admin-only. Keyboard: all checkboxes reachable and toggleable by keyboard.
- [ ] Sorting: problematic-first / name / category / catalog order.
- [ ] Empty results show a compact empty state with a working filter reset.
- [ ] Manual-only tools (unity, unreal, godot, qt, xcodebuild…) show honest
      badges/instructions — never install buttons that would do nothing.
- [ ] Unsupported-on-platform tools are marked «не поддерживается» and are
      excluded from score/hero alarm math.
- [ ] Tools with scan errors show «Ошибка проверки», not «не установлен».

## 3. Tool details drawer

- [ ] Open from card title/details button; Escape and backdrop close it;
      focus moves into the panel and returns to the trigger on close.
- [ ] Drawer shows: version vs recommended + assessment badge, detection
      outcome with per-install evidence/path/PATH-reachability, health checks
      with durations, PATH findings, dependencies/conflicts/dependents,
      per-OS sources with integrity marker, Docker image/notes, size/admin
      facts, notes/manual instructions, docs links, tool-filtered job log.
- [ ] For an external/unknown-provenance tool the footer offers
      «Отслеживать»; after confirming, the «отслеживается» badge appears
      (and survives reopening). It is NOT shown for StackPilot-managed tools.

## 4. Build Environment mode

- [ ] Picker loads from backend wizard tree (no hardcoded stacks); project-type
      presets fill languages/tools; framework search works; incompatible
      frameworks are disabled with a reason tooltip.
- [ ] Resolved profile groups appear with per-tool inclusion reasons
      («Выбрано в требованиях», bundled-with, dependency-of…), sizes, admin flags.
- [ ] Dual docker tools (postgresql/redis/mongodb/kafka/grafana/mysql) default
      to optional/Docker; opting into local install moves them to Required.
- [ ] Readiness badge is honest: without a scan it says data is missing.
- [ ] «Проверить план установки» opens plan review.

## 5. Plan review & job lifecycle

- [ ] Plan preview lists tasks with action/source/deps/size/UAC and truthful
      no-op rows («уже установлен…»).
- [ ] Warnings: unverified sources, admin elevation, reinstall-on-broken.
      Explicit consent checkboxes gate the start button when present.
- [ ] Blocked reasons render when there is nothing to do / not enough disk /
      elevation unsupported / another mutation is active.
- [ ] Start → job appears in activity strip and Job Center with task
      progress and running phase. Cancel kills the current task; remaining
      tasks become Cancelled; terminal status is Cancelled (not error).
- [ ] Logs expand per job row; recovered jobs show «после перезапуска» and
      offer retry; retry re-plans from fresh facts as a NEW job id.
- [ ] On success the snapshot refreshes in background; tool cards flip to
      installed states. No success is claimed from UI guesswork.

## 6. Safety spot-checks

- [ ] While any mutation job runs, a second mutating start is blocked with an
      explanation; scans remain allowed (read-only).
- [ ] PATH repair only inside an approved job; afterwards the tool's PATH
      findings clear on next scan; job record contains the audited diff.
- [ ] No secrets anywhere: metadata/status payloads, job records, logs, events.
      Secrets from an install appear once via the wizard's one-shot display.
- [ ] Kill the app mid-install → restart: job shows Interrupted («после
      перезапуска»), never a stuck «running»; retry works.

## 7. Cross-cutting

- [ ] Dark and light themes both readable (no hardcoded colors).
- [ ] Narrow window (~<980px): filter rail collapses behind a toggle, build
      mode stacks, hero stats reflow, drawer goes full-width.
- [ ] Keyboard-only pass: tabs, search, filters, card actions, drawer, modal
      plan review all operable; visible focus rings everywhere.
- [ ] Reduced motion (OS setting): pulse/spin animations effectively stop.
- [ ] Project Creator regression: wizard → environment check → optional docker
      toggles → install → completion still behaves exactly as before.
