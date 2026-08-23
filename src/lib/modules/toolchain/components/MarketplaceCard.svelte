<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Карточка витрины: определение-driven, живые факты накладываются только
  // когда есть скан. Действие правдиво относительно состояния скана:
  // установленный инструмент НЕ предлагается ставить заново (обновление —
  // отдельная кнопка, только когда скан говорит «доступно обновление»).
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import StateBadge from "./StateBadge.svelte";
  import type { MarketplaceItem } from "../marketplace";
  import { marketplaceAction } from "../marketplace";
  import {
    formatSizeMb,
    platformName,
    toolVersionDisplay,
  } from "../format";
  import type { OperationKind } from "../types";

  export type MarketplacePlanOp = OperationKind;

  let {
    item,
    busy = false,
    onplan,
    ondetails,
  }: {
    item: MarketplaceItem;
    /** Идёт перепроверка инструмента (после установки и т.п.). */
    busy?: boolean;
    onplan: (operation: MarketplacePlanOp, toolId: string) => void;
    ondetails: (toolId: string) => void;
  } = $props();

  const def = $derived(item.def);
  const scan = $derived(item.scan);
  const version = $derived(scan ? toolVersionDisplay(scan) : null);
  const recommended = $derived(def.versions?.recommended ?? null);

  /** Установлен, но версии нет ни в состоянии, ни в уликах — явная
   *  пометка вместо молчаливого «—» (контракт: версия или объяснение). */
  const versionUnknown = $derived(
    !!scan &&
      !version &&
      (scan.state.kind === "installed_healthy" ||
        scan.state.kind === "installed_health_unknown" ||
        scan.state.kind === "installed_unhealthy"),
  );

  const checksum = $derived.by(() => {
    switch (item.checksum) {
      case "all_verified":
        return { label: i18n.t("tc.install.checksum_yes") as TranslationKey, tone: "lime" as const };
      case "partial":
        return { label: i18n.t("tc.install.checksum_partial") as TranslationKey, tone: "amber" as const };
      case "none":
        return { label: i18n.t("tc.install.checksum_none") as TranslationKey, tone: "amber" as const };
      default:
        return null;
    }
  });

  /** Правдивое действие карточки: скан (если есть) важнее каталога. */
  const action = $derived(marketplaceAction(item, busy));

  /** Для bundled-инструментов (pip с python) подпись «предоставляется ОС»
   *  была бы ложью — честно «в комплекте с <хостом>». */
  const builtInLabel = $derived(
    def.bundled_with ? (i18n.t("tc.install.bundled_with", { tool: def.bundled_with }) as TranslationKey) : (i18n.t("tc.install.builtin_os") as TranslationKey),
  );

  const hasLinks = $derived(!!(def.docs_url || def.source_url));
</script>

