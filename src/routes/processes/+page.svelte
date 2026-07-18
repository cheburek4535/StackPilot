<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Terminal } from "@xterm/xterm";
  import "@xterm/xterm/css/xterm.css";
  import {
    listProcesses,
    spawnProcess,
    killProcess,
    refreshProcess,
    getProcessLogs,
  } from "$lib/modules/workspace/api";
  import type {
    TrackedProcess,
    ProcessStatus,
    ProcessLogs,
    ProcessOutputEvent,
    ProcessStatusEvent,
  } from "$lib/modules/workspace/types";

  let processes = $state<TrackedProcess[]>([]);
  let loading = $state(true);

  let command = $state("");
  let argsStr = $state("");
  let label = $state("");
  let spawning = $state(false);
  let errorMsg = $state("");

  let logProcessId = $state<string | null>(null);
  let terminalEl = $state<HTMLDivElement | null>(null);
  let terminal = $state<Terminal | null>(null);
  let logModalOpen = $state(false);

  let autoRefreshId: ReturnType<typeof setInterval> | null = null;
  let unlistenOutput: UnlistenFn | null = null;
  let unlistenStatus: UnlistenFn | null = null;

  let resultMsg = $state("");
  let resultType = $state<"ok" | "err" | "">("");

  let logProcessName = $derived(() => {
    if (!logProcessId) return "";
    const p = processes.find((pr) => pr.id === logProcessId);
    return p?.label ?? logProcessId;
  });

  onMount(async () => {
    await loadProcesses();

    unlistenOutput = await listen<ProcessOutputEvent>("process-output", (event) => {
      const { process_id, stream, line } = event.payload;
      if (logProcessId === process_id && terminal) {
        const prefix = stream === "stderr" ? "\x1b[31m" : "";
        const suffix = stream === "stderr" ? "\x1b[0m" : "";
        terminal.writeln(`${prefix}${line}${suffix}`);
      }
    });

    unlistenStatus = await listen<ProcessStatusEvent>("process-status", (event) => {
      const { process_id, status, error } = event.payload;
      const idx = processes.findIndex((p) => p.id === process_id);
      if (idx >= 0) {
        const updated = { ...processes[idx], status, last_error: error ?? null };
        processes = [...processes.slice(0, idx), updated, ...processes.slice(idx + 1)];
      }
    });

    autoRefreshId = setInterval(() => refreshAllStatuses(), 3000);
  });

  onDestroy(() => {
    if (autoRefreshId) clearInterval(autoRefreshId);
    unlistenOutput?.();
    unlistenStatus?.();
    terminal?.dispose();
  });

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      errorMsg = `Failed to load processes: ${e}`;
    }
    loading = false;
  }

  async function refreshAllStatuses() {
    const updated: TrackedProcess[] = [];
    for (const proc of processes) {
      try {
        await refreshProcess(proc.id);
      } catch {
        // process might be gone
      }
    }
    try {
      processes = await listProcesses();
    } catch {
      // ignore
    }
  }

  async function handleSpawn() {
    if (!command) return;
    spawning = true;
    errorMsg = "";
    resultMsg = "";
    resultType = "";
    try {
      const argList = argsStr
        .split(/\s+/)
        .filter((a) => a.length > 0);
      await spawnProcess(command, argList, label || command);
      await loadProcesses();
      command = "";
      argsStr = "";
      label = "";
      resultMsg = `✓ Spawned successfully`;
      resultType = "ok";
    } catch (e) {
      resultMsg = `✗ Spawn failed: ${e}`;
      resultType = "err";
    }
    spawning = false;
    setTimeout(() => { resultMsg = ""; resultType = ""; }, 4000);
  }

  async function handleKill(id: string) {
    const proc = processes.find((p) => p.id === id);
    if (!proc) return;
    try {
      await killProcess(id);
      await loadProcesses();
    } catch (e) {
      errorMsg = `Kill failed: ${e}`;
    }
  }

  async function handleRefresh(id: string) {
    try {
      await refreshProcess(id);
      await loadProcesses();
    } catch (e) {
      errorMsg = `Refresh failed: ${e}`;
    }
  }

  async function openLogs(id: string) {
    logProcessId = id;
    logModalOpen = true;

    // Wait for DOM to render the terminal div
    await tick();

    if (terminalEl) {
      if (terminal) terminal.dispose();
      const term = new Terminal({
        cursorBlink: true,
        fontSize: 13,
        fontFamily: "'JetBrains Mono', 'Cascadia Code', 'Fira Code', 'Consolas', monospace",
        theme: {
          background: "#0d1117",
          foreground: "#c9d1d9",
          cursor: "#58a6ff",
          selectionBackground: "#264f78",
          black: "#484f58",
          red: "#ff7b72",
          green: "#3fb950",
          yellow: "#d29922",
          blue: "#58a6ff",
          magenta: "#bc8cff",
          cyan: "#39c5cf",
          white: "#b1bac4",
          brightBlack: "#6e7681",
          brightRed: "#ffa198",
          brightGreen: "#56d364",
          brightYellow: "#e3b341",
          brightBlue: "#79c0ff",
          brightMagenta: "#d2a8ff",
          brightCyan: "#56d4dd",
          brightWhite: "#f0f6fc",
        },
        allowTransparency: true,
      });
      term.open(terminalEl);
      terminal = term;

      // Write existing logs
      try {
        const logs = await getProcessLogs(id);
        for (const line of logs.stdout_lines) {
          term.writeln(line);
        }
        for (const line of logs.stderr_lines) {
          term.writeln(`\x1b[31m${line}\x1b[0m`);
        }
      } catch (e) {
        term.writeln(`\x1b[33mFailed to load logs: ${e}\x1b[0m`);
      }
    }
  }

  async function tick() {
    return new Promise((resolve) => requestAnimationFrame(resolve));
  }

  function closeLogs() {
    logModalOpen = false;
    logProcessId = null;
    terminal?.dispose();
    terminal = null;
  }

  function statusLabel(status: ProcessStatus): string {
    if (status === "Running") return "Running";
    if (status === "Killed") return "Killed";
    if (status === "Crashed") return "Crashed";
    if (typeof status === "object" && "Exited" in status) {
      const code = (status as { Exited: number }).Exited;
      return code === 0 ? "Success" : `Failed (${code})`;
    }
    return "Unknown";
  }

  function statusClass(status: ProcessStatus): string {
    if (status === "Running") return "running";
    if (status === "Killed") return "killed";
    if (status === "Crashed") return "crashed";
    if (typeof status === "object" && "Exited" in status) {
      const code = (status as { Exited: number }).Exited;
      return code === 0 ? "exited-ok" : "exited-err";
    }
    return "";
  }

  function formatDuration(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  }

  function formatStarted(timestamp: string): string {
    if (!timestamp) return "—";
    const secs = Math.floor(Date.now() / 1000 - Number(timestamp));
    if (secs < 60) return `${secs}s ago`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
    return `${Math.floor(secs / 86400)}d ago`;
  }

  function copyCommand(proc: TrackedProcess) {
    navigator.clipboard.writeText(`${proc.label} (PID: ${proc.pid})`);
  }
