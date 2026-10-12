<script lang="ts">
  import { onMount } from "svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import {
    getToolchainEnvironmentsMatrix,
    pruneUnusedToolVersions,
  } from "../api";
  import type {
    ToolchainDeduplicationReport,
    ToolEnvironmentUsage,
  } from "../types";
  import { notifySuccess, notifyError } from "$lib/core/toasts";

  let report = $state<ToolchainDeduplicationReport | null>(null);
  let loading = $state(true);
  let pruning = $state(false);
  let expanded = $state(false);

  function formatBytes(bytes: number): string {
    if (!bytes || bytes <= 0) return "0 MB";
    const mb = bytes / (1024 * 1024);
    if (mb < 1024) return `${mb.toFixed(1)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  }

  async function loadMatrix() {
    loading = true;
    try {
      report = await getToolchainEnvironmentsMatrix();
    } catch (e) {
      console.warn("[DeduplicationBanner] Failed to load matrix:", e);
    } finally {
      loading = false;
    }
  }

  async function handlePrune() {
    pruning = true;
    try {
      const pruned = await pruneUnusedToolVersions();
      notifySuccess(
        i18n.t("tc.dedup.pruned_success", {
          n: pruned.length,
          mb: "0",
        }) as TranslationKey,
      );
      await loadMatrix();
    } catch (e) {
      notifyError("Failed to prune toolchains", String(e));
    } finally {
      pruning = false;
    }
  }

  onMount(() => {
    void loadMatrix();
  });
</script>

<div class="dedup-strip" class:is-expanded={expanded}>
  <div class="strip-left">
    <div class="strip-icon">
      <Icon name="database" size={14} />
    </div>
    <span class="strip-title">{i18n.t("tc.dedup.title" as TranslationKey)}</span>
    <span class="store-badge">Shared Store</span>

    <div class="strip-metrics">
      <span class="metric-chip emerald" title="Сэкономлено места за счёт дедупликации версий">
        <Icon name="sparkles" size={11} />
        <span>{report ? formatBytes(report.total_disk_saved_bytes) : "0 MB"} сэкономлено</span>
      </span>
      <span class="metric-chip neutral">
        <span>{report?.total_shared_versions ?? 0} версий</span>
      </span>
      <span class="metric-chip cyan">
        <span>{report?.total_bindings_using_shared ?? 0} песочниц</span>
      </span>
    </div>
  </div>

  <div class="strip-right">
    <Button
      variant="ghost"
      size="sm"
      icon="trash"
      loading={pruning}
      onclick={handlePrune}
      label="Удалить неиспользуемые версии"
    >
      {i18n.t("tc.dedup.prune_btn" as TranslationKey)}
    </Button>
    <button
      type="button"
      class="strip-toggle"
      onclick={() => (expanded = !expanded)}
      title={expanded ? "Скрыть детали" : "Показать детали"}
    >
      <span>Детали</span>
      <Icon name={expanded ? "chevronUp" : "chevronDown"} size={12} />
    </button>
  </div>
</div>

{#if expanded && report && report.usages.length > 0}
  <div class="usages-tray">
    <table class="usages-table">
      <thead>
        <tr>
          <th>Tool</th>
          <th>Version</th>
          <th>Source</th>
          <th>Disk Usage</th>
          <th>Bound Projects</th>
        </tr>
      </thead>
      <tbody>
        {#each report.usages as u}
          <tr>
            <td>
              <div class="tool-cell">
                <TechIcon icon={u.tool_id} alt={u.tool_id} size="sm" />
                <span class="tool-name">{u.tool_id}</span>
              </div>
            </td>
            <td>
              <code class="version-code">v{u.version}</code>
            </td>
            <td>
              <span class="source-pill" class:pill-store={u.source_kind === "shared_store"}>
                {u.source_kind === "shared_store" ? "StackPilot Store" : u.source_kind}
              </span>
            </td>
            <td>
              <span class="disk-text">{formatBytes(u.size_bytes)}</span>
            </td>
            <td>
              <div class="projects-cell">
                {#if u.bound_environments.length === 0}
                  <span class="unused-label">Unreferenced (safe to prune)</span>
                {:else}
                  <div class="project-tags">
                    {#each u.bound_environments as env}
                      <span class="project-tag" title={env.bound_projects.join(", ")}>
                        <Icon name="package" size={11} />
                        {env.name || env.binding_id}
                      </span>
                    {/each}
                  </div>
                {/if}
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .dedup-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.45rem 0.75rem;
    background: var(--sp-bg-2, #161821);
    border: 1px solid var(--sp-border, #262c3c);
    border-radius: 8px;
    margin-bottom: 1rem;
    transition: border-color 0.15s ease;
  }
  .dedup-strip.is-expanded {
    border-bottom-left-radius: 0;
    border-bottom-right-radius: 0;
    border-bottom-color: transparent;
    margin-bottom: 0;
  }
  .strip-left {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    flex-wrap: wrap;
  }
  .strip-icon {
    color: #a855f7;
    display: flex;
    align-items: center;
  }
  .strip-title {
    font-size: 0.82rem;
    font-weight: 600;
    color: var(--sp-text, #f1f5f9);
  }
  .store-badge {
    font-size: 0.65rem;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(168, 85, 247, 0.15);
    color: #c084fc;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }
  .strip-metrics {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-left: 0.4rem;
  }
  .metric-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.72rem;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--sp-bg-3, #1e222f);
    color: var(--sp-text-2, #cbd5e1);
  }
  .metric-chip.emerald {
    background: rgba(16, 185, 129, 0.12);
    color: #34d399;
    border: 1px solid rgba(16, 185, 129, 0.25);
    font-weight: 600;
  }
  .metric-chip.cyan {
    color: #38bdf8;
  }
  .strip-right {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .strip-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    background: transparent;
    border: 1px solid var(--sp-border-strong, #2f3547);
    color: var(--sp-text-3, #94a3b8);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .strip-toggle:hover {
    color: var(--sp-text-1, #f1f5f9);
    border-color: var(--sp-border-focus, #4f5872);
  }
  .usages-tray {
    background: var(--sp-bg-1, #10121a);
    border: 1px solid var(--sp-border, #262c3c);
    border-top: none;
    border-bottom-left-radius: 8px;
    border-bottom-right-radius: 8px;
    padding: 0.5rem 0.75rem;
    margin-bottom: 1rem;
    overflow-x: auto;
  }

  .usages-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  .usages-table th {
    text-align: left;
    padding: 0.4rem 0.6rem;
    font-size: 0.72rem;
    color: var(--sp-text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--sp-border-strong);
  }

  .usages-table td {
    padding: 0.5rem 0.6rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }

  .tool-cell {
    display: flex;
    align-items: center;
    gap: 0.45rem;
  }

  .tool-name {
    font-weight: 600;
    color: var(--sp-text);
    text-transform: capitalize;
  }

  .version-code {
    background: var(--sp-bg-3);
    padding: 2px 6px;
    border-radius: 4px;
    font-family: var(--sp-font-mono, monospace);
    font-size: 0.75rem;
    color: #e2e8f0;
  }

  .source-pill {
    font-size: 0.7rem;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--sp-bg-2);
    color: var(--sp-text-3);
    border: 1px solid var(--sp-border-strong);
  }

  .pill-store {
    background: rgba(168, 85, 247, 0.15);
    color: #c084fc;
    border-color: rgba(168, 85, 247, 0.3);
  }

  .disk-text {
    font-family: var(--sp-font-mono, monospace);
    color: var(--sp-text-2);
  }

  .project-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .project-tag {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 2px 6px;
    background: rgba(56, 189, 248, 0.1);
    color: #38bdf8;
    border: 1px solid rgba(56, 189, 248, 0.25);
    border-radius: 4px;
    font-size: 0.72rem;
  }

  .unused-label {
    font-size: 0.72rem;
    color: var(--sp-warning);
    font-style: italic;
  }
</style>
