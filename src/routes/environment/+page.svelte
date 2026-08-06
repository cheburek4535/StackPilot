<script lang="ts">
  // ================================================================
  // Страница "Окружение": общее состояние toolchain на машине.
  //
  // Показывает:
  //   1. Информацию об ОС и пакетных менеджерах (EnvironmentInfo)
  //   2. Health-отчёт по всем известным инструментам (HealthReport)
  //   3. Установленные инструменты и их пути (ToolchainMetadata)
  // ================================================================

  import { onMount } from "svelte";
  import {
    getEnvironmentInfo,
    getHealthReport,
    getToolchainMetadata,
  } from "$lib/modules/toolchain/api";
  import type {
    EnvironmentInfo,
    HealthReport,
    ToolchainMetadata,
    ToolHealth,
  } from "$lib/modules/toolchain/types";

  let envInfo = $state<EnvironmentInfo | null>(null);
  let health = $state<HealthReport | null>(null);
  let metadata = $state<ToolchainMetadata | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let status = $state("");
  let statusType = $state("");

  async function loadAll(spinner = true) {
    if (spinner) refreshing = true;
    status = "";
    statusType = "";
    try {
      const [e, h, m] = await Promise.all([
        getEnvironmentInfo(),
        getHealthReport(),
        getToolchainMetadata(),
      ]);
      envInfo = e;
      health = h;
      metadata = m;
    } catch (err) {
      status = `Ошибка загрузки: ${err}`;
      statusType = "error";
    }
    refreshing = false;
    loading = false;
  }

  onMount(() => loadAll(false));

  function installedCount(): number {
    return metadata ? Object.keys(metadata.tools).length : 0;
  }

  function toolVersion(tool: ToolHealth): string {
    return tool.ok ? "✓ ok" : "✗ сбой";
  }
</script>