</script>

<main>
  <div class="page-header">
    <div>
      <h1>⚙ Process Manager</h1>
      <p class="subtitle">Full control over running processes — spawn, monitor, view logs, terminate</p>
    </div>
    <button class="refresh-btn" onclick={refreshAllStatuses} title="Refresh all">
      ⟳ Refresh
    </button>
  </div>

  {#if errorMsg}
    <div class="msg err">{errorMsg}</div>
  {/if}

  {#if resultMsg}
    <div class="msg {resultType}">{resultMsg}</div>
  {/if}

  <!-- Spawn form -->
  <section class="card spawn-card">
    <h2>▶ Spawn Process</h2>
    <div class="spawn-form">
      <div class="field-row">
        <div class="field flex-2">
          <label for="cmd-input">Command *</label>
          <input id="cmd-input" type="text" bind:value={command} placeholder="npm run dev" />
        </div>
        <div class="field flex-1">
          <label for="label-input">Label</label>
          <input id="label-input" type="text" bind:value={label} placeholder="Dev Server" />
        </div>
      </div>
      <div class="field">
        <label for="args-input">Arguments</label>
        <input id="args-input" type="text" bind:value={argsStr} placeholder="--port 3000 --mode dev" />
      </div>
      <button class="primary" onclick={handleSpawn} disabled={spawning || !command}>
        {spawning ? "Spawning..." : "▶ Spawn"}
      </button>
    </div>
  </section>

  <!-- Process list -->
  <section>
    <h2>Processes ({processes.length})</h2>

    {#if loading}
      <p class="empty">Loading...</p>
    {:else if processes.length === 0}
      <div class="empty-state">
        <p class="empty">No processes running.</p>
        <p class="hint">Use the form above to start a process.</p>
      </div>
    {:else}
      <div class="process-list">
        {#each processes as proc (proc.id)}
          <div class="process-card" class:exited={proc.status !== "Running"}>
            <div class="card-top">
              <div class="proc-main">
                <div class="proc-label-row">
                  <span class="proc-icon">
                    {#if proc.status === "Running"}▶{:else}⬛{/if}
                  </span>
                  <strong class="proc-label">{proc.label}</strong>
                  <span class="status-badge {statusClass(proc.status)}">
                    {statusLabel(proc.status)}
                  </span>
                </div>
                <div class="proc-meta-row">
                  <span class="meta-item">PID <code>{proc.pid}</code></span>
                  <span class="meta-item sep">·</span>
                  <span class="meta-item">{formatDuration(proc.duration_secs)}</span>
                  <span class="meta-item sep">·</span>
                  <span class="meta-item">started {formatStarted(proc.started_at)}</span>
                  {#if proc.restarts > 0}
                    <span class="meta-item sep">·</span>
                    <span class="meta-item restart-count">restarts: {proc.restarts}</span>
                  {/if}
                </div>
                {#if proc.last_error}
                  <div class="proc-error">{proc.last_error}</div>
                {/if}
              </div>
              <div class="proc-actions">
                <button class="action-btn logs" onclick={() => openLogs(proc.id)} title="View logs">
                  📋 Logs
                </button>
                <button class="action-btn refresh" onclick={() => handleRefresh(proc.id)} title="Refresh status">
                  ⟳
                </button>
                <button
                  class="action-btn kill"
                  onclick={() => handleKill(proc.id)}
                  disabled={proc.status !== "Running"}
                  title="Kill process"
                >
                  ✕ Kill
                </button>
              </div>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</main>

<!-- Log Viewer Modal -->
{#if logModalOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="modal-overlay" onclick={closeLogs} role="presentation">
    <!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
    <div class="modal-content" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Process logs">
      <div class="modal-header">
        <div class="modal-title">
          <span class="modal-icon">📋</span>
          <span>Logs: {logProcessName()}</span>
          <span class="modal-pid">PID {processes.find(p => p.id === logProcessId)?.pid ?? "—"}</span>
        </div>
        <div class="modal-actions">
          <button class="modal-close" onclick={closeLogs}>✕</button>
        </div>
      </div>
      <div class="terminal-wrapper" bind:this={terminalEl}></div>
      <div class="modal-footer">
        <span class="footer-hint">Real-time output · stderr shown in red</span>
        <button class="secondary" onclick={closeLogs}>Close</button>
      </div>
    </div>
  </div>
{/if}

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    font-size: 16px;
    color: #0f0f0f;
    background-color: #f6f6f6;
  }

  main {
    max-width: 860px;
    margin: 0 auto;
    padding: 2rem;
  }

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1.5rem;
    gap: 1rem;
  }

  h1 { margin: 0; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.85rem; margin: 0.2rem 0 0; }

  .refresh-btn {
    padding: 0.4rem 1rem;
    border: 1px solid #ccc;
    border-radius: 6px;
    background: #fff;
    color: #444;
    font-size: 0.85rem;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s;
  }
  .refresh-btn:hover { background: #f0f0f0; }

  .msg {
    padding: 0.5rem 1rem;
    border-radius: 6px;
    margin-bottom: 1rem;
    font-size: 0.85rem;
  }
  .msg.ok { background: #e8f5e9; color: #2e7d32; border: 1px solid #a5d6a7; }
  .msg.err { background: #ffebee; color: #c62828; border: 1px solid #ef9a9a; }

  .card {
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 12px;
    padding: 1.25rem;
    box-shadow: 0 1px 4px rgba(0,0,0,0.06);
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.75rem;
    color: #444;
  }

  .spawn-form {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .field-row {
    display: flex;
    gap: 0.75rem;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .flex-2 { flex: 2; }
  .flex-1 { flex: 1; }

  .field label {
    font-size: 0.8rem;
    font-weight: 500;
    color: #555;
  }

  .field input {
    padding: 0.5rem 0.75rem;
    border: 1px solid #ccc;
    border-radius: 6px;
    font-size: 0.9rem;
    background: #fafafa;
    transition: border-color 0.15s;
  }

  .field input:focus {
    outline: none;
    border-color: #396cd8;
    background: #fff;
  }

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

  .empty-state {
    text-align: center;
    padding: 2rem 1rem;
  }

  .empty {
    color: #999;
    font-style: italic;
    font-size: 0.9rem;
    text-align: center;
    padding: 2rem;
  }

  .hint {
    color: #aaa;
    font-size: 0.85rem;
    margin: 0.5rem 0 0;
  }

  .process-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .process-card {
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 10px;
    padding: 0.85rem 1rem;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
    transition: border-color 0.15s;
  }
  .process-card:hover { border-color: #bbb; }
  .process-card.exited { opacity: 0.75; }

  .card-top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
  }

  .proc-main {
    flex: 1;
    min-width: 0;
  }

  .proc-label-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .proc-icon {
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .proc-label {
    font-size: 0.95rem;
    font-weight: 600;
  }

  .proc-meta-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.8rem;
    color: #888;
    flex-wrap: wrap;
  }

  .meta-item code {
    font-size: 0.78rem;
    background: #f0f0f0;
    padding: 0.05rem 0.35rem;
    border-radius: 3px;
    color: #555;
  }

  .meta-item.sep { color: #ccc; }

  .restart-count {
    background: #fff3e0;
    color: #e65100;
    padding: 0.05rem 0.4rem;
    border-radius: 3px;
    font-size: 0.75rem;
  }

  .proc-error {
    margin-top: 0.35rem;
    font-size: 0.78rem;
    color: #c62828;
    background: #ffebee;
    padding: 0.3rem 0.6rem;
    border-radius: 4px;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 3rem;
    overflow-y: auto;
  }

  .status-badge {
    font-size: 0.68rem;
    font-weight: 700;
    text-transform: uppercase;
    padding: 0.15rem 0.45rem;
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
    gap: 0.35rem;
    flex-shrink: 0;
    align-items: center;
  }

  .action-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid #e0e0e0;
    border-radius: 6px;
    background: transparent;
    color: #666;
    cursor: pointer;
    font-size: 0.8rem;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .action-btn:hover:not(:disabled) { background: #f5f5f5; color: #333; border-color: #bbb; }
  .action-btn.logs:hover { background: #e3f2fd; color: #1565c0; border-color: #90caf9; }
  .action-btn.refresh:hover { background: #f3e5f5; color: #7b1fa2; border-color: #ce93d8; }
  .action-btn.kill:hover:not(:disabled) { background: #ffebee; color: #c62828; border-color: #ef9a9a; }
  .action-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  /* Modal */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(2px);
  }

  .modal-content {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 12px;
    width: 90vw;
    max-width: 900px;
    height: 80vh;
    max-height: 700px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 20px 60px rgba(0,0,0,0.5);
    overflow: hidden;
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.7rem 1rem;
    background: #161b22;
    border-bottom: 1px solid #30363d;
    flex-shrink: 0;
  }

  .modal-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: #c9d1d9;
    font-size: 0.9rem;
    font-weight: 600;
  }

  .modal-icon { font-size: 1rem; }

  .modal-pid {
    color: #8b949e;
    font-size: 0.78rem;
    font-weight: 400;
    background: #21262d;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
  }

  .modal-close {
    background: none;
    border: none;
    color: #8b949e;
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
    transition: all 0.15s;
  }
  .modal-close:hover { background: #21262d; color: #f0f6fc; }

  .terminal-wrapper {
    flex: 1;
    overflow: hidden;
    padding: 0;
  }

  .terminal-wrapper :global(.xterm) {
    height: 100%;
    padding: 4px;
  }

  .modal-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.5rem 1rem;
    background: #161b22;
    border-top: 1px solid #30363d;
    flex-shrink: 0;
  }

  .footer-hint {
    color: #8b949e;
    font-size: 0.78rem;
  }

  button.secondary {
    padding: 0.35rem 1rem;
    border-radius: 6px;
    border: 1px solid #30363d;
    background: #21262d;
    color: #c9d1d9;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.15s;
  }
  button.secondary:hover { background: #30363d; }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    .card {
      background: #0f0f0f98;
      border-color: #444;
    }

    .field label { color: #aaa; }
    .field input {
      background: #2a2a2a;
      border-color: #555;
      color: #eee;
    }
    .field input:focus { border-color: #5b8def; background: #333; }

    .msg.ok { background: #1b3a1b; color: #81c784; border-color: #2e7d32; }
    .msg.err { background: #3a1a1a; color: #ef9a9a; border-color: #c62828; }

    .process-card {
      background: #0f0f0f98;
      border-color: #444;
    }
    .process-card:hover { border-color: #666; }

    .meta-item code { background: #2a2a2a; color: #aaa; }

    .restart-count { background: #3e2723; color: #ffb74d; }

    .status-badge.running { background: #1b3a1b; color: #81c784; }
    .status-badge.exited-ok { background: #1a237e; color: #c5cae9; }
    .status-badge.exited-err { background: #3e2723; color: #ffb74d; }
    .status-badge.killed { background: #3a1a1a; color: #ef9a9a; }
    .status-badge.crashed { background: #3a1a1a; color: #ef9a9a; }

    .action-btn {
      border-color: #444;
      color: #888;
    }
    .action-btn:hover:not(:disabled) { background: #333; color: #eee; border-color: #666; }
    .action-btn.logs:hover { background: #0d2137; color: #58a6ff; border-color: #1f6feb; }
    .action-btn.refresh:hover { background: #1c0d2e; color: #bc8cff; border-color: #7c3aed; }
    .action-btn.kill:hover:not(:disabled) { background: #3a1a1a; color: #ff7b72; border-color: #da3633; }

    h2 { color: #bbb; }

    .refresh-btn {
      background: #2a2a2a;
      color: #ccc;
      border-color: #555;
    }
    .refresh-btn:hover { background: #333; }
  }
</style>
