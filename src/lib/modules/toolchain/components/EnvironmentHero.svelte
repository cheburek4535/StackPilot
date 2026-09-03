<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import type { EnvironmentSnapshot } from "../types";
  import { formatAgeSeconds, formatSizeMb } from "../format";

  let {
    snapshot,
    freshness,
    scanning = false,
    onscan,
    onbuild,
    onupdates,
  }: {
    snapshot: EnvironmentSnapshot | null;
    freshness: "live" | "stale" | "none";
    scanning?: boolean;
    onscan: () => void;
    onbuild: () => void;
    onupdates: () => void;
  } = $props();

  const summary = $derived(snapshot?.summary ?? null);
  const installedCount = $derived(
    summary ? (summary.installed_healthy ?? 0) + (summary.installed_health_unknown ?? 0) + (summary.update_available ?? 0) : 0
  );
  const updatesCount = $derived(summary?.update_available ?? 0);
  const brokenCount = $derived(
    summary ? (summary.path_broken ?? 0) + (summary.installed_unhealthy ?? 0) : 0
  );

  const disk = $derived(
    snapshot && snapshot.disk.length > 0
      ? snapshot.disk.reduce((min, d) => (d.free_mb < min.free_mb ? d : min))
      : null
  );

  const metrics = $derived.by(() => {
    if (!snapshot) return [];
    return [
      { key: "installed", value: installedCount, label: i18n.t("tc.hero.installed") as TranslationKey },
      { key: "updates", value: updatesCount, label: i18n.t("tc.hero.updates") as TranslationKey, tone: "updates" },
      { key: "broken", value: brokenCount, label: i18n.t("tc.hero.broken_path") as TranslationKey, tone: "broken" },
      disk
        ? { key: "disk", value: formatSizeMb(disk.free_mb), label: i18n.t("tc.hero.free_space") as TranslationKey, tone: disk.free_mb < 5000 ? "broken" : "" }
        : null,
    ].filter((m) => m !== null) as {
      key: string;
      value: string | number;
      label: string;
      tone?: string;
    }[];
  });
</script>

<section class="metric-bar" aria-label={i18n.t("tc.hero.env_state") as TranslationKey}>
  <div class="metrics">
    {#if snapshot}
      {#each metrics as m (m.key)}
        <div class="metric" class:tone-updates={m.tone === "updates"} class:tone-broken={m.tone === "broken"}>
          <span class="metric-value">{m.value}</span>
          <span class="metric-label">{m.label}</span>
        </div>
      {/each}
    {:else}
      <span class="placeholder">{i18n.t("tc.no_scan_yet") as TranslationKey}</span>
    {/if}
  </div>

  <div class="side">
    {#if snapshot}
      <span class="meta">
        <span
          class="fresh-dot"
          class:live={freshness === "live"}
          class:stale={freshness === "stale"}
          aria-hidden="true"
        ></span>
        {freshness === "stale"
          ? (i18n.t("tc.data_stale") as TranslationKey)
          : freshness === "live"
            ? (i18n.t("tc.data_live") as TranslationKey)
            : (i18n.t("tc.no_data") as TranslationKey)}
        {#if snapshot}
          · {formatAgeSeconds(snapshot.age_seconds)}
        {/if}
      </span>
    {/if}

    <div class="actions">
      {#if snapshot && updatesCount > 0}
        <Button variant="primary" size="sm" icon="refresh" onclick={onupdates}>
          {i18n.t("tc.hero.update_all") as TranslationKey}
        </Button>
      {/if}
      <Button
        variant={snapshot ? "outline" : "primary"}
        size="sm"
        icon="refresh"
        loading={scanning}
        onclick={onscan}
      >
        {i18n.t("tc.hero.scan_env") as TranslationKey}
      </Button>
    </div>
  </div>
</section>

<style>
  .metric-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    flex-wrap: wrap;
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-md);
  }

  .metrics {
    display: flex;
    align-items: center;
    gap: var(--sp-6);
    flex-wrap: wrap;
    min-width: 0;
  }

  .metric {
    display: inline-flex;
    align-items: baseline;
    gap: var(--sp-1);
    white-space: nowrap;
  }

  .metric-value {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    font-variant-numeric: tabular-nums;
    color: var(--sp-text-1);
  }

  .metric-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .tone-updates .metric-value {
    color: var(--sp-amber);
  }

  .tone-broken .metric-value {
    color: var(--sp-danger);
  }

  .placeholder {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  .side {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    flex-wrap: wrap;
  }

  .meta {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
  }

  .fresh-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--sp-radius-full);
    background: var(--sp-text-3);
  }

  .fresh-dot.live {
    background: var(--sp-lime);
  }

  .fresh-dot.stale {
    background: var(--sp-amber);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
</style>