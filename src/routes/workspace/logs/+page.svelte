<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentProject, listProcesses, getProcessLogs } from "$lib/modules/workspace/api";
  import type { ProjectContext, TrackedProcess, ProcessLogs } from "$lib/modules/workspace/types";

  let project = $state<ProjectContext | null>(null);
  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  let selectedProc = $state<string | null>(null);
  let logs = $state<ProcessLogs | null>(null);
  let logsLoading = $state(false);

  onMount(async () => {
    project = await getCurrentProject();
    if (project) {
      processes = await listProcesses();
    }
    loading = false;
  });

  async function selectProcess(id: string) {
    selectedProc = id;
    logsLoading = true;
    try {
      logs = await getProcessLogs(id);
    } catch {
      logs = null;
    }
    logsLoading = false;
  }

  let allLines = $derived.by(() => {
    if (!logs) return [];
    const out = logs.stdout_lines.map((l) => ({ stream: "stdout" as const, line: l }));
    const err = logs.stderr_lines.map((l) => ({ stream: "stderr" as const, line: l }));
    return [...out, ...err].sort((a, b) => 0); // keep original order
  });
</script>

<div class="workspace-layout">
  <aside class="ws-sidebar">
    <div class="ws-brand">Workspace</div>
    <nav class="ws-nav">
      <a href="/workspace">Overview</a>
      <a href="/workspace/runtime">Runtime</a>
      <a href="/workspace/session">Session</a>
      <a href="/workspace/logs" class="active">Logs</a>
      <a href="/workspace/problems">Problems</a>
      <a href="/workspace/info">Info</a>
      <a href="/workspace/files">Files</a>
    </nav>
  </aside>

  <main class="ws-content">
    {#if loading}
      <p class="ws-empty">Loading...</p>
    {:else if !project}
      <p class="ws-empty">No project open.</p>
    {:else}
      <h1>Logs</h1>
      <p class="subtitle">{project.profile_name} — process output logs</p>

      {#if processes.length === 0}
        <p class="ws-empty">No processes yet.</p>
      {:else}
        <div class="logs-layout">
          <div class="proc-sidebar">
            {#each processes as p}
              <button
                class="proc-btn"
                class:active={selectedProc === p.id}
                onclick={() => selectProcess(p.id)}
              >
                <span class="btn-icon">{p.status === "Running" ? "▶" : "⬛"}</span>
                <span class="btn-label">{p.label}</span>
                <span class="btn-pid">PID {p.pid}</span>
              </button>
            {/each}
          </div>
          <div class="log-viewer">
            {#if !selectedProc}
              <p class="log-hint">Select a process to view its logs</p>
            {:else if logsLoading}
              <p class="log-hint">Loading logs...</p>
            {:else if logs}
              <div class="log-content">
                {#each logs.stdout_lines as line}
                  <pre class="log-line stdout">{line}</pre>
                {/each}
                {#each logs.stderr_lines as line}
                  <pre class="log-line stderr">{line}</pre>
                {/each}
              </div>
            {:else}
              <p class="log-hint">No logs available for this process.</p>
            {/if}
          </div>
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

  .ws-content { flex: 1; padding: 2rem; max-width: 960px; }
  .ws-empty { color: #999; font-style: italic; font-size: 0.85rem; padding: 2rem; text-align: center; background: #fafafa; border-radius: 8px; border: 1px dashed #ddd; }

  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.85rem; margin: 0.15rem 0 1rem; }

  .logs-layout { display: flex; gap: 1rem; height: calc(100vh - 180px); }
  .proc-sidebar { width: 200px; flex-shrink: 0; display: flex; flex-direction: column; gap: 0.3rem; overflow-y: auto; }
  .proc-btn {
    display: flex; align-items: center; gap: 0.35rem; padding: 0.4rem 0.6rem;
    border: 1px solid #e0e0e0; border-radius: 6px; background: #fff;
    cursor: pointer; font-size: 0.8rem; text-align: left; transition: all 0.1s;
  }
  .proc-btn:hover { background: #f5f5f5; }
  .proc-btn.active { background: #e8eaf6; border-color: #283593; }
  .btn-icon { font-size: 0.7rem; }
  .btn-label { flex: 1; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .btn-pid { color: #888; font-size: 0.72rem; flex-shrink: 0; }

  .log-viewer {
    flex: 1; background: #0d1117; border: 1px solid #30363d;
    border-radius: 8px; overflow-y: auto; padding: 0.75rem;
  }
  .log-hint { color: #8b949e; font-style: italic; font-size: 0.85rem; text-align: center; padding: 2rem; }
  .log-content { font-family: 'JetBrains Mono', 'Consolas', monospace; font-size: 0.78rem; line-height: 1.4; }
  .log-line { margin: 0; padding: 0; white-space: pre-wrap; word-break: break-all; color: #c9d1d9; }
  .log-line.stderr { color: #ff7b72; }

  @media (prefers-color-scheme: dark) {
    .ws-sidebar { background: #1e1e1e; border-right-color: #333; }
    .ws-brand { color: #eee; border-bottom-color: #333; }
    .ws-nav a { color: #aaa; }
    .ws-nav a:hover { background: #333; color: #eee; }
    .ws-nav a.active { background: #1a237e; color: #c5cae9; }
    .ws-empty { background: #2a2a2a; border-color: #444; color: #888; }
    .proc-btn { background: #0f0f0f98; border-color: #444; color: #ccc; }
    .proc-btn:hover { background: #333; }
    .proc-btn.active { background: #1a237e; border-color: #5b8def; color: #c5cae9; }
    .btn-pid { color: #aaa; }
  }
</style>
