<script lang="ts">
  import { onMount } from "svelte";
  import {
    getCurrentProject,
    getSessionInfo,
    listProcesses,
  } from "$lib/modules/workspace/api";
  import type { ProjectContext, SessionInfo, TrackedProcess } from "$lib/modules/workspace/types";

  let project = $state<ProjectContext | null>(null);
  let session = $state<SessionInfo | null>(null);
  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  onMount(async () => {
    project = await getCurrentProject();
    if (project) {
      session = await getSessionInfo();
      processes = await listProcesses();
    }
    loading = false;
  });

  function formatDuration(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
  }
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session" class="active">Session</a>
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
      <p class="ws-empty">No active session. Open a project in workspace first.</p>
    {:else}
      <h1>Session</h1>
      <p class="subtitle">{project.profile_name} — current development session</p>

      <div class="session-grid">
        <div class="s-card">
          <div class="s-label">Started</div>
          <div class="s-value">{new Date(Number(session?.started_at ?? "0") * 1000).toLocaleTimeString()}</div>
        </div>
        <div class="s-card">
          <div class="s-label">Duration</div>
          <div class="s-value">{session ? formatDuration(session.duration_secs) : "—"}</div>
        </div>
        <div class="s-card">
          <div class="s-label">Processes</div>
          <div class="s-value">{session?.process_count ?? 0}</div>
        </div>
        <div class="s-card">
          <div class="s-label">Errors</div>
          <div class="s-value err">{session?.error_count ?? 0}</div>
        </div>
      </div>

      <section class="ws-section">
        <h2>Project</h2>
        <table class="info-table">
          <tbody>
            <tr><td>Profile</td><td>{project.profile_name}</td></tr>
            <tr><td>Path</td><td>{project.project_path ?? "—"}</td></tr>
            <tr><td>Stack</td><td>{project.stack.join(", ") || "—"}</td></tr>
          </tbody>
        </table>
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
  .ws-empty { color: #999; font-style: italic; font-size: 0.85rem; padding: 2rem; text-align: center; background: #fafafa; border-radius: 8px; border: 1px dashed #ddd; }

  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.85rem; margin: 0.15rem 0 1.5rem; }

  .session-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 1.5rem; }
  .s-card { background: #fff; border: 1px solid #e0e0e0; border-radius: 10px; padding: 1rem; text-align: center; }
  .s-label { font-size: 0.75rem; color: #888; margin-bottom: 0.25rem; }
  .s-value { font-size: 1.2rem; font-weight: 700; color: #222; }
  .s-value.err { color: #c62828; }

  .ws-section { margin-bottom: 1.5rem; }
  .ws-section h2 { font-size: 1rem; color: #444; margin: 0 0 0.75rem; }

  .info-table { width: 100%; border-collapse: collapse; }
  .info-table td { padding: 0.4rem 0.75rem; font-size: 0.85rem; border-bottom: 1px solid #f0f0f0; }
  .info-table td:first-child { color: #888; width: 100px; font-weight: 500; }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }
    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .s-card { background: #0f0f0f98; border-color: #444; }
    .s-value { color: #eee; }
    .info-table td { border-bottom-color: #2a2a2a; }
    .info-table td:first-child { color: #aaa; }
    .ws-section h2 { color: #bbb; }
  }
</style>
