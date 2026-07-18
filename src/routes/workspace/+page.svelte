<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    getCurrentProject,
    clearCurrentProject,
    listProcesses,
    getSessionInfo,
  } from "$lib/modules/workspace/api";
  import type {
    ProjectContext,
    TrackedProcess,
    SessionInfo,
  } from "$lib/modules/workspace/types";

  let project = $state<ProjectContext | null>(null);
  let processes = $state<TrackedProcess[]>([]);
  let session = $state<SessionInfo | null>(null);
  let loading = $state(true);

  onMount(async () => {
    project = await getCurrentProject();
    if (project) {
      processes = await listProcesses();
      session = await getSessionInfo();
    }
    loading = false;
  });

  let runningCount = $derived(processes.filter((p) => p.status === "Running").length);
  let errorCount = $derived(
    processes.filter((p) => {
      if (p.status === "Crashed") return true;
      if (typeof p.status === "object" && "Exited" in p.status && p.status.Exited !== 0) return true;
      return false;
    }).length,
  );

  function formatDuration(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  }

  function closeProject() {
    clearCurrentProject();
    goto("/");
  }
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace" class="active">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs">Logs</a>
      <a href="/workspace/problems">Problems</a>
      <a href="/workspace/info">Info</a>
      <a href="/workspace/files">Files</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <div class="no-project">
        <h1>Welcome to Workspace</h1>
        <p>Open a project to see its overview here.</p>
        <div class="quick-actions">
          <a href="/profiles" class="qa-btn">Browse profiles</a>
          <a href="/analyze" class="qa-btn">Analyze a project</a>
          <a href="/processes" class="qa-btn">Process manager</a>
        </div>
      </div>
    {:else}
      <div class="project-bar">
        <div class="project-info">
          <h1>{project.profile_name}</h1>
          <span class="project-path">{project.project_path ?? "—"}</span>
          <p class="project-desc">{project.description}</p>
          {#if project.stack.length > 0}
            <div class="stack-tags">
              {#each project.stack as tech}
                <span class="tag">{tech}</span>
              {/each}
            </div>
          {/if}
        </div>
        <button class="close-btn" onclick={closeProject}>✕ Close workspace</button>
      </div>

      <div class="overview-grid">
        <div class="stat-card">
          <div class="stat-value">{runningCount}/{processes.length}</div>
          <div class="stat-label">Processes</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">{session ? formatDuration(session.duration_secs) : "—"}</div>
          <div class="stat-label">Session uptime</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">{errorCount}</div>
          <div class="stat-label">Errors</div>
        </div>
        <div class="stat-card">
          <div class="stat-value">{processes.reduce((s, p) => s + p.restarts, 0)}</div>
          <div class="stat-label">Restarts</div>
        </div>
      </div>

      <section class="ws-section">
        <h2>Running Processes</h2>
        {#if runningCount === 0}
          <p class="ws-empty">No processes running. Go to <a href="/workspace/runtime">Runtime</a> to spawn or run a profile.</p>
        {:else}
          <div class="mini-process-list">
            {#each processes.filter((p) => p.status === "Running") as proc}
              <div class="mini-proc">
                <span class="mini-proc-icon">▶</span>
                <span class="mini-proc-label">{proc.label}</span>
                <span class="mini-proc-pid">PID {proc.pid}</span>
                <span class="mini-proc-time">{formatDuration(proc.duration_secs)}</span>
                <a href="/workspace/runtime" class="mini-proc-link">→</a>
              </div>
            {/each}
          </div>
        {/if}
      </section>
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
  .ws-empty { color: #999; font-style: italic; font-size: 0.85rem; padding: 1.5rem; text-align: center; background: #fafafa; border-radius: 8px; border: 1px dashed #ddd; }
  .ws-empty a { color: #396cd8; }

  .no-project { text-align: center; padding: 3rem 1rem; }
  .no-project h1 { margin: 0 0 0.5rem; font-size: 1.3rem; }
  .no-project p { color: #888; font-size: 0.9rem; margin: 0 0 1.5rem; }

  .project-bar {
    display: flex; justify-content: space-between; align-items: flex-start;
    margin-bottom: 1.5rem; gap: 1rem;
  }
  .project-info h1 { margin: 0; font-size: 1.3rem; }
  .project-path { color: #888; font-size: 0.8rem; font-family: monospace; display: block; margin: 0.15rem 0; }
  .project-desc { color: #666; font-size: 0.85rem; margin: 0.25rem 0 0; }
  .stack-tags { display: flex; gap: 0.3rem; margin-top: 0.4rem; flex-wrap: wrap; }
  .tag { font-size: 0.72rem; background: #e8eaf6; color: #283593; padding: 0.1rem 0.45rem; border-radius: 4px; font-weight: 500; }

  .close-btn {
    padding: 0.35rem 0.8rem; border: 1px solid #e0e0e0; border-radius: 6px;
    background: transparent; color: #888; cursor: pointer; font-size: 0.8rem; white-space: nowrap;
    transition: all 0.15s;
  }
  .close-btn:hover { background: #ffebee; color: #c62828; border-color: #ef9a9a; }

  .overview-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 2rem; }
  .stat-card { background: #fff; border: 1px solid #e0e0e0; border-radius: 10px; padding: 1.25rem; text-align: center; box-shadow: 0 1px 3px rgba(0,0,0,0.04); }
  .stat-value { font-size: 1.5rem; font-weight: 700; color: #222; }
  .stat-label { font-size: 0.78rem; color: #888; margin-top: 0.2rem; }

  .ws-section { margin-bottom: 1.5rem; }
  .ws-section h2 { font-size: 1rem; color: #444; margin: 0 0 0.75rem; }

  .mini-process-list { display: flex; flex-direction: column; gap: 0.3rem; }
  .mini-proc {
    display: flex; align-items: center; gap: 0.5rem;
    padding: 0.5rem 0.8rem; background: #fff; border: 1px solid #e0e0e0;
    border-radius: 8px; font-size: 0.85rem;
  }
  .mini-proc-icon { font-size: 0.75rem; color: #2e7d32; }
  .mini-proc-label { font-weight: 600; flex: 1; }
  .mini-proc-pid { color: #888; font-size: 0.78rem; }
  .mini-proc-time { color: #888; font-size: 0.78rem; }
  .mini-proc-link { text-decoration: none; color: #396cd8; font-size: 1rem; }

  .quick-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; justify-content: center; }
  .qa-btn { padding: 0.5rem 1rem; border: 1px solid #e0e0e0; border-radius: 8px; background: #fff; color: #444; text-decoration: none; font-size: 0.85rem; transition: all 0.15s; }
  .qa-btn:hover { background: #f0f0f0; border-color: #bbb; }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }

    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .no-project p { color: #aaa; }

    .stat-card { background: #0f0f0f98; border-color: #444; }
    .stat-value { color: #eee; }

    .project-path { color: #aaa; }
    .project-desc { color: #999; }
    .tag { background: #1a237e; color: #c5cae9; }

    .ws-section h2 { color: #bbb; }
    .mini-proc { background: #0f0f0f98; border-color: #444; }
    .mini-proc-pid { color: #aaa; }
    .mini-proc-time { color: #aaa; }

    .qa-btn { background: #2a2a2a; border-color: #444; color: #ccc; }
    .qa-btn:hover { background: #333; border-color: #666; }
  }
</style>
