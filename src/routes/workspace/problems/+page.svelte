<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentProject, listProcesses } from "$lib/modules/workspace/api";
  import type { ProjectContext, TrackedProcess } from "$lib/modules/workspace/types";

  let project = $state<ProjectContext | null>(null);
  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  onMount(async () => {
    project = await getCurrentProject();
    if (project) {
      processes = await listProcesses();
    }
    loading = false;
  });

  let problems = $derived(
    processes.filter((p) => {
      if (p.status === "Crashed") return true;
      if (typeof p.status === "object" && "Exited" in p.status && p.status.Exited !== 0) return true;
      if (p.last_error) return true;
      return false;
    }),
  );

  function statusLabel(s: TrackedProcess["status"]): string {
    if (s === "Crashed") return "Crashed";
    if (typeof s === "object" && "Exited" in s) return `Exit ${s.Exited}`;
    return "—";
  }
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs">Logs</a>
      <a href="/workspace/problems" class="active">Problems</a>
      <a href="/workspace/info">Info</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <p class="ws-empty">No project open.</p>
    {:else}
      <h1>Problems</h1>
      <p class="subtitle">{project.profile_name} — errors and issues</p>

      {#if problems.length === 0}
        <div class="all-clear">
          <span class="clear-icon">✓</span>
          <p>No problems detected.</p>
        </div>
      {:else}
        <div class="problem-list">
          {#each problems as p}
            <div class="problem-card">
              <div class="problem-head">
                <span class="problem-status">{statusLabel(p.status)}</span>
                <strong>{p.label}</strong>
                <span class="problem-pid">PID {p.pid}</span>
              </div>
              {#if p.last_error}
                <div class="problem-error">{p.last_error}</div>
              {:else}
                <div class="problem-error">Process exited with non-zero code.</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </main>
</div>

<style>
  .workspace-layout { display: flex; min-height: calc(100vh - 49px); }
  .ws-sidebar {
    width: 200px; flex-shrink: 0; background: #fff;
    border-right: 1px solid #e0e0e0; padding: 1.25rem 0;
  }
  .ws-brand { font-weight: 700; font-size: 0.9rem; padding: 0 1.25rem 0.75rem; color: #222; border-bottom: 1px solid #eee; margin-bottom: 0.5rem; }
  .ws-nav { display: flex; flex-direction: column; gap: 0.15rem; }
  .ws-nav a { display: block; padding: 0.4rem 1.25rem; text-decoration: none; color: #555; font-size: 0.85rem; border-left: 3px solid transparent; transition: all 0.1s; }
  .ws-nav a:hover { background: #f5f5f5; color: #222; }
  .ws-nav a.active { background: #e8eaf6; color: #283593; border-left-color: #283593; font-weight: 600; }

  .ws-content { flex: 1; padding: 2rem; max-width: 860px; }
  .ws-empty { color: #999; font-style: italic; font-size: 0.85rem; padding: 2rem; text-align: center; background: #fafafa; border-radius: 8px; border: 1px dashed #ddd; }

  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.85rem; margin: 0.15rem 0 1.5rem; }

  .all-clear { text-align: center; padding: 3rem; color: #2e7d32; }
  .clear-icon { font-size: 2rem; display: block; margin-bottom: 0.5rem; }
  .all-clear p { font-size: 0.9rem; margin: 0; }

  .problem-list { display: flex; flex-direction: column; gap: 0.4rem; }
  .problem-card {
    background: #fff; border: 1px solid #ef9a9a; border-radius: 8px;
    padding: 0.7rem 0.9rem; box-shadow: 0 1px 3px rgba(0,0,0,0.04);
  }
  .problem-head { display: flex; align-items: center; gap: 0.4rem; margin-bottom: 0.3rem; }
  .problem-status {
    font-size: 0.65rem; font-weight: 700; text-transform: uppercase;
    padding: 0.1rem 0.4rem; border-radius: 4px;
    background: #ffebee; color: #b71c1c;
  }
  .problem-pid { color: #888; font-size: 0.75rem; margin-left: auto; }
  .problem-error {
    font-size: 0.78rem; color: #c62828; background: #ffebee;
    padding: 0.3rem 0.6rem; border-radius: 4px;
    white-space: pre-wrap; word-break: break-all; max-height: 4rem; overflow-y: auto;
  }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }
    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .all-clear { color: #81c784; }
    .problem-card { background: #0f0f0f98; border-color: #c62828; }
    .problem-status { background: #3a1a1a; color: #ef9a9a; }
    .problem-pid { color: #aaa; }
    .problem-error { background: #3a1a1a; color: #ff7b72; }
  }
</style>
