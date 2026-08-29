<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { page } from "$app/stores";
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
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

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
  let destroyed = false;

  let resultMsg = $state("");
  let resultType = $state<"ok" | "err" | "">("");

  let logProcessName = $derived(() => {
    if (!logProcessId) return "";
    const p = processes.find((pr) => pr.id === logProcessId);
    return p?.label ?? logProcessId;
  });

  onMount(async () => {
    await loadProcesses();

    // Register listeners up-front and resolve them together so that a
    // mid-await unmount cannot leave a late-registered listener leaking.
    const regOutput = listen<ProcessOutputEvent>("process-output", (event) => {
      const { process_id, stream, line } = event.payload;
      if (logProcessId === process_id && terminal) {
        const prefix = stream === "stderr" ? "\x1b[31m" : "";
        const suffix = stream === "stderr" ? "\x1b[0m" : "";
        terminal.writeln(`${prefix}${line}${suffix}`);
      }
    });

    const regStatus = listen<ProcessStatusEvent>("process-status", (event) => {
      const { process_id, status, error } = event.payload;
      const idx = processes.findIndex((p) => p.id === process_id);
      if (idx >= 0) {
        const updated = { ...processes[idx], status, last_error: error ?? null };
        processes = [...processes.slice(0, idx), updated, ...processes.slice(idx + 1)];
      }
    });

    const [outFn, statusFn] = await Promise.all([regOutput, regStatus]);
    if (destroyed) {
      outFn();
      statusFn();
      return;
    }
    unlistenOutput = outFn;
    unlistenStatus = statusFn;

    autoRefreshId = setInterval(() => refreshAllStatuses(), 3000);

    // Support the workspace deep link `/devlauncher/processes?log=<id>`.
    const logId = $page.url.searchParams.get("log");
    if (logId) {
      const proc = processes.find((p) => p.id === logId);
      if (proc) await openLogs(logId);
    }
  });

  onDestroy(() => {
    destroyed = true;
    if (autoRefreshId) clearInterval(autoRefreshId);
    unlistenOutput?.();
    unlistenStatus?.();
    terminal?.dispose();
  });

  async function loadProcesses() {
    try {
      processes = await listProcesses();
    } catch (e) {
      errorMsg = i18n.t("devl.load_processes_failed", { err: String(e) });
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
      resultMsg = i18n.t("devl.spawned") as TranslationKey;
      resultType = "ok";
    } catch (e) {
      resultMsg = i18n.t("devl.spawn_failed", { err: String(e) });
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
      errorMsg = i18n.t("devl.kill_failed", { err: String(e) });
    }
  }

  async function handleRefresh(id: string) {
    try {
      await refreshProcess(id);
      await loadProcesses();
    } catch (e) {
      errorMsg = i18n.t("devl.refresh_failed", { err: String(e) });
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
        const proc = processes.find((p) => p.id === id);
        if (
          logs.stdout_lines.length === 0 &&
          logs.stderr_lines.length === 0 &&
          proc?.tracking_quality === "terminal_wrapper"
        ) {
          term.writeln("\x1b[33mВывод идёт в отдельное окно терминала — здесь логов нет.\x1b[0m");
        } else {
          for (const line of logs.stdout_lines) {
            term.writeln(line);
          }
          for (const line of logs.stderr_lines) {
            term.writeln(`\x1b[31m${line}\x1b[0m`);
          }
        }
      } catch (e) {
        term.writeln(`\x1b[33m${i18n.t("devl.load_logs_failed", { err: String(e) })}\x1b[0m`);
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
    if (status === "starting") return i18n.t("devl.status_starting");
    if (status === "running") return i18n.t("devl.status_running");
    if (status === "ready") return i18n.t("devl.status_ready");
    if (status === "killed") return i18n.t("devl.status_killed");
    if (status === "crashed") return i18n.t("devl.status_crashed");
    if (status === "timed_out") return i18n.t("devl.status_timed_out");
    if (status === "cancelled") return i18n.t("devl.status_cancelled");
    if (status === "external_launch_accepted") return i18n.t("devl.status_external");
    if (status === "unknown") return i18n.t("devl.status_unknown");
    if (typeof status === "object" && "exited" in status) {
      const code = (status as { exited: number }).exited;
      return code === 0 ? "Success" : i18n.t("devl.status_failed", { code });
    }
    if (typeof status === "object" && "exited_with_error" in status) {
      const code = (status as { exited_with_error: number }).exited_with_error;
      return i18n.t("devl.status_failed", { code });
    }
    return i18n.t("devl.status_unknown");
  }

  function statusClass(status: ProcessStatus): string {
    if (status === "starting") return "starting";
    if (status === "running") return "running";
    if (status === "ready") return "ready";
    if (status === "killed") return "killed";
    if (status === "crashed") return "crashed";
    if (status === "timed_out") return "timed-out";
    if (status === "cancelled") return "cancelled";
    if (status === "external_launch_accepted") return "external";
    if (status === "unknown") return "";
    if (typeof status === "object" && "exited" in status) {
      const code = (status as { exited: number }).exited;
      return code === 0 ? "exited-ok" : "exited-err";
    }
    if (typeof status === "object" && "exited_with_error" in status) {
      return "exited-err";
    }
    return "";
  }

  function isRunning(status: ProcessStatus): boolean {
    return (
      status === "starting" ||
      status === "running" ||
      status === "ready" ||
      status === "external_launch_accepted"
    );
  }

  function formatDuration(secs: number): string {
    if (secs < 60) return i18n.t("time.dur_secs", { n: secs });
    if (secs < 3600) return i18n.t("time.dur_min", { m: Math.floor(secs / 60), s: secs % 60 });
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return i18n.t("time.dur_hour", { h, m });
  }

  function formatStarted(timestamp: string): string {
    if (!timestamp) return "—";
    const secs = Math.floor(Date.now() / 1000 - Number(timestamp));
    if (secs < 60) return i18n.t("time.secs_ago", { n: secs });
    if (secs < 3600) return i18n.t("time.mins_ago", { n: Math.floor(secs / 60) });
    if (secs < 86400) return i18n.t("time.hours_ago", { n: Math.floor(secs / 3600) });
    return i18n.t("time.days_ago", { n: Math.floor(secs / 86400) });
  }

  function copyCommand(proc: TrackedProcess) {
    navigator.clipboard.writeText(`${proc.label} (${i18n.t("devl.pid", { pid: proc.pid })})`);
  }
</script>

<main>
  <div class="page-header">
    <div>
      <h1>{i18n.t("devl.process_manager") as TranslationKey}</h1>
      <p class="subtitle">{i18n.t("devl.pm_subtitle") as TranslationKey}</p>
    </div>
    <button class="refresh-btn" onclick={refreshAllStatuses} title={i18n.t("devl.refresh_all") as TranslationKey}>
      {i18n.t("devl.refresh") as TranslationKey}
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
    <h2>{i18n.t("devl.spawn_process") as TranslationKey}</h2>
    <div class="spawn-form">
      <div class="field-row">
        <div class="field flex-2">
          <label for="cmd-input">{i18n.t("devl.cmd") as TranslationKey}</label>
          <input id="cmd-input" type="text" bind:value={command} placeholder={i18n.t("devl.cmd_ph") as TranslationKey} />
        </div>
        <div class="field flex-1">
          <label for="label-input">{i18n.t("devl.label") as TranslationKey}</label>
          <input id="label-input" type="text" bind:value={label} placeholder={i18n.t("devl.label_ph") as TranslationKey} />
        </div>
      </div>
      <div class="field">
        <label for="args-input">{i18n.t("devl.args") as TranslationKey}</label>
        <input id="args-input" type="text" bind:value={argsStr} placeholder={i18n.t("devl.args_ph") as TranslationKey} />
      </div>
      <button class="primary" onclick={handleSpawn} disabled={spawning || !command}>
        {spawning ? (i18n.t("devl.spawning") as TranslationKey) : (i18n.t("devl.spawn") as TranslationKey)}
      </button>
    </div>
  </section>

  <!-- Process list -->
  <section>
    <h2>{i18n.t("devl.processes_count", { n: processes.length }) as TranslationKey}</h2>

    {#if loading}
      <p class="empty">{i18n.t("devl.profiles_loading") as TranslationKey}</p>
    {:else if processes.length === 0}
      <div class="empty-state">
        <p class="empty">{i18n.t("devl.no_processes") as TranslationKey}</p>
        <p class="hint">{i18n.t("devl.use_form") as TranslationKey}</p>
      </div>
    {:else}
      <div class="process-list">
        {#each processes as proc (proc.id)}
          <div class="process-card" class:exited={!isRunning(proc.status)}>
            <div class="card-top">
              <div class="proc-main">
                <div class="proc-label-row">
                  <span class="proc-icon">
                    {#if isRunning(proc.status)}▶{:else}⬛{/if}
                  </span>
                  <strong class="proc-label">{proc.label}</strong>
                  {#if proc.visible}
                    <span class="visible-badge" title={i18n.t("devl.visible_terminal") as TranslationKey}>🖥</span>
                  {/if}
                  {#if proc.tracking_quality === "terminal_wrapper"}
                    <span class="tracking-badge" title="PID points to terminal wrapper, not the inner command">⚠ track</span>
                  {:else if proc.tracking_quality === "detached"}
                    <span class="tracking-badge" title="Process launched detached, no PID tracking">⊘ detached</span>
                  {:else if proc.tracking_quality === "approximate"}
                    <span class="tracking-badge" title="PID was the process but may have been replaced">~ approx</span>
                  {/if}
                  <span class="status-badge {statusClass(proc.status)}">
                    {statusLabel(proc.status)}
                  </span>
                </div>
                <div class="proc-meta-row">
                  <span class="meta-item"><code>{i18n.t("devl.pid", { pid: proc.pid }) as TranslationKey}</code></span>
                  <span class="meta-item sep">·</span>
                  <span class="meta-item">{formatDuration(proc.duration_secs)}</span>
                  <span class="meta-item sep">·</span>
                  <span class="meta-item">{i18n.t("devl.started", { when: formatStarted(proc.started_at) }) as TranslationKey}</span>
                  {#if proc.restarts > 0}
                    <span class="meta-item sep">·</span>
                    <span class="meta-item restart-count">{i18n.t("devl.restarts", { n: proc.restarts }) as TranslationKey}</span>
                  {/if}
                </div>
                {#if proc.command}
                  <div class="proc-command">{proc.command}</div>
                {/if}
                {#if proc.run_id}
                  <div class="proc-run-meta">
                    <span class="meta-item">run {proc.run_id.slice(0, 8)}</span>
                    {#if proc.step_id}
                      <span class="meta-item sep">·</span>
                      <span class="meta-item">step {proc.step_id}</span>
                    {/if}
                    {#if proc.working_dir}
                      <span class="meta-item sep">·</span>
                      <span class="meta-item proc-cwd">{proc.working_dir}</span>
                    {/if}
                  </div>
                {/if}
                {#if proc.last_error}
                  <div class="proc-error">{proc.last_error}</div>
                {/if}
              </div>
              <div class="proc-actions">
                <button class="action-btn logs" onclick={() => openLogs(proc.id)} title={i18n.t("devl.view_logs") as TranslationKey}>
                  {i18n.t("devl.logs") as TranslationKey}
                </button>
                <button class="action-btn refresh" onclick={() => handleRefresh(proc.id)} title={i18n.t("devl.refresh_status") as TranslationKey}>
                  ⟳
                </button>
                <button
                  class="action-btn kill"
                  onclick={() => handleKill(proc.id)}
                  disabled={!isRunning(proc.status)}
                  title={i18n.t("devl.kill_process") as TranslationKey}
                >
                  {i18n.t("devl.kill") as TranslationKey}
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
    <div class="modal-content" onclick={(e) => e.stopPropagation()} role="dialog" aria-label={i18n.t("devl.logs_aria") as TranslationKey}>
      <div class="modal-header">
        <div class="modal-title">
          <span class="modal-icon">📋</span>
          <span>{i18n.t("devl.logs") as TranslationKey}: {logProcessName()}</span>
          <span class="modal-pid">{i18n.t("devl.pid", { pid: processes.find(p => p.id === logProcessId)?.pid ?? "—" }) as TranslationKey}</span>
        </div>
        <div class="modal-actions">
          <button class="modal-close" onclick={closeLogs}>✕</button>
        </div>
      </div>
      <div class="terminal-wrapper" bind:this={terminalEl}></div>
      <div class="modal-footer">
        <span class="footer-hint">{i18n.t("devl.realtime") as TranslationKey}</span>
        <button class="secondary" onclick={closeLogs}>{i18n.t("devl.close") as TranslationKey}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    max-width: 860px;
    margin: 0 auto;
    padding: 2rem;
    color: var(--sp-text-1);
  }

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1.5rem;
    gap: 1rem;
  }

  h1 { margin: 0; font-size: var(--sp-fs-xl); color: var(--sp-text-1); }
  .subtitle { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.2rem 0 0; }

  .refresh-btn {
    padding: 0.4rem 1rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s;
  }
  .refresh-btn:hover { background: var(--sp-bg-3); }

  .msg {
    padding: 0.5rem 1rem;
    border-radius: var(--sp-radius-sm);
    margin-bottom: 1rem;
    font-size: var(--sp-fs-sm);
  }
  .msg.ok { background: rgba(163, 230, 53, 0.12); color: var(--sp-success); border: 1px solid rgba(163, 230, 53, 0.35); }
  .msg.err { background: rgba(248, 113, 113, 0.12); color: var(--sp-danger); border: 1px solid rgba(248, 113, 113, 0.35); }

  .card {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    padding: 1.25rem;
    box-shadow: var(--sp-shadow-1);
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: var(--sp-fs-md);
    margin: 0 0 0.75rem;
    color: var(--sp-text-1);
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
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-2);
  }

  .field input {
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-sm);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    transition: border-color 0.15s;
  }

  .field input:focus {
    outline: none;
    border-color: var(--sp-accent);
    box-shadow: var(--sp-shadow-accent);
  }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: none;
    background: var(--sp-accent-strong);
    color: #fff;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    transition: background 0.15s;
    align-self: flex-start;
  }
  button.primary:hover:not(:disabled) { background: var(--sp-accent); }
  button.primary:disabled { opacity: 0.5; cursor: not-allowed; }

  .empty-state {
    text-align: center;
    padding: 2rem 1rem;
  }

  .empty {
    color: var(--sp-text-3);
    font-style: italic;
    font-size: var(--sp-fs-sm);
    text-align: center;
    padding: 2rem;
  }

  .hint {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    margin: 0.5rem 0 0;
  }

  .process-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .process-card {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    padding: 0.85rem 1rem;
    box-shadow: var(--sp-shadow-1);
    transition: border-color 0.15s;
  }
  .process-card:hover { border-color: var(--sp-border-strong); }
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
    font-size: var(--sp-fs-sm);
    flex-shrink: 0;
  }

  .proc-label {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .proc-meta-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    flex-wrap: wrap;
  }

  .meta-item code {
    font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
    padding: 0.05rem 0.35rem;
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-1);
  }

  .meta-item.sep { color: var(--sp-text-3); }

  .restart-count {
    background: rgba(251, 191, 36, 0.14);
    color: var(--sp-warning);
    padding: 0.05rem 0.4rem;
    border-radius: var(--sp-radius-xs);
    font-size: var(--sp-fs-xs);
  }

  .proc-error {
    margin-top: 0.35rem;
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.12);
    padding: 0.3rem 0.6rem;
    border-radius: var(--sp-radius-xs);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 3rem;
    overflow-y: auto;
  }

  .proc-command {
    margin-top: 0.35rem;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    font-family: var(--sp-font-mono);
    background: var(--sp-code-bg);
    padding: 0.25rem 0.5rem;
    border-radius: var(--sp-radius-xs);
    word-break: break-all;
  }

  .proc-run-meta {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    margin-top: 0.3rem;
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
    flex-wrap: wrap;
  }

  .proc-cwd {
    font-family: var(--sp-font-mono);
    word-break: break-all;
  }

  .status-badge {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-bold);
    text-transform: uppercase;
    padding: 0.15rem 0.45rem;
    border-radius: var(--sp-radius-xs);
    letter-spacing: 0.03em;
    flex-shrink: 0;
  }

  .status-badge.running { background: rgba(163, 230, 53, 0.14); color: var(--sp-success); }
  .status-badge.starting { background: rgba(96, 165, 250, 0.14); color: var(--sp-blue); }
  .status-badge.ready { background: rgba(34, 211, 238, 0.14); color: var(--sp-cyan); }
  .status-badge.exited-ok { background: rgba(96, 165, 250, 0.14); color: var(--sp-blue); }
  .status-badge.exited-err { background: rgba(251, 191, 36, 0.14); color: var(--sp-warning); }
  .status-badge.killed { background: rgba(248, 113, 113, 0.14); color: var(--sp-danger); }
  .status-badge.crashed { background: rgba(248, 113, 113, 0.2); color: var(--sp-danger); }
  .status-badge.timed-out { background: rgba(251, 191, 36, 0.14); color: var(--sp-warning); }
  .status-badge.cancelled { background: rgba(139, 92, 246, 0.14); color: var(--sp-violet); }
  .status-badge.external { background: rgba(34, 211, 238, 0.14); color: var(--sp-cyan); }

  .visible-badge {
    font-size: var(--sp-fs-xs);
    background: rgba(96, 165, 250, 0.14);
    color: var(--sp-blue);
    padding: 0.05rem 0.35rem;
    border-radius: var(--sp-radius-xs);
    cursor: help;
  }

  .tracking-badge {
    font-size: var(--sp-fs-2xs);
    background: rgba(251, 191, 36, 0.14);
    color: var(--sp-warning);
    padding: 0.05rem 0.35rem;
    border-radius: var(--sp-radius-xs);
    cursor: help;
    white-space: nowrap;
  }

  .proc-actions {
    display: flex;
    gap: 0.35rem;
    flex-shrink: 0;
    align-items: center;
  }

  .action-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-2);
    cursor: pointer;
    font-size: var(--sp-fs-xs);
    font-family: var(--sp-font-sans);
    transition: all 0.15s;
    white-space: nowrap;
  }

  .action-btn:hover:not(:disabled) { background: var(--sp-bg-2); color: var(--sp-text-1); border-color: var(--sp-border-strong); }
  .action-btn.logs:hover { background: rgba(96, 165, 250, 0.14); color: var(--sp-blue); border-color: rgba(96, 165, 250, 0.4); }
  .action-btn.refresh:hover { background: rgba(139, 92, 246, 0.14); color: var(--sp-violet); border-color: rgba(139, 92, 246, 0.4); }
  .action-btn.kill:hover:not(:disabled) { background: rgba(248, 113, 113, 0.14); color: var(--sp-danger); border-color: rgba(248, 113, 113, 0.4); }
  .action-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  /* Modal */
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(2px);
  }

  .modal-content {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: var(--sp-radius-lg);
    width: 90vw;
    max-width: 900px;
    height: 80vh;
    max-height: 700px;
    display: flex;
    flex-direction: column;
    box-shadow: var(--sp-shadow-3);
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
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
  }

  .modal-icon { font-size: 1rem; }

  .modal-pid {
    color: #8b949e;
    font-size: var(--sp-fs-xs);
    font-weight: 400;
    background: #21262d;
    padding: 0.1rem 0.4rem;
    border-radius: var(--sp-radius-xs);
  }

  .modal-close {
    background: none;
    border: none;
    color: #8b949e;
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0.2rem 0.4rem;
    border-radius: var(--sp-radius-xs);
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
    font-size: var(--sp-fs-xs);
  }

  button.secondary {
    padding: 0.35rem 1rem;
    border-radius: var(--sp-radius-sm);
    border: 1px solid #30363d;
    background: #21262d;
    color: #c9d1d9;
    font-size: var(--sp-fs-xs);
    font-family: var(--sp-font-sans);
    cursor: pointer;
    transition: all 0.15s;
  }
  button.secondary:hover { background: #30363d; }
</style>
