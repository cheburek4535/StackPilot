<script lang="ts">
  import { onMount } from "svelte";
  import {
    listProcesses,
    spawnProcess,
    killProcess,
    refreshProcess,
  } from "$lib/modules/workspace/api";
  import type { TrackedProcess, ProcessStatus } from "$lib/modules/workspace/types";

  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  let command = $state("");
  let args = $state("");
  let label = $state("");
  let spawning = $state(false);

  onMount(() => loadProcesses());

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      console.error("Failed to load processes:", e);
    }
    loading = false;
  }

  async function handleSpawn() {
    if (!command) return;
    spawning = true;
    try {
      const argList = args
        .split(/\s+/)
        .filter((a) => a.length > 0);
      await spawnProcess(command, argList, label || command);
      await loadProcesses();
      command = "";
      args = "";
      label = "";
    } catch (e) {
      console.error("Spawn failed:", e);
      alert(`Spawn failed: ${e}`);
    }
    spawning = false;
  }

  async function handleKill(id: string) {
    try {
      await killProcess(id);
      await loadProcesses();
    } catch (e) {
      console.error("Kill failed:", e);
    }
  }

  async function handleRefresh(id: string) {
    try {
      await refreshProcess(id);
      await loadProcesses();
    } catch (e) {
      console.error("Refresh failed:", e);
    }
  }

  function statusLabel(status: ProcessStatus): string {
    if (status === "Running") return "Running";
    if (status === "Killed") return "Killed";
    if (status === "Crashed") return "Crashed";
    if (typeof status === "object" && "Exited" in status)
      return `Exited (${status.Exited})`;
    return "Unknown";
  }

  function statusClass(status: ProcessStatus): string {
    if (status === "Running") return "running";
    if (status === "Killed") return "killed";
    if (status === "Crashed") return "crashed";
    if (typeof status === "object" && "Exited" in status)
      return status.Exited === 0 ? "exited-ok" : "exited-err";
    return "";
  }

  function startedAgo(timestamp: string): string {
    const secs = Math.floor(
      (Date.now() / 1000 - Number(timestamp))
    );
    if (secs < 60) return `${secs}s ago`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    return `${Math.floor(secs / 3600)}h ago`;
  }
</script>

