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

<div class="dedup-card">
  <div class="dedup-header">
    <div class="dedup-title-block">
      <div class="dedup-icon">
        <Icon name="database" size={18} />
      </div>
      <div>
        <div class="title-row">
          <h3 class="dedup-title">{i18n.t("tc.dedup.title" as TranslationKey)}</h3>
          <Badge tone="violet">{i18n.t("create.shared_store_tag" as TranslationKey)}</Badge>
        </div>
        <p class="dedup-desc">{i18n.t("tc.dedup.desc" as TranslationKey)}</p>
      </div>
    </div>

    <div class="header-actions">
      <Button
        variant="secondary"
        size="sm"
        icon="trash"
        loading={pruning}
        onclick={handlePrune}
      >
        {i18n.t("tc.dedup.prune_btn" as TranslationKey)}
      </Button>
      <button
        type="button"
        class="expand-toggle"
        onclick={() => (expanded = !expanded)}
        title={expanded ? "Collapse details" : "Expand details"}
      >
        <Icon name={expanded ? "chevronUp" : "chevronDown"} size={14} />
      </button>
    </div>
  </div>

  <div class="stats-grid">
    <div class="stat-box highlight">
      <span class="stat-label">{i18n.t("tc.dedup.savings" as TranslationKey)}</span>
      <span class="stat-value text-emerald">
        {report ? formatBytes(report.total_disk_saved_bytes) : "0 MB"}
      </span>
      <span class="stat-sub">Zero duplicate copies</span>
    </div>

    <div class="stat-box">
      <span class="stat-label">{i18n.t("tc.dedup.shared_versions" as TranslationKey)}</span>
      <span class="stat-value">
        {report?.total_shared_versions ?? 0}
      </span>
      <span class="stat-sub">Portable versions in store</span>
    </div>

    <div class="stat-box">
      <span class="stat-label">{i18n.t("tc.dedup.bound_projects" as TranslationKey)}</span>
      <span class="stat-value text-cyan">
        {report?.total_bindings_using_shared ?? 0}
      </span>
      <span class="stat-sub">Active project sandboxes</span>
    </div>
  </div>

  {#if expanded && report && report.usages.length > 0}
    <div class="usages-table-wrapper">
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
</div>

<style>
  .dedup-card {
    background: linear-gradient(135deg, rgba(26, 27, 38, 0.85) 0%, rgba(20, 21, 31, 0.95) 100%);
    border: 1px solid rgba(168, 85, 247, 0.25);
    border-radius: 12px;
    padding: 1.1rem 1.25rem;
    margin-bottom: 1.25rem;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.2);
    backdrop-filter: blur(12px);
  }

  .dedup-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 1rem;
  }

  .dedup-title-block {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
  }

  .dedup-icon {
    width: 34px;
    height: 34px;
    border-radius: 8px;
    background: rgba(168, 85, 247, 0.15);
    border: 1px solid rgba(168, 85, 247, 0.3);
    color: #c084fc;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.2rem;
  }

  .dedup-title {
    font-size: 1rem;
    font-weight: 600;
    color: var(--sp-text);
    margin: 0;
  }

  .dedup-desc {
    font-size: 0.8rem;
    color: var(--sp-text-3);
    margin: 0;
    max-width: 600px;
    line-height: 1.35;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .expand-toggle {
    width: 32px;
    height: 32px;
    border-radius: 6px;
    border: 1px solid var(--sp-border-strong);
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .expand-toggle:hover {
    background: var(--sp-bg-3);
    color: var(--sp-text);
    border-color: var(--sp-border-focus);
  }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.75rem;
  }

  .stat-box {
    background: rgba(15, 17, 23, 0.6);
    border: 1px solid var(--sp-border-strong);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .stat-box.highlight {
    border-color: rgba(16, 185, 129, 0.3);
    background: rgba(16, 185, 129, 0.05);
  }

  .stat-label {
    font-size: 0.72rem;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sp-text-3);
  }

  .stat-value {
    font-size: 1.35rem;
    font-weight: 700;
    color: var(--sp-text);
    font-family: var(--sp-font-mono, monospace);
  }

  .text-emerald { color: #34d399; }
  .text-cyan { color: #38bdf8; }

  .stat-sub {
    font-size: 0.72rem;
    color: var(--sp-text-3);
  }

  .usages-table-wrapper {
    margin-top: 1rem;
    border-top: 1px solid var(--sp-border-strong);
    padding-top: 0.75rem;
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
