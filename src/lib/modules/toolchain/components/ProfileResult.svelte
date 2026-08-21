<script lang="ts">
  // Результат резолвера Build Environment: сгруппированные секции
  // (обязательные / рекомендованные / опциональные / docker / ручные /
  // неподдерживаемые) + конфликты и предупреждения. Статусы инструментов —
  // только факты снапшота скана; без скана честно «данных нет».
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import StateBadge from "./StateBadge.svelte";
  import type { EnvironmentProfile, ProfileTool, ToolScanResult } from "../types";
  import { formatSizeMb } from "../format";

  let {
    profile,
    liveStateById = {},
    resolving = false,
    onlocaltoggle,
    onreviewplan,
  }: {
    profile: EnvironmentProfile;
    /** Живые состояния из снапшота по tool_id (могут отсутствовать). */
    liveStateById?: Record<string, ToolScanResult>;
    resolving?: boolean;
    onlocaltoggle: (toolId: string, local: boolean) => void;
    onreviewplan: () => void;
  } = $props();

  const hasSnapshot = $derived(Object.keys(liveStateById).length > 0);
  const readiness = $derived(profile.readiness);

  function stateFor(toolId: string): ToolScanResult | null {
    return liveStateById[toolId] ?? null;
  }

  const totalSize = $derived(profile.estimated_download_size_mb);
</script>

