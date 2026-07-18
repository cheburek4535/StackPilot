<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    getDemoProfile,
    listProfiles,
    executeAction,
  } from "$lib/modules/devlauncher/api";
  import type { LaunchProfile, ActionType, ActionStatus } from "$lib/modules/devlauncher/types";

  let profile = $state<LaunchProfile | null>(null);
  let savedProfiles = $state<LaunchProfile[]>([]);
  let actionResults = $state<Map<string, string>>(new Map());
  let runningAll = $state(false);

  onMount(async () => {
    profile = await getDemoProfile();
    savedProfiles = await listProfiles();
  });

  async function runAction(actionId: string) {
    if (!profile) return;
    const action = profile.actions.find((a) => a.id === actionId);
    if (!action) return;

    const result = await executeAction(action);
    const msg = formatResult(result);
    actionResults = new Map(actionResults.set(actionId, msg));
  }

  async function runAll() {
    if (!profile) return;
    runningAll = true;
    const results = new Map<string, string>();
    for (const action of profile.actions) {
      const result = await executeAction(action);
      results.set(action.id, formatResult(result));
    }
    actionResults = results;
    runningAll = false;
  }

  function formatResult(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }

  function actionSummary(act: ActionType): string {
    if ("RunCommand" in act) return act.RunCommand.command;
    if ("OpenUrl" in act) return act.OpenUrl.url;
    if ("OpenApplication" in act) return act.OpenApplication.path;
    if ("WaitForUrl" in act) return `⏳ ${act.WaitForUrl.url}`;
    if ("WaitForPort" in act) return `⏳ ${act.WaitForPort.host}:${act.WaitForPort.port}`;
    if ("Delay" in act) return `⏱ ${act.Delay.seconds}s`;
    if ("ExecuteScript" in act) return `▶ ${act.ExecuteScript.script}`;
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
</script>

<main>
  <div class="hero">
    <h1>DevLauncher</h1>
    <p class="tagline">Development Session Manager &mdash; запускай проект одной кнопкой</p>
    <div class="hero-actions">
      <button class="primary" onclick={runAll} disabled={runningAll || !profile}>
        {runningAll ? "Выполняется..." : "▶ Запустить демо-профиль"}
      </button>
      <button class="secondary" onclick={() => goto("/devlauncher/profiles")}>
        Все профили
      </button>
    </div>
  </div>

  {#if profile}
    <section>
      <div class="section-header">
        <h2>📦 {profile.name}</h2>
        <span class="badge">демо</span>
      </div>
      <p class="desc">{profile.description}</p>

      <div class="action-list">
        {#each profile.actions as action}
          <div class="action-row" class:disabled={!action.enabled}>
            <span class="action-icon">{actionIcon(action.action_type)}</span>
            <div class="action-info">
              <span class="action-label">{action.label}</span>
              <span class="action-detail">{actionSummary(action.action_type)}</span>
            </div>
            <button
              class="run-btn"
              onclick={() => runAction(action.id)}
              disabled={!action.enabled}
              title="Выполнить"
            >
              ▶
            </button>
            {#if actionResults.has(action.id)}
              <span class="action-result">{actionResults.get(action.id)}</span>
            {/if}
          </div>
        {/each}
      </div>
    </section>
  {/if}

  <section>
    <h2>💾 Сохранённые профили</h2>
    {#if savedProfiles.length > 0}
      <div class="profile-cards">
        {#each savedProfiles as p}
          <button class="profile-card" onclick={() => goto(`/devlauncher/profiles/${encodeURIComponent(p.name)}`)}>
            <strong>{p.name}</strong>
            <span class="meta">{p.actions.length} действий</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="empty">Нет сохранённых профилей.</p>
    {/if}
  </section>
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

  .hero {
    text-align: center;
    padding: 2rem 0 1.5rem;
  }

  h1 {
    margin: 0;
    font-size: 1.6rem;
  }

  .tagline {
    color: #888;
    font-size: 1rem;
    margin: 0.25rem 0 1.25rem;
  }

  .hero-actions {
    display: flex;
    gap: 0.75rem;
    justify-content: center;
    flex-wrap: wrap;
  }

  section {
    margin-bottom: 2rem;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    margin-bottom: 0.25rem;
  }

  .section-header h2 {
    margin: 0;
    font-size: 1.1rem;
  }

  .badge {
    font-size: 0.7rem;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    background: #e8eaf6;
    color: #283593;
    font-weight: 600;
    text-transform: uppercase;
  }

  .desc {
    color: #777;
    font-size: 0.9rem;
    margin: 0 0 0.75rem;
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
    padding: 0.6rem 0.8rem;
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

  .action-detail {
    font-size: 0.8rem;
    color: #999;
    word-break: break-all;
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

  .action-result {
    font-size: 0.8rem;
    color: #2e7d32;
    max-width: 220px;
    text-align: right;
    word-break: break-all;
  }

  .profile-cards {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .profile-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: 0.7rem 1rem;
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    background: #fff;
    cursor: pointer;
    text-align: left;
    font-size: 0.9rem;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
    transition: border-color 0.15s;
  }

  .profile-card:hover {
    border-color: #396cd8;
  }

  .meta {
    color: #999;
    font-size: 0.8rem;
  }

  .empty {
    color: #999;
    font-style: italic;
    font-size: 0.9rem;
  }

  button.primary {
    padding: 0.6rem 1.4rem;
    border-radius: 8px;
    border: none;
    background: #396cd8;
    color: #fff;
    font-size: 0.95rem;
    font-weight: 600;
    cursor: pointer;
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
    padding: 0.6rem 1.4rem;
    border-radius: 8px;
    border: 1px solid #ccc;
    background: #fff;
    color: #444;
    font-size: 0.95rem;
    cursor: pointer;
    transition: background 0.15s;
  }

  button.secondary:hover {
    background: #f0f0f0;
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

    .profile-card {
      background: #0f0f0f98;
      border-color: #444;
    }

    .profile-card:hover {
      border-color: #5b8def;
    }

    .run-btn {
      background: #2a2a2a;
      border-color: #555;
    }

    .run-btn:hover:not(:disabled) {
      background: #1b3a1b;
      border-color: #2e7d32;
    }

    .badge {
      background: #1a237e;
      color: #c5cae9;
    }

    button.secondary {
      background: #2a2a2a;
      color: #ccc;
      border-color: #555;
    }

    .tagline {
      color: #888;
    }
  }
</style>
