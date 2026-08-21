<script lang="ts">
  // Герой состояния окружения: оценка, счётчики, диск, права, действия.
  // Честность данных: нет скана или данные устарели — нейтральные/янтарные
  // тоны и «—» вместо успеха; зелёный только при живых полных данных.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import ScoreRing from "./ScoreRing.svelte";
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

  const score = $derived(snapshot?.score.score ?? null);
  const summary = $derived(snapshot?.summary ?? null);
  const scoreInfo = $derived.by(() => {
    if (score == null) return { label: "Нет данных", tone: "neutral" as const };
    if (score >= 80) return { label: "Окружение здорово", tone: "lime" as const };
    if (score >= 50) return { label: "Требует внимания", tone: "amber" as const };
    return { label: "Есть критичные проблемы", tone: "red" as const };
  });

  const problems = $derived(
    summary ? summary.path_broken + summary.installed_unhealthy : 0,
  );
  const healthUnknown = $derived(summary?.installed_health_unknown ?? 0);

  /** Сводное состояние с учётом свежести и полноты данных. */
  const verdict = $derived.by(() => {
    if (!snapshot || freshness === "none") {
      return { label: "Данных ещё нет", tone: "neutral" as const };
    }
    if (problems > 0 || (score != null && score < 50)) {
      return { label: "Требует ремонта", tone: "red" as const };
    }
    if (
      !snapshot.complete ||
      snapshot.errors.length > 0 ||
      freshness === "stale" ||
      summary!.missing > 0 ||
      summary!.update_available > 0 ||
      healthUnknown > 0
    ) {
      return { label: "Внимание", tone: "amber" as const };
    }
    return { label: "Всё готово", tone: "lime" as const };
  });

  const disk = $derived(
    snapshot && snapshot.disk.length > 0
      ? snapshot.disk.reduce((min, d) => (d.free_mb < min.free_mb ? d : min))
      : null,
  );

  const admin = $derived(snapshot?.admin ?? null);
</script>

<section class="hero" class:hero-empty={!snapshot} aria-label="Состояние окружения">
  <div class="hero-score">
    <ScoreRing
      {score}
      tone={scoreInfo.tone === "lime" ? "lime" : scoreInfo.tone === "amber" ? "amber" : scoreInfo.tone === "red" ? "red" : "neutral"}
      partial={!!snapshot && !snapshot.complete}
    />
    <div class="score-meta">
      <div class="verdict-row">
        <span class={`verdict verdict-${verdict.tone}`}>
          <Icon
            name={verdict.tone === "lime" ? "check" : verdict.tone === "neutral" ? "info" : "alert"}
            size={14}
          />
          {verdict.label}
        </span>
        {#if freshness === "stale"}
          <Badge tone="amber">Данные устарели</Badge>
        {:else if freshness === "live"}
          <Badge tone="cyan" dot>Актуально</Badge>
        {/if}
      </div>
      <p class="last-scan">
        {#if snapshot}
          Последнее сканирование: {formatAgeSeconds(snapshot.age_seconds)}
          {#if !snapshot.complete}<span class="warn-text"> · отчёт частичный</span>{/if}
        {:else}
          Сканирование ещё не выполнялось
        {/if}
      </p>
    </div>
  </div>

  <dl class="stats">
    <div class="stat" title="Обязательные инструменты в порядке">
      <dt>Обязательные готовы</dt>
      <dd>
        {#if snapshot}
          {snapshot.score.healthy_required}/{snapshot.score.counted_tools}
        {:else}&mdash;{/if}
      </dd>
    </div>
    <div class="stat" class:stat-warn={!!summary && summary.update_available > 0}>
      <dt>Обновления</dt>
      <dd>{summary ? summary.update_available : "—"}</dd>
    </div>
    <div class="stat" class:stat-bad={problems > 0}>
      <dt>Проблемные</dt>
      <dd>{summary ? problems : "—"}</dd>
    </div>
    <div class="stat" title="Не обнаружены при последнем скане">
      <dt>Отсутствуют</dt>
      <dd>{summary ? summary.missing : "—"}</dd>
    </div>
    <div class="stat" title={disk ? `Свободно на ${disk.root}` : undefined}>
      <dt>Диск</dt>
      <dd>{disk ? formatSizeMb(disk.free_mb) : "—"}</dd>
    </div>
    <div class="stat" title="Права администратора для установок">
      <dt>Права</dt>
      <dd>
        {#if admin}
          {admin.required_by_tools ? (admin.elevation_supported ? "нужны · доступны" : "нужны · недоступны") : "не требуются"}
        {:else}&mdash;{/if}
      </dd>
    </div>
  </dl>

  <div class="actions">
    <Button onclick={onscan} loading={scanning} icon="refresh">
      {scanning ? "Сканирование…" : "Сканировать"}
    </Button>
    <Button variant="secondary" icon="layers" onclick={onbuild}>
      Собрать окружение
    </Button>
    <Button
      variant="outline"
      icon="rocket"
      disabled={!summary || summary.update_available === 0}
      onclick={onupdates}
    >
      Обзор обновлений{summary && summary.update_available > 0 ? ` (${summary.update_available})` : ""}
    </Button>
  </div>
</section>

<style>
  .hero {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--sp-8);
    padding: var(--sp-6);
    border-radius: var(--sp-radius-xl);
    border: 1px solid var(--sp-border);
    background:
      radial-gradient(1200px 300px at 85% -60%, var(--sp-accent-soft), transparent),
      var(--sp-glass-bg);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    box-shadow: var(--sp-shadow-1);
  }

  .hero-score {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    min-width: 0;
  }

  .score-meta {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }

  .verdict-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .verdict {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .verdict-lime { color: var(--sp-lime); }
  .verdict-amber { color: var(--sp-amber); }
  .verdict-red { color: var(--sp-red); }
  .verdict-neutral { color: var(--sp-text-2); }

  .last-scan {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  .warn-text { color: var(--sp-warning); }

  .stats {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--sp-3) var(--sp-6);
    margin: 0;
    min-width: 0;
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-3);
    border-left: 2px solid var(--sp-border);
    min-width: 0;
  }

  .stat dt {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .stat dd {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    font-variant-numeric: tabular-nums;
  }

  .stat-warn dd { color: var(--sp-amber); }
  .stat-bad dd { color: var(--sp-red); }

  .actions {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 13rem;
  }

  @media (max-width: 1100px) {
    .hero {
      grid-template-columns: 1fr;
      gap: var(--sp-5);
    }
    .actions {
      flex-direction: row;
      flex-wrap: wrap;
      min-width: 0;
    }
  }

  @media (max-width: 640px) {
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .hero-score {
      flex-direction: column;
      text-align: center;
    }
    .verdict-row {
      justify-content: center;
    }
  }
</style>
