<script lang="ts">
  // Герой состояния окружения: оценка здоровья, счётчики, диск, права.
  // Честность данных: нет скана или данные устарели — нейтральные/янтарные
  // тоны и «—» вместо успеха; зелёный только при живых полных данных.
  //
  // Подписи счётчиков — ЗДОРОВЬЕ окружения, а не «обязательный набор»:
  // «Применимые» вместо «обязательные готовы», «Не проверено» вместо
  // скрытого unchecked. Поповер у кольца объясняет формулу и вклад.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import ScoreRing from "./ScoreRing.svelte";
  import type { EnvironmentSnapshot } from "../types";
  import { formatAgeSeconds, formatSizeMb } from "../format";
  import { SCORE_FORMULA_TEXT, scoreBreakdown } from "../score";

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
  const breakdown = $derived(scoreBreakdown(snapshot));
  const scoreInfo = $derived.by(() => {
    if (score == null) return { label: "Нет данных", tone: "neutral" as const };
    if (score >= 80) return { label: "Окружение здорово", tone: "lime" as const };
    if (score >= 50) return { label: "Требует внимания", tone: "amber" as const };
    return { label: "Есть критичные проблемы", tone: "red" as const };
  });

  const problems = $derived(
    summary ? (summary.path_broken ?? 0) + (summary.installed_unhealthy ?? 0) : 0,
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

  // ---- Поповер объяснения оценки ----
  let explainOpen = $state(false);
  let explainBtn = $state<HTMLButtonElement | null>(null);

  function toggleExplain(): void {
    explainOpen = !explainOpen;
  }

  function closeExplain(): void {
    explainOpen = false;
    explainBtn?.focus();
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (explainOpen && e.key === "Escape") {
      e.preventDefault();
      closeExplain();
    }
  }

  /** Строки сводки для поповера (правдивые подписи, без «обязательных»). */
  const explainRows = $derived.by(() => [
    { label: "Применимые инструменты", value: breakdown.applicable, tone: "" },
    { label: "Здоровы", value: breakdown.healthy, tone: "lime" },
    { label: "Обновления (деградация)", value: breakdown.degraded, tone: "amber" },
    { label: "Проблемные (сломаны/нездоровы)", value: breakdown.unhealthy, tone: "red" },
    { label: "Отсутствуют", value: breakdown.missing, tone: "amber" },
    { label: "Не проверено", value: breakdown.unchecked, tone: "neutral" },
    { label: "Неприменимо здесь", value: breakdown.not_applicable, tone: "neutral" },
    { label: "Исключено (bundled-дети)", value: breakdown.excluded_bundled, tone: "cyan" },
  ]);

  /** Только инструменты, влияющие на формулу (для компактности). */
  const contributingTools = $derived(
    breakdown.contributions.filter((c) => c.counted),
  );
</script>

<svelte:window onkeydown={handleKeydown} />

<section class="hero" class:hero-empty={!snapshot} aria-label="Состояние окружения">
  <div class="hero-score">
    <div class="ring-wrap">
      <ScoreRing
        {score}
        tone={scoreInfo.tone === "lime" ? "lime" : scoreInfo.tone === "amber" ? "amber" : scoreInfo.tone === "red" ? "red" : "neutral"}
        partial={!!snapshot && !snapshot.complete}
      />
      {#if snapshot}
        <button
          bind:this={explainBtn}
          type="button"
          class="explain-btn"
          aria-expanded={explainOpen}
          aria-label="Как посчитана оценка"
          onclick={toggleExplain}
        >
          <Icon name="help" size={14} />
        </button>
      {/if}
    </div>
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

    {#if explainOpen}
      <button
        type="button"
        class="explain-backdrop"
        aria-label="Закрыть объяснение оценки"
        onclick={closeExplain}
      ></button>
      <div class="explain-panel" role="dialog" aria-label="Как посчитана оценка">
        <div class="explain-head">
          <span>Как посчитана оценка</span>
          <IconButton icon="x" label="Закрыть объяснение" size="sm" onclick={closeExplain} />
        </div>
        <p class="explain-formula">{SCORE_FORMULA_TEXT}</p>
        <dl class="explain-rows">
          {#each explainRows as row (row.label)}
            <div class="explain-row">
              <dt>{row.label}</dt>
              <dd class={row.tone ? `tone-${row.tone}` : ""}>{row.value}</dd>
            </div>
          {/each}
        </dl>
        {#if contributingTools.length > 0}
          <p class="explain-sub">Вклад инструментов (здоровый 100 · устарел 50 · 0):</p>
          <ul class="explain-tools">
            {#each contributingTools as c (c.tool_id)}
              <li>
                <span class="tool-name" title={c.tool_id}>{c.display}</span>
                <span class="tool-label">{c.label}</span>
                <span class="tool-points">{c.contribution}</span>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="explain-sub">В формуле нет ни одного инструмента — оценивать нечего.</p>
        {/if}
      </div>
    {/if}
  </div>

  <dl class="stats">
    <div class="stat" title="Применимые инструменты, вошедшие в формулу">
      <dt>Применимые</dt>
      <dd>
        {#if snapshot}
          {snapshot.score.counted_tools}
        {:else}&mdash;{/if}
      </dd>
    </div>
    <div class="stat" title="Установлены и проверки здоровья пройдены">
      <dt>Здоровы</dt>
      <dd>{summary ? summary.installed_healthy : "—"}</dd>
    </div>
    <div class="stat" class:stat-warn={!!summary && summary.update_available > 0}>
      <dt>Обновления</dt>
      <dd>{summary ? summary.update_available : "—"}</dd>
    </div>
    <div class="stat" class:stat-bad={problems > 0}>
      <dt>Проблемы</dt>
      <dd>{summary ? problems : "—"}</dd>
    </div>
    <div class="stat" title="Не обнаружены при последнем скане">
      <dt>Отсутствуют</dt>
      <dd>{summary ? summary.missing : "—"}</dd>
    </div>
    <div class="stat" title="Не проверено: ошибка опроса или нет проверок здоровья">
      <dt>Не проверено</dt>
      <dd>
        {summary ? summary.scan_failed + summary.scan_pending + summary.installed_health_unknown : "—"}
      </dd>
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
    position: relative;
  }

  .ring-wrap {
    position: relative;
  }

  .explain-btn {
    position: absolute;
    top: -0.35rem;
    right: -0.35rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.4rem;
    height: 1.4rem;
    border-radius: var(--sp-radius-full);
    border: 1px solid var(--sp-border-strong);
    background: var(--sp-bg-1);
    color: var(--sp-text-3);
    cursor: pointer;
    transition: color 0.15s ease, border-color 0.15s ease;
  }

  .explain-btn:hover {
    color: var(--sp-text-1);
    border-color: var(--sp-accent-border);
  }

  .score-meta {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }

  .explain-backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    border: none;
    background: transparent;
    cursor: default;
    padding: 0;
  }

  .explain-panel {
    position: absolute;
    left: 0;
    top: calc(100% + var(--sp-3));
    z-index: 40;
    width: min(26rem, calc(100vw - 2rem));
    max-height: min(60vh, 34rem);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-4);
    background: var(--sp-glass-strong);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-3);
  }

  .explain-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .explain-formula {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  .explain-rows {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin: 0;
    font-size: var(--sp-fs-xs);
  }

  .explain-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .explain-row dt {
    color: var(--sp-text-3);
  }

  .explain-row dd {
    margin: 0;
    font-weight: var(--sp-fw-semibold);
    font-variant-numeric: tabular-nums;
    color: var(--sp-text-1);
  }

  .tone-lime { color: var(--sp-lime) !important; }
  .tone-amber { color: var(--sp-amber) !important; }
  .tone-red { color: var(--sp-red) !important; }
  .tone-cyan { color: var(--sp-cyan) !important; }

  .explain-sub {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .explain-tools {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    max-height: 12rem;
    overflow-y: auto;
  }

  .explain-tools li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: baseline;
    gap: var(--sp-2);
    font-size: var(--sp-fs-xs);
  }

  .tool-name {
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tool-label {
    color: var(--sp-text-3);
    white-space: nowrap;
  }

  .tool-points {
    color: var(--sp-text-2);
    font-variant-numeric: tabular-nums;
    min-width: 1.6rem;
    text-align: right;
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
    grid-template-columns: repeat(4, minmax(0, 1fr));
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
    .explain-panel {
      left: 50%;
      transform: translateX(-50%);
    }
  }
</style>