<main>
  <h1>⚙ Process Manager</h1>
  <p class="subtitle">Spawn, monitor, and kill processes</p>

  <!-- Spawn form -->
  <section class="spawn-card">
    <h2>Spawn Process</h2>
    <div class="spawn-form">
      <label>
        Command *
        <input
          type="text"
          bind:value={command}
          placeholder="npm run dev"
        />
      </label>
      <label>
        Arguments
        <input
          type="text"
          bind:value={args}
          placeholder="--port 3000"
        />
      </label>
      <label>
        Label
        <input
          type="text"
          bind:value={label}
          placeholder="My Dev Server"
        />
      </label>
      <button class="primary" onclick={handleSpawn} disabled={spawning || !command}>
        {spawning ? "Spawning..." : "▶ Spawn"}
      </button>
    </div>
  </section>

  <!-- Process list -->
  <section>
    <div class="list-header">
      <h2>Running Processes ({processes.length})</h2>
      <button class="secondary" onclick={loadProcesses}>Refresh</button>
    </div>

    {#if loading}
      <p class="empty">Loading...</p>
    {:else if processes.length === 0}
      <p class="empty">No processes running.</p>
    {:else}
      <div class="process-list">
        {#each processes as proc}
          <div class="process-row">
            <div class="proc-info">
              <strong>{proc.label}</strong>
              <span class="proc-meta">PID {proc.pid} &middot; {startedAgo(proc.started_at)}</span>
            </div>
            <span class="status-badge {statusClass(proc.status)}">
              {statusLabel(proc.status)}
            </span>
            <div class="proc-actions">
              <button
                class="icon-btn"
                onclick={() => handleRefresh(proc.id)}
                title="Refresh status"
              >⟳</button>
              <button
                class="icon-btn danger"
                onclick={() => handleKill(proc.id)}
                title="Kill process"
              >✕</button>
            </div>
          </div>
        {/each}
      </div>
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

  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.9rem; margin: 0.2rem 0 1.5rem; }

  section { margin-bottom: 1.5rem; }
  h2 { font-size: 1rem; margin: 0 0 0.75rem; color: #444; }

  .spawn-card {
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 10px;
    padding: 1.25rem;
    box-shadow: 0 1px 4px rgba(0,0,0,0.06);
  }

  .spawn-form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .spawn-form label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: #555;
  }

  .spawn-form input {
    padding: 0.5rem 0.75rem;
    border: 1px solid #ccc;
    border-radius: 6px;
    font-size: 0.9rem;
    background: #fafafa;
    transition: border-color 0.15s;
  }

  .spawn-form input:focus {
    outline: none;
    border-color: #396cd8;
    background: #fff;
  }

  .list-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  .list-header h2 { margin: 0; }

  .empty {
    color: #999;
    font-style: italic;
    font-size: 0.9rem;
    text-align: center;
    padding: 2rem;
  }

  .process-list {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .process-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0.8rem;
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
  }

  .proc-info {
    flex: 1;
    min-width: 0;
  }

  .proc-info strong {
    display: block;
    font-size: 0.9rem;
  }

  .proc-meta {
    font-size: 0.75rem;
    color: #999;
  }

  .status-badge {
    font-size: 0.7rem;
    font-weight: 700;
    text-transform: uppercase;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    letter-spacing: 0.03em;
    flex-shrink: 0;
  }

  .status-badge.running { background: #e8f5e9; color: #2e7d32; }
  .status-badge.exited-ok { background: #e8eaf6; color: #283593; }
  .status-badge.exited-err { background: #fff3e0; color: #e65100; }
  .status-badge.killed { background: #fce4ec; color: #c62828; }
  .status-badge.crashed { background: #ffebee; color: #b71c1c; }

  .proc-actions {
    display: flex;
    gap: 0.25rem;
    flex-shrink: 0;
  }

  .icon-btn {
    padding: 0.25rem 0.5rem;
    border: 1px solid #e0e0e0;
    border-radius: 5px;
    background: transparent;
    color: #888;
    cursor: pointer;
    font-size: 0.85rem;
    transition: all 0.15s;
  }

  .icon-btn:hover { background: #f5f5f5; color: #333; }
  .icon-btn.danger:hover { background: #ffebee; color: #c62828; border-color: #ef9a9a; }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: 8px;
    border: none;
    background: #396cd8;
    color: #fff;
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
    align-self: flex-start;
  }

  button.primary:hover:not(:disabled) { background: #2b5ab0; }
  button.primary:disabled { opacity: 0.5; cursor: not-allowed; }

  button.secondary {
    padding: 0.4rem 1rem;
    border-radius: 8px;
    border: 1px solid #ccc;
    background: #fff;
    color: #444;
    font-size: 0.85rem;
    cursor: pointer;
    transition: background 0.15s;
  }

  button.secondary:hover { background: #f0f0f0; }

  @media (prefers-color-scheme: dark) {
    :root { color: #f6f6f6; background-color: #2f2f2f; }

    .spawn-card { background: #0f0f0f98; border-color: #444; }
    .spawn-form label { color: #aaa; }
    .spawn-form input { background: #2a2a2a; border-color: #555; color: #eee; }
    .spawn-form input:focus { border-color: #5b8def; background: #333; }

    .process-row { background: #0f0f0f98; border-color: #444; }

    .status-badge.running { background: #1b3a1b; color: #81c784; }
    .status-badge.exited-ok { background: #1a237e; color: #c5cae9; }
    .status-badge.exited-err { background: #3e2723; color: #ffb74d; }
    .status-badge.killed { background: #3a1a1a; color: #ef9a9a; }
    .status-badge.crashed { background: #3a1a1a; color: #ef9a9a; }

    .icon-btn { border-color: #444; color: #888; }
    .icon-btn:hover { background: #333; color: #eee; }
    .icon-btn.danger:hover { background: #3a1a1a; color: #ef9a9a; border-color: #ef9a9a; }

    button.secondary { background: #2a2a2a; color: #ccc; border-color: #555; }
  }
</style>