<main>
  <h1>🖥 Окружение</h1>
  <p class="subtitle">Состояние инструментов разработки на этой машине</p>

  <div class="toolbar">
    <button onclick={() => loadAll()} disabled={refreshing}>
      {refreshing ? "Проверка..." : "Проверить заново"}
    </button>
  </div>

  {#if status}
    <div class="status" class:success={statusType === "success"} class:error={statusType === "error"}>
      {status}
    </div>
  {/if}

  {#if loading}
    <p class="loading">Загрузка...</p>
  {:else}
    <!-- ===== Система ===== -->
    <section>
      <h2>Система</h2>
      <div class="card">
        {#if envInfo}
          <div class="kv">
            <span>ОС</span><span>{envInfo.os}</span>
            <span>Версия</span><span>{envInfo.os_version}</span>
            <span>Пакетные менеджеры</span>
            <span>{envInfo.package_managers.join(", ") || "не найдены"}</span>
            <span>Инструментов известно</span><span>{envInfo.tool_count}</span>
          </div>
        {:else}
          <p class="empty">Нет данных</p>
        {/if}
      </div>
    </section>

    <!-- ===== Health-отчёт ===== -->
    <section>
      <h2>Health-отчёт</h2>
      {#if health}
        <div class="card">
          <div class="score-row">
            <span class="score-value">{health.score}%</span>
            <div class="score-bar">
              <div class="score-fill" style="width: {health.score}%"></div>
            </div>
            <span class="muted">проверено {new Date(health.scanned_at).toLocaleString()}</span>
          </div>

          {#if health.tools.length === 0}
            <p class="empty">Ни один инструмент не найден</p>
          {:else}
            <div class="tool-list">
              {#each health.tools as tool}
                <details>
                  <summary>
                    <span class="tool-dot" class:ok={tool.ok}></span>
                    <span class="tool-name">{tool.display}</span>
                    <span class="tool-status">{toolVersion(tool)}</span>
                  </summary>
                  <ul class="checks">
                    {#each tool.checks as check}
                      <li class:bad={!check.ok}>
                        <span class="check-label">{check.label}</span>
                        <span class="check-detail">
                          {#if check.ok}✓ {check.detail}{:else}✗ {check.detail}{/if}
                        </span>
                      </li>
                    {/each}
                  </ul>
                </details>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </section>

    <!-- ===== Установленные инструменты ===== -->
    <section>
      <h2>Установлено: {installedCount()}</h2>
      {#if metadata}
        <div class="card">
          <p class="hint">
            Последнее сканирование: {metadata.last_scan
              ? new Date(metadata.last_scan).toLocaleString()
              : "ещё не выполнялось"}
          </p>
          {#if installedCount() === 0}
            <p class="empty">Ничего не установлено через StackPilot</p>
          {:else}
            <table class="installed">
              <thead>
                <tr><th>Инструмент</th><th>Версия</th><th>Путь</th><th>Установлен</th></tr>
              </thead>
              <tbody>
                {#each Object.entries(metadata.tools) as [toolId, info]}
                  <tr>
                    <td class="tool-id">{toolId}</td>
                    <td>{info.version}</td>
                    <td class="path-cell">{info.path}</td>
                    <td>{new Date(info.installed_at).toLocaleDateString()}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      {/if}
    </section>
  {/if}
</main>

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
  }

  h1 {
    margin: 0 0 1.5rem;
    font-size: 1.3rem;
  }

  .subtitle {
    color: #888;
    font-size: 0.9rem;
    margin: -1rem 0 1.5rem;
  }

  .loading {
    color: #888;
    font-style: italic;
  }

  .muted {
    color: #999;
    font-size: 0.8rem;
  }

  .toolbar {
    display: flex;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .status {
    padding: 0.6rem 1rem;
    border-radius: 8px;
    font-size: 0.9rem;
    margin-bottom: 1rem;
  }
  .status.success { background: #e8f5e9; color: #2e7d32; border: 1px solid #a5d6a7; }
  .status.error { background: #ffebee; color: #c62828; border: 1px solid #ef9a9a; }

  section { margin-bottom: 1.5rem; }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
    color: #444;
  }

  .hint { font-size: 0.85rem; color: #888; margin: 0 0 0.5rem; }

  .card {
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 10px;
    padding: 1.25rem;
    box-shadow: 0 1px 4px rgba(0,0,0,0.06);
  }

  .kv {
    display: grid;
    grid-template-columns: 180px 1fr;
    gap: 0.4rem 1rem;
    font-size: 0.9rem;
  }
  .kv span:nth-child(odd) { color: #777; font-weight: 500; }
  .kv span:nth-child(even) { word-break: break-all; }

  .score-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }
  .score-value {
    font-size: 1.2rem;
    font-weight: 700;
    color: #396cd8;
    min-width: 3.5rem;
  }
  .score-bar {
    flex: 1;
    height: 10px;
    background: #ececec;
    border-radius: 5px;
    overflow: hidden;
  }
  .score-fill {
    height: 100%;
    background: linear-gradient(90deg, #f39c12, #396cd8, #2e7d32);
    border-radius: 5px;
    transition: width 0.4s ease;
  }

  .tool-list { display: flex; flex-direction: column; gap: 0.3rem; }
  .tool-list details {
    border: 1px solid #e5e5e5;
    border-radius: 8px;
    padding: 0.5rem 0.75rem;
    background: #fafafa;
  }
  .tool-list summary {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    cursor: pointer;
    font-size: 0.9rem;
    list-style: none;
  }
  .tool-list summary::-webkit-details-marker { display: none; }
  .tool-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #e53935;
    flex: 0 0 auto;
  }
  .tool-dot.ok { background: #43a047; }
  .tool-name { font-weight: 600; }
  .tool-status { margin-left: auto; font-size: 0.8rem; color: #999; }

  .checks { margin: 0.5rem 0 0; padding: 0; list-style: none; }
  .checks li {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    font-size: 0.85rem;
    padding: 0.2rem 0;
    border-top: 1px dashed #e0e0e0;
  }
  .checks li.bad .check-detail { color: #c62828; }
  .check-detail { color: #2e7d32; text-align: right; word-break: break-all; }

  .installed { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  .installed th {
    text-align: left;
    color: #888;
    font-weight: 600;
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid #e0e0e0;
  }
  .installed td { padding: 0.35rem 0.5rem; border-bottom: 1px solid #f0f0f0; }
  .tool-id { font-family: Consolas, monospace; font-weight: 600; }
  .path-cell {
    font-family: Consolas, monospace;
    font-size: 0.8rem;
    color: #555;
    word-break: break-all;
    max-width: 260px;
  }

  .empty { color: #999; font-style: italic; font-size: 0.9rem; }

  button {
    border-radius: 8px;
    border: 1px solid transparent;
    padding: 0.5em 1.2em;
    font-size: 0.9rem;
    cursor: pointer;
    background: #396cd8;
    color: #fff;
    font-weight: 500;
    transition: background 0.15s, opacity 0.15s;
  }
  button:hover:not(:disabled) { background: #2b5ab0; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }

  @media (prefers-color-scheme: dark) {
    h2 { color: #bbb; }
    .subtitle { color: #888; }
    .card { background: #0f0f0f98; border-color: #444; }
    .kv span:nth-child(odd) { color: #999; }
    .score-bar { background: #333; }
    .tool-list details { background: #1a1a1a; border-color: #444; }
    .tool-status { color: #888; }
    .checks li { border-top-color: #333; }
    .installed td { border-bottom-color: #2a2a2a; }
    .installed th { color: #999; border-bottom-color: #444; }
    .path-cell { color: #aaa; }
  }
</style>
