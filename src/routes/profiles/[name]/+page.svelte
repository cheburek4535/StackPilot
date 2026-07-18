<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getProfile, getDemoProfile, executeAction, deleteProfile } from "$lib/modules/devlauncher/api";
  import { setCurrentProject } from "$lib/modules/workspace/api";
  import type { LaunchProfile, ActionType, ActionStatus } from "$lib/modules/devlauncher/types";

  let profile = $state<LaunchProfile | null>(null);
  let loading = $state(true);
  let errorMsg = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let runningAll = $state(false);

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
      errorMsg = "Profile name not specified";
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
      errorMsg = `Failed to load profile "${name}": ${e}`;
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
    if (!profile) return;
    runningAll = true;
    const results = new Map<string, string>();
    for (const action of profile.actions) {
      try {
        const result = await executeAction(action);
        results.set(action.id, formatResult(result));
      } catch (e) {
        results.set(action.id, `✗ ${e}`);
      }
    }
    actionResults = results;
    runningAll = false;
  }

  async function retryFailed() {
    if (!profile) return;
    runningAll = true;
    const results = new Map(actionResults);
    for (const id of failedIds) {
      const action = profile.actions.find((a) => a.id === id);
      if (!action) continue;
      try {
        const result = await executeAction(action);
        results.set(action.id, formatResult(result));
      } catch (e) {
        results.set(action.id, `✗ ${e}`);
      }
    }
    actionResults = results;
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
    if ("RunCommand" in act) return "Command";
    if ("OpenUrl" in act) return "URL";
    if ("OpenApplication" in act) return "App";
    if ("WaitForUrl" in act) return "Wait URL";
    if ("WaitForPort" in act) return "Wait Port";
    if ("Delay" in act) return "Delay";
    if ("ExecuteScript" in act) return "Script";
    return "?";
  }

  function goBack() {
    goto("/profiles");
  }

  async function handleDelete() {
    if (!profile) return;
    if (!confirm(`Delete profile "${profile.name}"? This cannot be undone.`)) return;
    try {
      await deleteProfile(profile.name);
      goto("/profiles");
    } catch (e) {
      errorMsg = `Failed to delete: ${e}`;
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
      errorMsg = `Failed to open workspace: ${e}`;
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
  <button class="back-btn" onclick={goBack}>← All profiles</button>

  {#if loading}
    <p class="empty">Loading profile...</p>

  {:else if errorMsg}
    <div class="error-card">
      <p>{errorMsg}</p>
      <button class="secondary" onclick={goBack}>Back to list</button>
    </div>

  {:else if profile}
    <div class="profile-header">
      <div>
        <h1>{profile.name}</h1>
        <p class="desc">{profile.description}</p>
      </div>
      <div class="header-actions">
        <button class="danger-outline" onclick={handleDelete} disabled={runningAll}>
          🗑 Delete
        </button>
        <button class="secondary" onclick={openInWorkspace}>
          Open in Workspace
        </button>
        <button class="primary" onclick={runAll} disabled={runningAll}>
          {runningAll ? "Running..." : "▶ Run all"}
        </button>
      </div>
    </div>

    <section>
      <h2>Actions ({profile.actions.length})</h2>
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
                title="Execute"
              >
                ▶
              </button>
              <span class="toggle" class:active={action.enabled}>
                {action.enabled ? "on" : "off"}
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
            <button class="secondary" onclick={retryFailed}>Retry failed</button>
          {/if}
          <button class="secondary" onclick={clearResults}>Clear results</button>
        </div>
      </div>
    {/if}
  {/if}
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    font-size: 16px;
    color: #0f0f0f;
    background-color: #f6f6f6;
  }

  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
  }

  .back-btn {
    background: none;
    border: none;
    color: #888;
    cursor: pointer;
    font-size: 0.85rem;
    padding: 0;
    margin-bottom: 1rem;
  }

  .back-btn:hover {
    color: #396cd8;
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
    border: 1px solid #e53935;
    color: #e53935;
    padding: 0.45rem 1rem;
    border-radius: 6px;
    cursor: pointer;
    font-size: 0.85rem;
  }

  .danger-outline:hover {
    background: #ffebee;
  }

  h1 {
    margin: 0;
    font-size: 1.3rem;
  }

  .desc {
    color: #888;
    font-size: 0.9rem;
    margin: 0.15rem 0 0;
  }

  .empty {
    color: #999;
    font-style: italic;
  }

  .error-card {
    background: #ffebee;
    border: 1px solid #ef9a9a;
    border-radius: 8px;
    padding: 1.5rem;
    text-align: center;
    color: #c62828;
  }

  section {
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.75rem;
    color: #555;
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
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
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
    font-weight: 600;
    font-size: 0.9rem;
  }

  .action-type {
    font-size: 0.7rem;
    color: #999;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    margin-right: 0.5rem;
  }

  .action-detail {
    font-size: 0.8rem;
    color: #999;
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
    border: 1px solid #ccc;
    border-radius: 6px;
    background: #fff;
    cursor: pointer;
    font-size: 0.85rem;
    color: #2e7d32;
    transition: background 0.15s;
  }

  .run-btn:hover:not(:disabled) {
    background: #e8f5e9;
    border-color: #a5d6a7;
  }

  .run-btn:disabled {
    color: #ccc;
    cursor: default;
  }

  .toggle {
    font-size: 0.75rem;
    color: #aaa;
  }

  .toggle.active {
    color: #2e7d32;
  }

  .result-row {
    font-size: 0.8rem;
    padding: 0.35rem 0.8rem 0.35rem 2.6rem;
  }
  .result-row.ok { color: #2e7d32; }
  .result-row.err { color: #c62828; }
  .result-row.skip { color: #f57f17; }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: 8px;
    border: none;
    background: #396cd8;
    color: #fff;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s;
  }

  button.primary:hover:not(:disabled) {
    background: #2b5ab0;
  }

  button.primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  button.secondary {
    padding: 0.5rem 1.2rem;
    border-radius: 8px;
    border: 1px solid #ccc;
    background: #fff;
    color: #444;
    font-size: 0.9rem;
    cursor: pointer;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    .action-row {
      background: #0f0f0f98;
      border-color: #444;
    }

    .run-btn {
      background: #2a2a2a;
      border-color: #555;
    }

    .run-btn:hover:not(:disabled) {
      background: #1b3a1b;
      border-color: #2e7d32;
    }

    .error-card {
      background: #3a1a1a;
      border-color: #c62828;
      color: #ef9a9a;
    }

    h2 {
      color: #bbb;
    }

    button.secondary {
      background: #2a2a2a;
      color: #ccc;
      border-color: #555;
    }

    .summary {
      background: #2a2a2a;
      border-color: #444;
    }
  }

  .summary {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.7rem 1rem;
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    margin-top: 0.5rem;
    font-size: 0.9rem;
  }

  .summary-ok { color: #2e7d32; }
  .summary-err { color: #c62828; }
  .summary-skip { color: #f57f17; }

  .summary-actions {
    margin-left: auto;
    display: flex;
    gap: 0.4rem;
  }
</style>
