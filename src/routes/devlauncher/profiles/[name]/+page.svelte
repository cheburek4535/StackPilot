<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getProfile, getDemoProfile, executeAction, deleteProfile, runProfile } from "$lib/modules/devlauncher/api";
  import { setCurrentProject } from "$lib/modules/workspace/api";
  import type { LaunchProfile, ActionType, ActionStatus } from "$lib/modules/devlauncher/types";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let profile = $state<LaunchProfile | null>(null);
  let loading = $state(true);
  let errorMsg = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let runningAll = $state(false);
  let currentAction = $state<string | null>(null);

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

  let profileName = $derived($page.params.name);

  onMount(async () => {
    await loadProfile();
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
    const results = new Map<string, string>();
    try {
      const profileResults = await runProfile(profile);
      for (const [actionId, status] of profileResults) {
        results.set(actionId, formatResult(status));
      }
    } catch (e) {
      // Fallback: execute actions one by one so the user still sees results.
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

    // Navigate to the process manager so the user sees the running processes
    // with their logs immediately.
    goto("/devlauncher/processes");
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
      </div>
      <div class="header-actions">
        <button class="danger-outline" onclick={handleDelete} disabled={runningAll}>
          {i18n.t("devl.delete_profile") as TranslationKey}
        </button>
        <button class="secondary" onclick={openInWorkspace}>
          {i18n.t("devl.open_in_workspace") as TranslationKey}
        </button>
        <button class="primary" onclick={runAll} disabled={runningAll}>
          {runningAll ? (i18n.t("devl.running") as TranslationKey) : (i18n.t("devl.run_all") as TranslationKey)}
        </button>
      </div>
    </div>

    {#if currentAction}
      <p class="running-hint">▶ {currentAction}…</p>
    {/if}

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

  .back-btn:hover {
    color: var(--sp-accent);
  }

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

  .danger-outline:hover {
    background: rgba(248, 113, 113, 0.14);
  }

  h1 {
    margin: 0;
    font-size: var(--sp-fs-xl);
    color: var(--sp-text-1);
  }

  .desc {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    margin: 0.15rem 0 0;
  }

  .running-hint {
    margin: 0 0 1rem;
    font-size: var(--sp-fs-sm);
    color: var(--sp-accent);
  }

  .empty {
    color: var(--sp-text-3);
    font-style: italic;
  }

  .error-card {
    background: rgba(248, 113, 113, 0.1);
    border: 1px solid rgba(248, 113, 113, 0.35);
    border-radius: var(--sp-radius-md);
    padding: 1.5rem;
    text-align: center;
    color: var(--sp-danger);
  }

  section {
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: var(--sp-fs-md);
    margin: 0 0 0.75rem;
    color: var(--sp-text-2);
  }

  .action-list {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

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

  .action-row.disabled {
    opacity: 0.4;
  }

  .action-icon {
    font-size: 1rem;
    width: 1.4rem;
    text-align: center;
    flex-shrink: 0;
  }

  .action-info {
    flex: 1;
    min-width: 0;
  }

  .action-label {
    display: block;
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
  }

  .action-type {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    text-transform: uppercase;
    letter-spacing: 0.03em;
    margin-right: 0.5rem;
  }

  .action-detail {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .action-controls {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-shrink: 0;
  }

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

  .run-btn:hover:not(:disabled) {
    background: rgba(163, 230, 53, 0.14);
    border-color: var(--sp-success);
  }

  .run-btn:disabled {
    color: var(--sp-text-3);
    cursor: default;
  }

  .toggle {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .toggle.active {
    color: var(--sp-success);
  }

  .result-row {
    font-size: var(--sp-fs-xs);
    padding: 0.35rem 0.8rem 0.35rem 2.6rem;
  }
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

  button.primary:hover:not(:disabled) {
    background: var(--sp-accent);
  }

  button.primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

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

  button.secondary:hover:not(:disabled) {
    background: var(--sp-bg-3);
  }

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

  .summary-actions {
    margin-left: auto;
    display: flex;
    gap: 0.4rem;
  }
</style>
