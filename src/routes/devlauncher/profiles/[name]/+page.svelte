<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getProfile, getDemoProfile, executeAction, deleteProfile, runProfile } from "$lib/modules/devlauncher/api";
  import { setCurrentProject } from "$lib/modules/workspace/api";
  import type { LaunchProfile, ActionType, ActionStatus, LaunchRun } from "$lib/modules/devlauncher/types";
  import { isV2Profile, isRunTerminal, runStatusLabel, stepKindIcon, stepKindLabel, stepKindSummary, stepStatusClass, trackingQualityLabel } from "$lib/modules/devlauncher/types";
  import * as runStore from "$lib/modules/devlauncher/runStore";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let profile = $state<LaunchProfile | null>(null);
  let loading = $state(true);
  let errorMsg = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let runningAll = $state(false);
  let currentAction = $state<string | null>(null);

  /** V2 run state. */
  let activeRun = $state<LaunchRun | null>(null);
  /** Whether the profile has been launched (drives visibility of the
   *  "Open in Workspace" button — it stays hidden until the launch). */
  let launched = $state(false);
  let logRunId = $state<string | null>(null);
  let logStepId = $state<string | null>(null);
  let logLines = $state<string[]>([]);
  let logOpen = $state(false);

  let profileName = $derived($page.params.name);
  const isV2 = $derived(profile ? isV2Profile(profile) : false);

  let summary = $derived.by(() => {
    if (actionResults.size === 0) return null;
    let ok = 0, err = 0, skip = 0;
    for (const v of actionResults.values()) {
      if (v.startsWith("✓")) ok++;
      else if (v.startsWith("✗")) err++;
      else if (v.startsWith("—")) skip++;
    }
    return { ok, err, skip };
  });

  let failedIds = $derived.by(() => {
    if (!profile || actionResults.size === 0) return [];
    return profile.actions
      .filter((a) => actionResults.get(a.id)?.startsWith("✗"))
      .map((a) => a.id);
  });

  onMount(async () => {
    await runStore.init();
    await loadProfile();
    // Recover any active run for this profile.
    await recoverRun();
    // If the user clicked "Run" on a recent mini-card on the Home page, the
    // profile page was opened with ?run=1 — launch the profile here.
    if ($page.url.searchParams.get("run") === "1") {
      await runAll();
    }
  });

  onDestroy(() => {
    runStore.destroy();
  });

  async function loadProfile() {
    loading = true;
    errorMsg = "";
    const name = profileName;

    if (!name) {
      errorMsg = i18n.t("devl.profile_not_specified") as TranslationKey;
      loading = false;
      return;
    }

    try {
      const demo = await getDemoProfile();
      if (demo.name === name) {
        profile = demo;
      } else {
        profile = await getProfile(name);
      }
    } catch (e) {
      errorMsg = i18n.t("devl.profile_load_failed", { name, err: String(e) }) as TranslationKey;
    }

    loading = false;
    // Recover the latest run and refresh history after the profile loads.
    await recoverRun();
    refreshHistory();
  }

  /** Map a step id to its display label using the profile's step graph. */
  function stepLabel(stepId: string): string {
    if (!profile || !Array.isArray((profile as any).steps)) return stepId;
    const steps = (profile as any).steps as Array<{ id: string; label?: string }>;
    const found = steps.find((s) => s.id === stepId);
    return found?.label || stepId;
  }

  /** Stop all processes of a run. */
  async function stopRun(runId: string) {
    try {
      await runStore.stopProcesses(runId);
      if (activeRun?.run_id === runId) {
        const updated = await runStore.fetchRun(runId);
        if (updated) activeRun = updated;
      }
      refreshHistory();
    } catch { /* non-critical */ }
  }

  /** Recover the latest run for this profile (including finished runs). */
  async function recoverRun() {
    if (!profile) return;
    const latest = runStore.getLatestRunForProfile(profile.name);
    if (latest) {
      activeRun = latest;
    } else {
      const runs = runStore.getActiveRuns();
      if (runs.length > 0) {
        activeRun = runs[runs.length - 1];
      }
    }
    // A still-active run means the profile has really been launched.
    if (activeRun && !isRunTerminal(activeRun.status)) {
      launched = true;
    }
  }

  /** Runs for this profile (history). */
  let historyRuns = $state<LaunchRun[]>([]);

  function refreshHistory() {
    if (!profile) {
      historyRuns = [];
      return;
    }
    historyRuns = runStore
      .getAllRuns()
      .filter((r) => r.profile_name === profile!.name)
      .sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  async function runAction(actionId: string) {
    if (!profile) return;
    const action = profile.actions.find((a) => a.id === actionId);
    if (!action) return;

    try {
      const result = await executeAction(action);
      const msg = formatResult(result);
      actionResults = new Map(actionResults.set(actionId, msg));
    } catch (e) {
      actionResults = new Map(actionResults.set(actionId, `✗ ${e}`));
    }
  }

  async function runAll() {
    if (!profile || runningAll) return;
    runningAll = true;
    activeRun = null;
    launched = true;

    // Bind the profile to the workspace project so the Workspace module
    // (Overview/Runtime/Logs) shows the processes after the launch.
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
    } catch {
      // Non-critical — the launch itself continues.
    }

    if (isV2 && profile.id && profile.schema_version) {
      // V2 path: use event-driven orchestrator.
      try {
        const v2Profile = profile as unknown as import("$lib/modules/devlauncher/types").LaunchProfileV2;
        const run = await runStore.launchRun(v2Profile);
        activeRun = run;
        refreshHistory();
        // Poll for updates.
        pollRun(run.run_id);
      } catch (e) {
        errorMsg = i18n.t("devl.launch_failed", { err: String(e) }) as TranslationKey;
      }
      runningAll = false;
      return;
    }

    // Legacy path.
    const results = new Map<string, string>();
    try {
      const profileResults = await runProfile(profile);
      for (const [actionId, status] of profileResults) {
        results.set(actionId, formatResult(status));
      }
    } catch (e) {
      for (const action of profile.actions) {
        if (!action.enabled) continue;
        currentAction = action.label;
        try {
          const result = await executeAction(action);
          results.set(action.id, formatResult(result));
        } catch (err) {
          results.set(action.id, `✗ ${err}`);
        }
        actionResults = new Map(results);
      }
    }
    actionResults = new Map(results);
    currentAction = null;
    runningAll = false;
    goto("/devlauncher/processes");
  }

  /** Poll a V2 run for updates. */
  async function pollRun(runId: string) {
    try {
      const run = await runStore.fetchRun(runId);
      if (run) {
        activeRun = run;
        if (!isRunTerminal(run.status)) {
          setTimeout(() => pollRun(runId), 1000);
        }
      }
    } catch { /* non-critical */ }
  }

  async function retryFailed() {
    if (!profile || runningAll) return;
    runningAll = true;
    const results = new Map(actionResults);
    for (const id of failedIds) {
      const action = profile.actions.find((a) => a.id === id);
      if (!action) continue;
      currentAction = action.label;
      try {
        const result = await executeAction(action);
        results.set(action.id, formatResult(result));
      } catch (e) {
        results.set(action.id, `✗ ${e}`);
      }
      actionResults = new Map(results);
    }
    currentAction = null;
    runningAll = false;
  }

  function clearResults() {
    actionResults = new Map();
  }

  /** Cancel the active V2 run. Idempotent. */
  async function cancelRun() {
    await runStore.cancelCurrentRun();
    if (activeRun) {
      const updated = await runStore.fetchRun(activeRun.run_id);
      if (updated) activeRun = updated;
    }
  }

  /** View logs for a V2 step. */
  async function viewStepLogs(stepId: string) {
    if (!activeRun) return;
    logRunId = activeRun.run_id;
    logStepId = stepId;
    logOpen = true;
    logLines = [];
    try {
      const { getStepLogs } = await import("$lib/modules/devlauncher/api");
      const logs = await getStepLogs(activeRun.run_id, stepId);
      logLines = [...logs.stdout, ...logs.stderr];
    } catch (e) {
      logLines = [`Failed to load logs: ${e}`];
    }
  }

  /** View combined logs for an entire run. */
  async function viewRunLogs(runId: string) {
    logRunId = runId;
    logStepId = null;
    logOpen = true;
    logLines = [];
    try {
      const { getRunLogs } = await import("$lib/modules/devlauncher/api");
      const logs = await getRunLogs(runId);
      logLines = [...logs.stdout, ...logs.stderr];
    } catch (e) {
      logLines = [`Failed to load logs: ${e}`];
    }
  }

  function closeLogs() {
    logOpen = false;
    logRunId = null;
    logStepId = null;
    logLines = [];
  }

  function formatResult(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }

  function actionIcon(act: ActionType): string {
    if ("RunCommand" in act) return "▶";
    if ("OpenUrl" in act) return "🌐";
    if ("OpenApplication" in act) return "⬛";
    if ("WaitForUrl" in act) return "⏳";
    if ("WaitForPort" in act) return "⏳";
    if ("Delay" in act) return "⏱";
    if ("ExecuteScript" in act) return "▶";
    return "?";
  }

  function actionDetail(act: ActionType): string {
    if ("RunCommand" in act) return act.RunCommand.command;
    if ("OpenUrl" in act) return act.OpenUrl.url;
    if ("OpenApplication" in act) return act.OpenApplication.path;
    if ("WaitForUrl" in act) return act.WaitForUrl.url;
    if ("WaitForPort" in act) return `${act.WaitForPort.host}:${act.WaitForPort.port}`;
    if ("Delay" in act) return `${act.Delay.seconds}s`;
    if ("ExecuteScript" in act) return act.ExecuteScript.script;
    return "?";
  }

  function actionTypeLabel(act: ActionType): string {
    if ("RunCommand" in act) return i18n.t("act.command");
    if ("OpenUrl" in act) return i18n.t("act.url");
    if ("OpenApplication" in act) return i18n.t("act.app");
    if ("WaitForUrl" in act) return i18n.t("act.wait_url");
    if ("WaitForPort" in act) return i18n.t("act.wait_port");
    if ("Delay" in act) return i18n.t("act.delay");
    if ("ExecuteScript" in act) return i18n.t("act.script");
    return i18n.t("act.unknown");
  }

  function goBack() {
    goto("/devlauncher/profiles");
  }

  async function handleDelete() {
    if (!profile) return;
    if (!confirm(i18n.t("devl.confirm_delete", { name: profile.name }))) return;
    try {
      await deleteProfile(profile.name);
      goto("/devlauncher/profiles");
    } catch (e) {
      errorMsg = i18n.t("devl.toast_delete_failed", { err: String(e) }) as TranslationKey;
    }
  }

  async function openInWorkspace() {
    if (!profile) return;
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
      goto("/workspace");
    } catch (e) {
      errorMsg = i18n.t("devl.open_workspace_failed", { err: String(e) }) as TranslationKey;
    }
  }

  function resultClass(msg: string): string {
    if (msg.startsWith("✓")) return "ok";
    if (msg.startsWith("✗")) return "err";
    if (msg.startsWith("—")) return "skip";
    return "";
  }
