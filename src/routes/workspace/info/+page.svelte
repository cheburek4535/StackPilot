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
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs">Logs</a>
      <a href="/workspace/problems">Problems</a>
      <a href="/workspace/info" class="active">Info</a>
      <a href="/workspace/files">Files</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <p class="ws-empty">No project open.</p>
    {:else}
      <h1>Project Information</h1>
      <p class="subtitle">{project.profile_name}</p>

      <section class="info-section">
        <div class="info-row"><span class="info-key">Profile name</span><span class="info-val">{project.profile_name}</span></div>
        <div class="info-row"><span class="info-key">Description</span><span class="info-val">{project.description || "—"}</span></div>
        <div class="info-row"><span class="info-key">Project path</span><span class="info-val mono">{project.project_path ?? "—"}</span></div>
        <div class="info-row"><span class="info-key">Opened at</span><span class="info-val">{new Date(Number(project.opened_at) * 1000).toLocaleString()}</span></div>
        <div class="info-row">
          <span class="info-key">Stack</span>
          <span class="info-val">
            {#if project.stack.length > 0}
              {#each project.stack as tech}
                <span class="tag">{tech}</span>
              {/each}
            {:else}—{/if}
          </span>
        </div>
        <div class="info-row"><span class="info-key">Processes</span><span class="info-val">{processes.length}</span></div>
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

  .info-section { background: #fff; border: 1px solid #e0e0e0; border-radius: 10px; overflow: hidden; }
  .info-row { display: flex; padding: 0.6rem 1rem; border-bottom: 1px solid #f0f0f0; font-size: 0.85rem; align-items: center; }
  .info-row:last-child { border-bottom: none; }
  .info-key { width: 130px; flex-shrink: 0; color: #888; font-weight: 500; }
  .info-val { color: #222; display: flex; gap: 0.3rem; flex-wrap: wrap; }
  .info-val.mono { font-family: monospace; font-size: 0.82rem; }

  .tag { font-size: 0.72rem; background: #e8eaf6; color: #283593; padding: 0.1rem 0.45rem; border-radius: 4px; font-weight: 500; }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }
    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .info-section { background: #0f0f0f98; border-color: #444; }
    .info-row { border-bottom-color: #2a2a2a; }
    .info-key { color: #aaa; }
    .info-val { color: #ddd; }
    .tag { background: #1a237e; color: #c5cae9; }
  }
</style>
