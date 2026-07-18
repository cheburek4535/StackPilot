<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    getCurrentProject,
    listProcesses,
    killProcess,
    refreshProcess,
  } from "$lib/modules/workspace/api";
  import type { ProjectContext, TrackedProcess } from "$lib/modules/workspace/types";

  let project = $state<ProjectContext | null>(null);
  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  onMount(async () => {
    project = await getCurrentProject();
    if (project) {
      await loadProcesses();
    }
    loading = false;
  });

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch {}
  }

  async function handleKill(id: string) {
    try {
      await killProcess(id);
      await loadProcesses();
    } catch {}
  }

  async function handleRefresh(id: string) {
    try {
      await refreshProcess(id);
      await loadProcesses();
    } catch {}
  }

  function statusLabel(s: TrackedProcess["status"]): string {
    if (s === "Running") return "Running";
    if (s === "Killed") return "Killed";
    if (s === "Crashed") return "Crashed";
    if (typeof s === "object" && "Exited" in s) {
      return s.Exited === 0 ? "Success" : `Exit ${s.Exited}`;
    }
    return "?";
  }

  function statusClass(s: TrackedProcess["status"]): string {
    if (s === "Running") return "running";
    if (s === "Killed") return "killed";
    if (s === "Crashed") return "crashed";
    if (typeof s === "object" && "Exited" in s) {
      return s.Exited === 0 ? "ok" : "err";
    }
    return "";
  }

  function formatDuration(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  }

  function openLogs(id: string) {
    goto(`/processes?log=${id}`);
  }
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime" class="active">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs">Logs</a>
      <a href="/workspace/problems">Problems</a>
      <a href="/workspace/info">Info</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <p class="ws-empty">No project open. <a href="/profiles">Open a profile</a> to see its runtime.</p>
    {:else}
      <div class="header-row">
        <div>
          <h1>Runtime</h1>
          <p class="subtitle">{project.profile_name} — running processes</p>
        </div>
        <button class="refresh-btn" onclick={loadProcesses}>⟳ Refresh</button>
      </div>

      {#if processes.length === 0}
        <p class="ws-empty">No processes. Spawn one from the <a href="/processes">Process Manager</a> or run a profile.</p>
      {:else}
        <div class="proc-list">
          {#each processes as p}
            <div class="proc-card">
              <div class="proc-main">
                <div class="proc-head">
                  <span class="proc-icon">{p.status === "Running" ? "▶" : "⬛"}</span>
                  <strong>{p.label}</strong>
                  <span class="badge {statusClass(p.status)}">{statusLabel(p.status)}</span>
                </div>
                <div class="proc-meta">
                  PID <code>{p.pid}</code>
                  <span class="sep">·</span>
                  {formatDuration(p.duration_secs)}
                  <span class="sep">·</span>
                  started {((Date.now() / 1000) - Number(p.started_at)).toFixed(0)}s ago
                </div>
                {#if p.restarts > 0}
                  <span class="restart-badge">restarts: {p.restarts}</span>
                {/if}
                {#if p.last_error}
                  <div class="proc-error">{p.last_error}</div>
                {/if}
              </div>
              <div class="proc-actions">
                <button class="action-btn" onclick={() => openLogs(p.id)} title="View logs">📋</button>
                <button class="action-btn" onclick={() => handleRefresh(p.id)} title="Refresh">⟳</button>
                <button class="action-btn kill" onclick={() => handleKill(p.id)} disabled={p.status !== "Running"} title="Kill">✕</button>
              </div>
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
  .ws-empty a { color: #396cd8; }

  .header-row { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 1.5rem; }
  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.85rem; margin: 0.15rem 0 0; }
  .refresh-btn { padding: 0.35rem 1rem; border: 1px solid #ccc; border-radius: 6px; background: #fff; color: #444; font-size: 0.82rem; cursor: pointer; }
  .refresh-btn:hover { background: #f0f0f0; }

  .proc-list { display: flex; flex-direction: column; gap: 0.4rem; }
  .proc-card {
    display: flex; justify-content: space-between; align-items: flex-start;
    padding: 0.7rem 0.9rem; background: #fff; border: 1px solid #e0e0e0;
    border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,0.04); gap: 0.75rem;
  }
  .proc-main { flex: 1; min-width: 0; }
  .proc-head { display: flex; align-items: center; gap: 0.4rem; margin-bottom: 0.15rem; }
  .proc-icon { font-size: 0.8rem; }
  .proc-meta { font-size: 0.78rem; color: #888; display: flex; gap: 0.3rem; align-items: center; flex-wrap: wrap; }
  .proc-meta code { background: #f0f0f0; padding: 0.05rem 0.3rem; border-radius: 3px; color: #555; font-size: 0.76rem; }
  .sep { color: #ccc; }

  .badge { font-size: 0.65rem; font-weight: 700; text-transform: uppercase; padding: 0.1rem 0.4rem; border-radius: 4px; letter-spacing: 0.02em; }
  .badge.running { background: #e8f5e9; color: #2e7d32; }
  .badge.ok { background: #e8eaf6; color: #283593; }
  .badge.err { background: #fff3e0; color: #e65100; }
  .badge.killed { background: #fce4ec; color: #c62828; }
  .badge.crashed { background: #ffebee; color: #b71c1c; }

  .restart-badge { font-size: 0.72rem; background: #fff3e0; color: #e65100; padding: 0.05rem 0.4rem; border-radius: 3px; display: inline-block; margin-top: 0.15rem; }

  .proc-error { margin-top: 0.3rem; font-size: 0.76rem; color: #c62828; background: #ffebee; padding: 0.25rem 0.5rem; border-radius: 4px; white-space: pre-wrap; word-break: break-all; max-height: 2.5rem; overflow-y: auto; }

  .proc-actions { display: flex; gap: 0.25rem; flex-shrink: 0; }
  .action-btn {
    padding: 0.25rem 0.5rem; border: 1px solid #e0e0e0; border-radius: 5px;
    background: transparent; color: #888; cursor: pointer; font-size: 0.8rem; transition: all 0.15s;
  }
  .action-btn:hover:not(:disabled) { background: #f5f5f5; color: #333; }
  .action-btn.kill:hover:not(:disabled) { background: #ffebee; color: #c62828; border-color: #ef9a9a; }
  .action-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }

    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .refresh-btn { background: #2a2a2a; color: #ccc; border-color: #555; }
    .refresh-btn:hover { background: #333; }

    .proc-card { background: #0f0f0f98; border-color: #444; }
    .proc-meta code { background: #2a2a2a; color: #aaa; }

    .restart-badge { background: #3e2723; color: #ffb74d; }

    .badge.running { background: #1b3a1b; color: #81c784; }
    .badge.ok { background: #1a237e; color: #c5cae9; }
    .badge.err { background: #3e2723; color: #ffb74d; }
    .badge.killed { background: #3a1a1a; color: #ef9a9a; }
    .badge.crashed { background: #3a1a1a; color: #ef9a9a; }

    .action-btn { border-color: #444; color: #888; }
    .action-btn:hover:not(:disabled) { background: #333; color: #eee; }
    .action-btn.kill:hover:not(:disabled) { background: #3a1a1a; color: #ff7b72; border-color: #da3633; }
  }
</style>