</script>

<main>
  <button class="back-btn" onclick={goBack}>{i18n.t("devl.all_profiles") as TranslationKey}</button>

  {#if loading}
    <p class="empty">{i18n.t("devl.profile_loading") as TranslationKey}</p>

  {:else if errorMsg}
    <div class="error-card">
      <p>{errorMsg}</p>
      <button class="secondary" onclick={goBack}>{i18n.t("devl.back_to_list") as TranslationKey}</button>
    </div>

  {:else if profile}
    <div class="profile-header">
      <div>
        <h1>{profile.name}</h1>
        <p class="desc">{profile.description}</p>
        {#if isV2}
          <span class="v2-badge">V2</span>
        {/if}
      </div>
      <div class="header-actions">
        <button class="danger-outline" onclick={handleDelete} disabled={runningAll}>
          {i18n.t("devl.delete_profile") as TranslationKey}
        </button>
        {#if launched}
          <button class="secondary" onclick={openInWorkspace}>
            {i18n.t("devl.open_in_workspace") as TranslationKey}
          </button>
        {/if}
        <button class="primary" onclick={runAll} disabled={runningAll}>
          {runningAll ? (i18n.t("devl.running") as TranslationKey) : (i18n.t("devl.run_all") as TranslationKey)}
        </button>
        {#if activeRun && !isRunTerminal(activeRun.status)}
          <button class="danger-outline" onclick={cancelRun}>
            {i18n.t("devl.cancel_run") as TranslationKey}
          </button>
        {:else if activeRun}
          <button class="danger-outline" onclick={() => stopRun(activeRun!.run_id)}>
            {i18n.t("devl.stop_processes") as TranslationKey}
          </button>
        {/if}
      </div>
    </div>

    {#if currentAction}
      <p class="running-hint">▶ {currentAction}…</p>
    {/if}

    <!-- V2 Run State -->
    {#if activeRun}
      <section class="run-section">
        <div class="run-header">
          <h2>Run: {activeRun.profile_name}</h2>
          <span class="run-status {activeRun.status}">{runStatusLabel(activeRun.status)}</span>
        </div>
        <div class="run-steps">
          {#each activeRun.steps as step}
            <div class="run-step {stepStatusClass(step.status)}">
              <span class="step-icon">
                {#if step.status === "pending"}○
                {:else if step.status === "running"}◉
                {:else if step.status === "succeeded"}✓
                {:else if step.status === "failed"}✗
                {:else if step.status === "skipped"}—
                {:else if step.status === "cancelled"}⊘
                {:else if step.status === "retrying"}↻
                {:else}?{/if}
              </span>
              <div class="step-info">
                <span class="step-id">{stepLabel(step.step_id)}</span>
                {#if step.error}
                  <span class="step-error">{step.error}</span>
                {/if}
              </div>
              <div class="step-controls">
                {#if (step.status === "running" || step.status === "succeeded") && step.process_id}
                  <button class="small-btn" onclick={() => viewStepLogs(step.step_id)}>
                    Logs
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
        {#if activeRun.diagnostics.length > 0}
          <div class="run-diagnostics">
            <h3>Diagnostics</h3>
            {#each activeRun.diagnostics as diag}
              <p class="diag {diag.severity === "error" ? "diag-err" : diag.severity === "warning" ? "diag-warn" : "diag-info"}">
                [{diag.source}] {diag.message}
                {#if diag.step_id}
                  <span class="diag-step">(step: {diag.step_id})</span>
                {/if}
              </p>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    <!-- Run history -->
    {#if historyRuns.length > 0}
      <section class="run-section">
        <div class="run-header">
          <h2>{i18n.t("devl.run_history") as TranslationKey}</h2>
        </div>
        <div class="run-steps">
          {#each historyRuns as r}
            <div class="run-history-row" class:active={activeRun?.run_id === r.run_id}>
              <button class="history-main" onclick={() => (activeRun = runStore.getRunById(r.run_id) ?? r)}>
                <span class="history-id">#{r.run_id.slice(0, 8)}</span>
                <span class="run-status {r.status}">{runStatusLabel(r.status)}</span>
                <span class="history-time">{new Date(r.created_at).toLocaleString()}</span>
              </button>
              <div class="step-controls">
                {#if !isRunTerminal(r.status)}
                  <button class="small-btn" onclick={() => stopRun(r.run_id)}>
                    {i18n.t("devl.stop_processes") as TranslationKey}
                  </button>
                {/if}
                <button class="small-btn" onclick={() => viewRunLogs(r.run_id)}>
                  {i18n.t("devl.logs") as TranslationKey}
                </button>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- Legacy Actions (always shown for backward compatibility) -->
    <section>
      <h2>{i18n.t("devl.actions", { n: profile.actions.length }) as TranslationKey}</h2>
      <div class="action-list">
        {#each profile.actions as action}
          <div class="action-row" class:disabled={!action.enabled}>
            <span class="action-icon">{actionIcon(action.action_type)}</span>
            <div class="action-info">
              <span class="action-label">{action.label}</span>
              <span class="action-type">{actionTypeLabel(action.action_type)}</span>
              <span class="action-detail">{actionDetail(action.action_type)}</span>
            </div>
            <div class="action-controls">
              <button
                class="run-btn"
                onclick={() => runAction(action.id)}
                disabled={!action.enabled}
                title={i18n.t("devl.execute_tooltip") as TranslationKey}
              >
                ▶
              </button>
              <span class="toggle" class:active={action.enabled}>
                {action.enabled ? (i18n.t("devl.on") as TranslationKey) : (i18n.t("devl.off") as TranslationKey)}
              </span>
            </div>
          </div>
          {#if actionResults.has(action.id)}
            <div class="result-row {resultClass(actionResults.get(action.id)!)}">
              {actionResults.get(action.id)}
            </div>
          {/if}
        {/each}
      </div>
    </section>

    {#if summary}
      <div class="summary">
        <span class="summary-ok">✓ {summary.ok}</span>
        <span class="summary-err">✗ {summary.err}</span>
        <span class="summary-skip">— {summary.skip}</span>
        <div class="summary-actions">
          {#if failedIds.length > 0 && !runningAll}
            <button class="secondary" onclick={retryFailed}>{i18n.t("devl.retry_failed") as TranslationKey}</button>
          {/if}
          <button class="secondary" onclick={clearResults}>{i18n.t("devl.clear_results") as TranslationKey}</button>
        </div>
      </div>
    {/if}
  {/if}
</main>

<!-- Log Viewer Modal -->
{#if logOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="modal-overlay" onclick={closeLogs} role="presentation">
    <!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
    <div class="modal-content" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Step logs">
      <div class="modal-header">
        <span>Logs: {logStepId}</span>
        <button class="modal-close" onclick={closeLogs}>✕</button>
      </div>
      <div class="modal-body">
        {#if logLines.length === 0}
          <div class="log-empty">Нет логов для этого шага (процесс не запускался или терминал открыт в отдельном окне).</div>
        {:else}
          {#each logLines as line}
            <div class="log-line">{line}</div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
    color: var(--sp-text-1);
  }

  .back-btn {
    background: none;
    border: none;
    color: var(--sp-text-3);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    padding: 0;
    margin-bottom: 1rem;
  }
  .back-btn:hover { color: var(--sp-accent); }

  .profile-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.5rem;
  }

  .header-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
  }

  .v2-badge {
    display: inline-block;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-bold);
    padding: 0.1rem 0.4rem;
    border-radius: var(--sp-radius-xs);
    background: rgba(139,92,246,0.14);
    color: var(--sp-violet);
    margin-left: 0.5rem;
    vertical-align: middle;
  }

  .danger-outline {
    background: transparent;
    border: 1px solid var(--sp-danger);
    color: var(--sp-danger);
    padding: 0.45rem 1rem;
    border-radius: var(--sp-radius-md);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
  }
  .danger-outline:hover { background: rgba(248, 113, 113, 0.14); }

  h1 { margin: 0; font-size: var(--sp-fs-xl); color: var(--sp-text-1); }
  .desc { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.15rem 0 0; }
  .running-hint { margin: 0 0 1rem; font-size: var(--sp-fs-sm); color: var(--sp-accent); }
  .empty { color: var(--sp-text-3); font-style: italic; }

  .error-card {
    background: rgba(248, 113, 113, 0.1);
    border: 1px solid rgba(248, 113, 113, 0.35);
    border-radius: var(--sp-radius-md);
    padding: 1.5rem;
    text-align: center;
    color: var(--sp-danger);
  }

  section { margin-bottom: 1.5rem; }
  h2 { font-size: var(--sp-fs-md); margin: 0 0 0.75rem; color: var(--sp-text-2); }

  /* V2 Run section */
  .run-section {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: 1rem;
    margin-bottom: 1.5rem;
  }

  .run-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  .run-header h2 { margin: 0; }

  .run-status {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-bold);
    text-transform: uppercase;
    padding: 0.15rem 0.5rem;
    border-radius: var(--sp-radius-xs);
  }
  .run-status.pending { background: var(--sp-bg-2); color: var(--sp-text-2); }
  .run-status.running { background: rgba(163,230,53,0.14); color: var(--sp-success); }
  .run-status.succeeded { background: rgba(163,230,53,0.14); color: var(--sp-success); }
  .run-status.failed { background: rgba(248,113,113,0.14); color: var(--sp-danger); }
  .run-status.cancelled { background: rgba(251,191,36,0.14); color: var(--sp-warning); }
  .run-status.partial_success { background: rgba(251,191,36,0.14); color: var(--sp-warning); }

  .run-steps { display: flex; flex-direction: column; gap: 0.3rem; }

  .run-step {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
  }

  .run-step.step-pending { opacity: 0.5; }
  .run-step.step-running { background: var(--sp-accent-soft); }
  .run-step.step-succeeded { color: var(--sp-success); }
  .run-step.step-failed { color: var(--sp-danger); background: rgba(248,113,113,0.08); }
  .run-step.step-skipped { color: var(--sp-warning); opacity: 0.7; }
  .run-step.step-cancelled { color: var(--sp-text-3); text-decoration: line-through; }
  .run-step.step-retrying { color: var(--sp-amber); }

  .run-history-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: var(--sp-radius-sm);
    border: 1px solid var(--sp-border);
    font-size: var(--sp-fs-xs);
  }

  .run-history-row.active {
    border-color: var(--sp-accent-border);
    background: var(--sp-accent-soft);
  }

  .history-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: none;
    border: none;
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    cursor: pointer;
    text-align: left;
    padding: 0;
  }

  .history-id {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
  }

  .history-time {
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
    margin-left: auto;
  }

  .step-icon { width: 1.2em; text-align: center; flex-shrink: 0; }
  .step-info { flex: 1; min-width: 0; }
  .step-id { font-family: var(--sp-font-mono); }
  .step-error { color: var(--sp-danger); font-size: var(--sp-fs-2xs); display: block; }
  .step-controls { display: flex; gap: 0.3rem; flex-shrink: 0; }

  .small-btn {
    padding: 0.15rem 0.4rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-xs);
    background: transparent;
    color: var(--sp-text-2);
    font-size: var(--sp-fs-2xs);
    cursor: pointer;
    font-family: var(--sp-font-sans);
  }
  .small-btn:hover { background: var(--sp-bg-2); color: var(--sp-text-1); }

  .run-diagnostics { margin-top: 0.75rem; padding-top: 0.75rem; border-top: 1px solid var(--sp-border); }
  .run-diagnostics h3 { font-size: var(--sp-fs-sm); margin: 0 0 0.5rem; color: var(--sp-text-2); }

  .diag { margin: 0; font-size: var(--sp-fs-xs); font-family: var(--sp-font-mono); padding: 0.25rem 0.5rem; border-radius: var(--sp-radius-sm); }
  .diag-err { color: var(--sp-danger); background: rgba(248,113,113,0.08); }
  .diag-warn { color: var(--sp-warning); background: rgba(251,191,36,0.08); }
  .diag-info { color: var(--sp-text-3); }
  .diag-step { color: var(--sp-text-3); font-size: var(--sp-fs-2xs); }

  /* Legacy actions */
  .action-list { display: flex; flex-direction: column; gap: 0.35rem; }

  .action-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 0.8rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    box-shadow: var(--sp-shadow-1);
  }
  .action-row.disabled { opacity: 0.4; }

  .action-icon { font-size: 1rem; width: 1.4rem; text-align: center; flex-shrink: 0; }

  .action-info { flex: 1; min-width: 0; }
  .action-label { display: block; font-weight: var(--sp-fw-semibold); font-size: var(--sp-fs-sm); color: var(--sp-text-1); }
  .action-type { font-size: var(--sp-fs-xs); color: var(--sp-text-3); text-transform: uppercase; letter-spacing: 0.03em; margin-right: 0.5rem; }
  .action-detail { font-size: var(--sp-fs-xs); color: var(--sp-text-3); word-break: break-all; }

  .action-controls { display: flex; align-items: center; gap: 0.5rem; flex-shrink: 0; }

  .run-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-bg-2);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    color: var(--sp-success);
    transition: background 0.15s;
  }
  .run-btn:hover:not(:disabled) { background: rgba(163, 230, 53, 0.14); border-color: var(--sp-success); }
  .run-btn:disabled { color: var(--sp-text-3); cursor: default; }

  .toggle { font-size: var(--sp-fs-xs); color: var(--sp-text-3); }
  .toggle.active { color: var(--sp-success); }

  .result-row { font-size: var(--sp-fs-xs); padding: 0.35rem 0.8rem 0.35rem 2.6rem; }
  .result-row.ok { color: var(--sp-success); }
  .result-row.err { color: var(--sp-danger); }
  .result-row.skip { color: var(--sp-warning); }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: none;
    background: var(--sp-accent-strong);
    color: #fff;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s;
  }
  button.primary:hover:not(:disabled) { background: var(--sp-accent); }
  button.primary:disabled { opacity: 0.5; cursor: not-allowed; }

  button.secondary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    cursor: pointer;
  }
  button.secondary:hover:not(:disabled) { background: var(--sp-bg-3); }

  .summary {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.7rem 1rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    margin-top: 0.5rem;
    font-size: var(--sp-fs-sm);
  }
  .summary-ok { color: var(--sp-success); }
  .summary-err { color: var(--sp-danger); }
  .summary-skip { color: var(--sp-warning); }
  .summary-actions { margin-left: auto; display: flex; gap: 0.4rem; }

  /* Modal */
  .modal-overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,0.55);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(2px);
  }
  .modal-content {
    background: var(--sp-bg-0);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    width: 90vw; max-width: 800px;
    max-height: 80vh;
    display: flex; flex-direction: column;
    box-shadow: var(--sp-shadow-3);
    overflow: hidden;
  }
  .modal-header {
    display: flex; justify-content: space-between; align-items: center;
    padding: 0.7rem 1rem;
    border-bottom: 1px solid var(--sp-border);
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
  }
  .modal-close {
    background: none; border: none; color: var(--sp-text-3);
    font-size: 1.2rem; cursor: pointer;
    padding: 0.2rem 0.4rem; border-radius: var(--sp-radius-xs);
  }
  .modal-close:hover { background: var(--sp-bg-2); color: var(--sp-text-1); }
  .modal-body {
    flex: 1; overflow-y: auto; padding: 0.75rem 1rem;
    font-family: var(--sp-font-mono); font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
  }
  .log-line { white-space: pre-wrap; word-break: break-all; line-height: 1.5; }
</style>
