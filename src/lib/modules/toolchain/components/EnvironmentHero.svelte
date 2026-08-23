<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
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
  const admin = $derived(snapshot?.admin ?? null);
</script>

<section class="hero" class:hero-empty={!snapshot} aria-label={i18n.t("tc.hero.env_state") as TranslationKey}>
  <div class="hero-main">
    <div class="hero-title">
      <h2>{i18n.t("tc.hero.local_env") as TranslationKey}</h2>
      {#if freshness === "stale"}
        <Badge tone="amber">{i18n.t("tc.hero.outdated") as TranslationKey}</Badge>
      {:else if freshness === "live"}
        <Badge tone="cyan" dot>{i18n.t("tc.fresh") as TranslationKey}</Badge>
      {/if}
    </div>
    <p class="last-scan">
      {#if snapshot}{i18n.t("tc.hero.last_scan") as TranslationKey} {formatAgeSeconds(snapshot.age_seconds)}
      {:else}
        {i18n.t("tc.no_scan_yet") as TranslationKey}
      {/if}
    </p>

    {#if snapshot}
      <div class="stats-grid">
        <div class="stat-card">
          <span class="stat-value">{installedCount}</span>
          <span class="stat-label">{i18n.t("tc.hero.installed") as TranslationKey}</span>
        </div>
        {#if updatesCount > 0}
          <div class="stat-card stat-cyan">
            <span class="stat-value">{updatesCount}</span>
            <span class="stat-label">{i18n.t("tc.hero.updates") as TranslationKey}</span>
          </div>
        {/if}
        {#if brokenCount > 0}
          <div class="stat-card stat-red">
            <span class="stat-value">{brokenCount}</span>
            <span class="stat-label">{i18n.t("tc.hero.needs_repair") as TranslationKey}</span>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <div class="hero-side">
    {#if snapshot}
      <div class="sys-info">
        
        {#if disk}
          <span class="sys-tag" class:warn-text={disk.free_mb < 5000}>
            <Icon name="folder" size={14} /> {i18n.t("tc.hero.free") as TranslationKey} {formatSizeMb(disk.free_mb)}
          </span>
        {/if}
      </div>
    {/if}

    <div class="hero-actions">
      {#if !snapshot}
        <Button variant="primary" loading={scanning} onclick={onscan}>{i18n.t("tc.hero.start_scan") as TranslationKey}</Button>
      {:else}
        {#if updatesCount > 0}
          <Button variant="primary" onclick={onupdates}>{i18n.t("tc.updates_btn", { n: updatesCount }) as TranslationKey}</Button>
        {/if}
        <Button variant="outline" onclick={onbuild}>{i18n.t("tc.build_for_project") as TranslationKey}</Button>
      {/if}
    </div>
  </div>
</section>

<style>
  .hero {
    display: flex;
    justify-content: space-between;
    gap: var(--sp-6);
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-lg);
    padding: var(--sp-5);
  }
  .hero-empty {
    align-items: center;
  }
  .hero-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .hero-title {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }
  .hero-title h2 {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }
  .last-scan {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }
  .stats-grid {
    display: flex;
    gap: var(--sp-4);
    margin-top: var(--sp-4);
  }
  .stat-card {
    display: flex;
    flex-direction: column;
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-md);
    min-width: 120px;
  }
  .stat-cyan {
    border-color: rgba(6, 182, 212, 0.3);
    background: rgba(6, 182, 212, 0.05);
  }
  .stat-cyan .stat-value { color: var(--sp-cyan); }
  .stat-red {
    border-color: rgba(248, 113, 113, 0.3);
    background: rgba(248, 113, 113, 0.05);
  }
  .stat-red .stat-value { color: var(--sp-danger); }
  .stat-value {
    font-size: var(--sp-fs-2xl);
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    line-height: 1;
  }
  .stat-label {
    margin-top: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }
  .hero-side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    justify-content: center;
    gap: var(--sp-4);
  }
  .sys-info {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--sp-1);
  }
  .sys-tag {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }
  .warn-text {
    color: var(--sp-warning);
  }
  .hero-actions {
    display: flex;
    gap: var(--sp-3);
  }
</style>