<div class="profile">
  <!-- ===== Сводка готовности ===== -->
  <div class="summary">
    <div class="summary-main">
      <span class="summary-title">
        Профиль окружения
        {#if resolving}<span class="resolving">пересчитывается…</span>{/if}
      </span>
      <span class="summary-meta">
        {profile.required.length} обязательных · {totalSize > 0 ? formatSizeMb(totalSize) : "0 МБ"}
        {#if profile.warnings.some((w) => w.code === "admin_required")}
          · <span class="warn-text">нужны права администратора</span>
        {/if}
      </span>
    </div>
    <div class="readiness">
      {#if !hasSnapshot}
        <Badge tone="neutral">готовность неизвестна — выполните скан</Badge>
      {:else if readiness.all_ready}
        <Badge tone="lime" dot>окружение готово ({readiness.satisfied_count}/{readiness.required_total})</Badge>
      {:else}
        <Badge tone={readiness.satisfied_count === 0 ? "red" : "amber"} dot>
          готово {readiness.satisfied_count}/{readiness.required_total}
        </Badge>
      {/if}
      <Button
        variant="primary"
        size="sm"
        icon="check"
        disabled={profile.required.length === 0}
        onclick={onreviewplan}
      >
        Проверить план установки
      </Button>
    </div>
  </div>

  <!-- ===== Конфликты и предупреждения ===== -->
  {#if profile.conflicts.length > 0 || profile.unsupported.length > 0}
    <div class="alerts" role="alert">
      {#each profile.conflicts as c (c.tool + c.conflicts_with)}
        <p class="alert alert-red">
          <Icon name="alert" size={14} />
          Конфликт: «{c.tool}» конфликтует с «{c.conflicts_with}» (заявлено каталогом).
        </p>
      {/each}
      {#each profile.warnings as w (w.code + w.message)}
        <p class={`alert ${w.code === "admin_required" ? "alert-amber" : "alert-red"}`}>
          <Icon name="info" size={14} />
          {w.message}
        </p>
      {/each}
    </div>
  {/if}

  <!-- ===== Обязательные ===== -->
  {#if profile.required.length > 0}
    <section aria-label="Обязательные инструменты">
      <h4 class="group-title"><Badge tone="violet">{profile.required.length}</Badge> Обязательные</h4>
      <ul class="items">
        {#each profile.required as t (t.tool_id)}
          {@const live = stateFor(t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <div class="item-main">
              <span class="item-name">{t.display}</span>
              <span class="item-reason" title={t.reason}>{t.reason}</span>
            </div>
            <span class="item-how">{t.source_description}</span>
            <div class="item-side">
              {#if t.needs_admin}<Icon name="alert" size={12} class="admin-icon" /><span class="sr-only">нужны права администратора</span>{/if}
              <span class="item-size">{formatSizeMb(t.size_mb)}</span>
              {#if live}
                <StateBadge state={live.state} withVersion />
              {:else}
                <span class="no-state">нет данных</span>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="muted-note">Выберите стек слева — профиль появится здесь.</p>
  {/if}

  <!-- ===== Рекомендованные (честно пусто) ===== -->
  {#if profile.recommended.length > 0}
    <section aria-label="Рекомендованные">
      <h4 class="group-title"><Badge tone="cyan">{profile.recommended.length}</Badge> Рекомендованные</h4>
      <ul class="items">
        {#each profile.recommended as t (t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <span class="item-name">{t.display}</span>
            <span class="item-size">{formatSizeMb(t.size_mb)}</span>
          </li>
        {/each}
      </ul>
    </section>
  {:else if profile.required.length > 0}
    <p class="reserved-note">Рекомендации появятся, когда каталог получит метаданные рекомендаций.</p>
  {/if}

  <!-- ===== Опциональные (docker с локальным выбором) ===== -->
  {#if profile.optional.length > 0}
    <section aria-label="Опциональные">
      <h4 class="group-title"><Badge tone="blue">{profile.optional.length}</Badge> Опциональные — Docker или локально</h4>
      <ul class="items">
        {#each profile.optional as t (t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <div class="item-main">
              <span class="item-name">{t.display}</span>
              <span class="item-reason">по умолчанию разворачивается контейнером проекта</span>
            </div>
            <label class="opt-toggle">
              <input
                type="checkbox"
                checked={profile.local_alternatives.includes(t.tool_id)}
                onchange={(e) => onlocaltoggle(t.tool_id, e.currentTarget.checked)}
              />
              установить локально
            </label>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <!-- ===== Docker-managed ===== -->
  {#if profile.docker_managed.length > 0}
    <details class="collapsed-group">
      <summary>Docker-managed ({profile.docker_managed.length})</summary>
      <ul class="items">
        {#each profile.docker_managed as t (t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <span class="item-name">{t.display}</span>
            <span class="item-how">docker-compose проекта</span>
          </li>
        {/each}
      </ul>
    </details>
  {/if}

  <!-- ===== Только вручную ===== -->
  {#if profile.manual.length > 0}
    <section aria-label="Устанавливаются вручную">
      <h4 class="group-title"><Badge tone="amber">{profile.manual.length}</Badge> Установка вручную</h4>
      <ul class="items">
        {#each profile.manual as t (t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <div class="item-main">
              <span class="item-name">{t.display}</span>
              <span class="item-reason">{t.reason || "движки и SDK не устанавливаются автоматически"}</span>
            </div>
            <span class="item-how">{t.source_description}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <!-- ===== Неподдерживаемые ===== -->
  {#if profile.unsupported.length > 0}
    <details class="collapsed-group">
      <summary>Недоступно на этой ОС / вне каталога ({profile.unsupported.length})</summary>
      <ul class="items">
        {#each profile.unsupported as t (t.tool_id)}
          <li class="item">
            <TechIcon icon={t.icon} alt="" size="sm" />
            <span class="item-name">{t.display}</span>
            <span class="item-how warn-text">{t.source_description || "нет источника для этой платформы"}</span>
          </li>
        {/each}
      </ul>
    </details>
  {/if}
</div>

<style>
  .profile {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
  }

  .summary-main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 0;
  }

  .summary-title {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .resolving {
    margin-left: var(--sp-2);
    font-weight: var(--sp-fw-normal);
    font-size: var(--sp-fs-xs);
    color: var(--sp-cyan);
  }

  .summary-meta {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .warn-text {
    color: var(--sp-warning);
  }

  .readiness {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .alerts {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .alert {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-md);
    font-size: var(--sp-fs-xs);
  }

  .alert-red {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.08);
    border: 1px solid rgba(248, 113, 113, 0.28);
  }

  .alert-amber {
    color: var(--sp-amber);
    background: rgba(251, 191, 36, 0.08);
    border: 1px solid rgba(251, 191, 36, 0.3);
  }

  section {
    min-width: 0;
  }

  .group-title {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 0 var(--sp-2);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .items {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    overflow: hidden;
  }

  .item {
    display: grid;
    grid-template-columns: auto minmax(9rem, 1fr) auto auto;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .item:last-child {
    border-bottom: none;
  }

  .item-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .item-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-1);
  }

  .item-reason {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item-how {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    text-align: right;
  }

  .item-side {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
  }

  .item-side :global(.admin-icon) {
    color: var(--sp-amber);
  }

  .item-size {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
    min-width: 3.5rem;
    text-align: right;
  }

  .no-state {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
    white-space: nowrap;
  }

  .opt-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    cursor: pointer;
    user-select: none;
    white-space: nowrap;
  }

  .opt-toggle input {
    accent-color: var(--sp-accent-strong);
  }

  .collapsed-group {
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .collapsed-group summary {
    padding: var(--sp-2) var(--sp-3);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    user-select: none;
  }

  .collapsed-group .items {
    margin: 0 var(--sp-2) var(--sp-2);
  }

  .muted-note {
    margin: 0;
    padding: var(--sp-6);
    text-align: center;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    border: 1px dashed var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
  }

  .reserved-note {
    margin: calc(-1 * var(--sp-2)) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  @media (max-width: 760px) {
    .item {
      grid-template-columns: auto 1fr auto;
    }
    .item-how {
      display: none;
    }
  }
</style>