<article class="card">
  <header class="head">
    <TechIcon icon={def.icon} alt="" size="md" />
    <div class="title-wrap">
      <button type="button" class="title-btn" onclick={() => ondetails(def.id)}>
        <span class="title">{def.display}</span>
      </button>
      <span class="category">{def.category}</span>
    </div>
    {#if scan}
      <StateBadge state={scan.state} />
    {:else}
      <Badge tone="neutral">{i18n.t("tc.install.no_scan") as TranslationKey}</Badge>
    {/if}
  </header>

  {#if def.description}
    <p class="description">{i18n.t(def.description as TranslationKey)}</p>
  {/if}

  <!-- ===== Живой факт: версия / состояние ===== -->
  <dl class="meta">
    {#if scan}
      <div class="meta-item" title={version?.text ?? (versionUnknown ? (i18n.t("tc.install.version_unknown") as TranslationKey) : "")}>
        <dt>{i18n.t("tc.card.version") as TranslationKey}</dt>
        <dd>
          {#if version}
            {version.text}{#if !version.parsed}
              <span title={i18n.t("tc.install.raw_output") as TranslationKey}>*</span>
            {/if}
          {:else if versionUnknown}
            <span title={i18n.t("tc.install.raw_output") as TranslationKey}>
              {i18n.t("tc.install.version_unknown") as TranslationKey}
            </span>
          {:else}
            —
          {/if}
        </dd>
      </div>
    {/if}
    {#if scan?.state.kind === "update_available"}
      <div class="meta-item" title={scan.state.recommended}>
        <dt>{i18n.t("tc.install.recommended_label") as TranslationKey}</dt>
        <dd class="update-target">{scan.state.recommended}</dd>
      </div>
    {:else if recommended}
      <div class="meta-item" title={recommended}>
        <dt>{i18n.t("tc.install.recommended_label") as TranslationKey}</dt>
        <dd>{recommended}</dd>
      </div>
    {/if}
    <div class="meta-item">
      <dt>{i18n.t("tc.install.size") as TranslationKey}</dt>
      <dd>{formatSizeMb(def.size_mb)}</dd>
    </div>
  </dl>

  <!-- ===== Компактные факты каталога (одна строка, всё усекается) ===== -->
  <div class="facts" aria-label={i18n.t("tc.install.facts") as TranslationKey}>
    {#if item.availability.length > 0}
      {#each item.availability as p (p)}
        <Badge tone={p === item.os ? "cyan" : "neutral"}>{platformName(p)}</Badge>
      {/each}
    {/if}
    {#if checksum}
      <Badge tone={checksum.tone}>{checksum.label}</Badge>
    {/if}
    {#if item.docker_alternative}
      <Badge tone="blue">{i18n.t("tc.install.docker_image", { image: item.docker_alternative.image ?? (i18n.t("tc.install.no_image") as TranslationKey) }) as TranslationKey}</Badge>
    {/if}
    {#if def.manual_install}
      <Badge tone="violet">{i18n.t("tc.install.manual_only") as TranslationKey}</Badge>
    {/if}
  </div>

  {#if def.manual_install}
    <p class="manual-note" title={i18n.t(def.manual_install as TranslationKey)}>{i18n.t(def.manual_install as TranslationKey)}</p>
  {/if}

  <footer class="foot">
    {#if action.kind === "install"}
      <Button
        variant="primary"
        size="sm"
        icon="plus"
        loading={busy}
        onclick={() => onplan("install", def.id)}
      >
        {action.label}
      </Button>
    {:else if action.kind === "update"}
      <Button variant="primary" size="sm" icon="refresh" onclick={() => onplan("update", def.id)}>
        {action.label}
      </Button>
    {:else if action.kind === "installed"}
      <Button variant="secondary" size="sm" disabled onclick={() => ondetails(def.id)}>
        {action.label}
      </Button>
    {:else if action.kind === "manual"}
      <Button variant="secondary" size="sm" onclick={() => ondetails(def.id)}>
        {action.label}
      </Button>
    {:else if action.kind === "no_source" || action.kind === "built_in"}
      <Button variant="ghost" size="sm" disabled>
        {action.kind === "built_in" && def.bundled_with ? builtInLabel : action.label}
      </Button>
    {/if}
    <span class="spacer"></span>
    {#if hasLinks}
      {#if def.docs_url}
        <a class="link-icon" href={def.docs_url} target="_blank" rel="noreferrer noopener" title={i18n.t("tc.install.docs") as TranslationKey}>
          <Icon name="external" size={12} />
        </a>
      {/if}
      {#if def.source_url}
        <a class="link-icon" href={def.source_url} target="_blank" rel="noreferrer noopener" title={i18n.t("tc.install.source") as TranslationKey}>
          <Icon name="file" size={12} />
        </a>
      {/if}
    {/if}
    <IconButton
      icon="chevronRight"
      label={i18n.t("tc.install.more_info", { name: def.display }) as TranslationKey}
      size="sm"
      onclick={() => ondetails(def.id)}
    />
  </footer>
</article>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    height: 100%;
    min-height: 0;
    padding: var(--sp-4);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-glass-bg);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    box-shadow: var(--sp-shadow-1);
    transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
    min-width: 0;
    overflow: hidden;
  }

  .card:hover {
    border-color: var(--sp-border-strong);
    box-shadow: var(--sp-shadow-2);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .title-wrap {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
    flex: 1 1 auto;
  }

  .title-btn {
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--sp-radius-xs);
    min-width: 0;
  }

  .title {
    display: block;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .category {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    text-transform: capitalize;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .description {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    min-height: calc(2 * var(--sp-lh-normal) * var(--sp-fs-sm));
  }

  .meta {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(0, 1fr));
    gap: var(--sp-2) var(--sp-4);
    margin: 0;
    padding: var(--sp-2) 0;
    border-top: 1px solid var(--sp-border-faint);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .meta-item {
    min-width: 0;
  }

  .meta-item dt {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .meta-item dd {
    margin: 0;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-2);
    font-family: var(--sp-font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .update-target {
    color: var(--sp-amber);
    font-weight: var(--sp-fw-semibold);
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
    min-width: 0;
  }

  .facts :global(.sp-badge) {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .facts :global(.sp-badge-label) {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .manual-note {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-warning);
    line-height: var(--sp-lh-normal);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: auto;
    min-width: 0;
  }

  .spacer {
    flex: 1 1 auto;
  }

  .link-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.4rem;
    height: 1.4rem;
    padding: 0 0.2rem;
    border-radius: var(--sp-radius-sm);
    color: var(--sp-text-3);
    text-decoration: none;
    font-size: var(--sp-fs-xs);
  }

  .link-icon:hover {
    color: var(--sp-accent);
    background: var(--sp-bg-2);
  }
</style>