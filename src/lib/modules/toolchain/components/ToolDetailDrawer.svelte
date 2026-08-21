<script lang="ts">
  // Боковая панель деталей инструмента: статус, версии, обнаружение,
  // здоровье, PATH, происхождение, источники, зависимости и журнал.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import StateBadge from "./StateBadge.svelte";
  import type {
    CardPlanOp,
  } from "./ToolCard.svelte";
  import type { ToolDefinition, ToolScanResult } from "../types";
  import {
    capabilityLabels,
    formatRelativeTime,
    formatSizeMb,
    healthStateInfo,
    installSourceDescription,
    platformName,
    provenanceInfo,
    toolStateVersion,
    versionAssessmentInfo,
  } from "../format";

  let {
    open,
    tool,
    def = null,
    dependents = [],
    jobLogLines = [],
    busyRecheck = false,
    os = "",
    adopted = false,
    onclose,
    onplan,
    onrecheck,
    onadopt,
  }: {
    open: boolean;
    tool: ToolScanResult | null;
    def?: ToolDefinition | null;
    /** Инструменты каталога, которые зависят от этого (обратные связи). */
    dependents?: string[];
    /** Строки журналов заданий, относящиеся к этому инструменту. */
    jobLogLines?: { text: string; timestamp: string }[];
    busyRecheck?: boolean;
    os?: string;
    /** Инструмент взят пользователем под наблюдение (adopt-метка). */
    adopted?: boolean;
    onclose: () => void;
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
    /** Явное «отслеживать эту установку» (tcx_adopt_tool). */
    onadopt?: (toolId: string) => void;
  } = $props();

  const EVIDENCE_LABELS: Record<string, string> = {
    version_probe: "проба версии",
    known_path: "известный путь",
    footprint: "след установки",
  };

  function evidenceLabel(kind: string): string {
    return EVIDENCE_LABELS[kind] ?? kind;
  }

  const version = $derived(tool ? toolStateVersion(tool.state) : null);
  const assessment = $derived(tool ? versionAssessmentInfo(tool.version_assessment) : null);
  const recommended = $derived(def?.versions.recommended ?? null);
  const provenance = $derived(tool ? provenanceInfo(tool.provenance) : null);
  const health = $derived(tool?.health ? healthStateInfo(tool.health.state) : null);

  const sourcesForOs = $derived.by(() => {
    if (!def || !os) return [];
    const list =
      os === "windows" ? def.sources.windows : os === "linux" ? def.sources.linux : def.sources.macos;
    return list.map((s) => ({
      description: installSourceDescription(s),
      verified: !!s.sha256,
      needs_admin: s.needs_admin === true || (s.needs_admin == null && def.needs_admin),
    }));
  });

  type PrimaryAction =
    | { kind: "plan"; operation: CardPlanOp; label: string }
    | { kind: "recheck"; label: string }
    | null;

  const action = $derived.by<PrimaryAction>(() => {
    if (!tool) return null;
    const s = tool.state.kind;
    if (s === "missing" && tool.capabilities.installable) {
      return { kind: "plan", operation: "install", label: "Установить" };
    }
    if (s === "update_available" && tool.capabilities.updatable) {
      return { kind: "plan", operation: "update", label: "Обновить" };
    }
    if (s === "path_broken" && tool.capabilities.repairable) {
      return { kind: "plan", operation: "repair_path", label: "Починить PATH" };
    }
    if (
      (s === "installed_healthy" ||
        s === "installed_health_unknown" ||
        s === "installed_unhealthy") &&
      tool.capabilities.health_checkable
    ) {
      return { kind: "recheck", label: busyRecheck ? "Проверка…" : "Проверить здоровье" };
    }
    return null;
  });

  let drawerEl = $state<HTMLDivElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  // Фокус в панель при открытии; на закрытии — возврат туда, откуда
  // пришли (клавиатурная навигация без потери места на странице).
  $effect(() => {
    if (open) {
      lastFocused =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
      queueMicrotask(() => drawerEl?.focus());
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function handleKeydown(e: KeyboardEvent): void {
    if (open && e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <div class="drawer-root">
    <button
      type="button"
      class="backdrop"
      aria-label="Закрыть панель"
      onclick={onclose}
    ></button>
    <div
      bind:this={drawerEl}
      class="drawer"
      role="dialog"
      aria-modal="true"
      aria-label={tool ? `Детали: ${tool.display}` : "Детали инструмента"}
      tabindex="-1"
    >
      {#if !tool}
        <header class="head">
          <h2 class="title">Нет данных</h2>
          <IconButton icon="x" label="Закрыть" size="sm" onclick={onclose} />
        </header>
        <p class="placeholder">
          Инструмент ещё не проверялся сканом. Запустите сканирование или повторите позже.
        </p>
      {:else}
        <!-- ===== Hero ===== -->
        <header class="head hero">
          <TechIcon icon={tool.icon} alt="" size="lg" />
          <div class="hero-text">
            <h2 class="title">{tool.display}</h2>
            <span class="tool-id">{tool.tool_id}</span>
            <div class="badges">
              <StateBadge state={tool.state} />
              {#if provenance}
                <Badge tone={provenance.tone}>{provenance.label}</Badge>
              {/if}
              {#if adopted}
                <Badge tone="violet" dot>отслеживается</Badge>
              {/if}
              {#if tool.category}
                <Badge tone="neutral">{tool.category}</Badge>
              {/if}
            </div>
          </div>
          <IconButton icon="x" label="Закрыть" size="sm" onclick={onclose} />
        </header>

        <div class="body">
          {#if def?.description}
            <p class="description">{def.description}</p>
          {/if}

          <!-- ===== Версии ===== -->
          <section>
            <h3 class="section-title">Версия</h3>
            <dl class="kv">
              <dt>Установлена</dt>
              <dd>{version ?? "—"}</dd>
              {#if recommended}
                <dt>Рекомендуемая</dt>
                <dd>{recommended}</dd>
              {/if}
              {#if assessment}
                <dt>Оценка</dt>
                <dd><Badge tone={assessment.tone}>{assessment.label}</Badge></dd>
              {/if}
            </dl>
          </section>

          <!-- ===== Обнаружение ===== -->
          <section>
            <h3 class="section-title">Обнаружение</h3>
            {#if tool.detection.kind === "pending"}
              <p class="muted">Ещё не проверялось.</p>
            {:else if tool.detection.kind === "not_detected"}
              <p class="muted">Установок не найдено.</p>
            {:else if tool.detection.kind === "failed"}
              <p class="warn-text">Проверка не удалась: {tool.detection.reason || "причина неизвестна"}. Результат — «неизвестно», а не «отсутствует».</p>
            {/if}

            {#if tool.installs.length > 0}
              <ul class="installs">
                {#each tool.installs as inst, i (i)}
                  <li class="install">
                    <div class="install-head">
                      <span class="mono">{inst.parsed_version || inst.raw_version || "?"}</span>
                      <Badge tone={inst.reachable_via_path ? "lime" : "amber"}>
                        {inst.reachable_via_path ? "доступен из PATH" : "вне PATH"}
                      </Badge>
                    </div>
                    <code class="path">{inst.location || "путь неизвестен"}</code>
                    <span class="evidence">{evidenceLabel(inst.evidence.kind)}</span>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if tool.duration_ms > 0}
              <p class="muted">Длительность проверки: {(tool.duration_ms / 1000).toFixed(1)} с</p>
            {/if}
          </section>

          <!-- ===== Здоровье ===== -->
          {#if health}
            <section>
              <h3 class="section-title">Здоровье</h3>
              <p class="health-line"><Badge tone={health.tone} dot>{health.label}</Badge></p>
              {#if tool.health && tool.health.results.length > 0}
                <ul class="checks">
                  {#each tool.health.results as check (check.label)}
                    <li class="check">
                      <Icon
                        name={check.passed ? "check" : check.process_failed ? "help" : "alert"}
                        size={13}
                        class={check.passed ? "ok" : "fail"}
                      />
                      <span class="check-label">{check.label}</span>
                      <span class="check-detail" title={check.detail}>{check.detail}</span>
                      <span class="check-ms">{check.duration_ms} мс</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/if}

          <!-- ===== PATH ===== -->
          {#if tool.path_findings.length > 0}
            <section>
              <h3 class="section-title">PATH</h3>
              <ul class="findings">
                {#each tool.path_findings as f, i (i)}
                  <li class="finding">
                    <Icon name="alert" size={13} class="fail" />
                    <div>
                      <code class="path">{f.entry || "(запись)"}</code>
                      <p class="finding-detail">{f.detail}</p>
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          <!-- ===== Зависимости ===== -->
          <section>
            <h3 class="section-title">Зависимости</h3>
            <div class="chip-row">
              {#if def?.bundled_with}
                <Badge tone="cyan">в комплекте с {def.bundled_with}</Badge>
              {/if}
              {#each def?.dependencies ?? [] as dep (dep)}
                <Badge tone="neutral">требует {dep}</Badge>
              {/each}
              {#each def?.conflicts ?? [] as conflict (conflict)}
                <Badge tone="red">конфликт с {conflict}</Badge>
              {/each}
              {#if dependents.length > 0}
                {#each dependents as dep (dep)}
                  <Badge tone="violet">нужен для {dep}</Badge>
                {/each}
              {/if}
              {#if !def?.bundled_with && (def?.dependencies ?? []).length === 0 && (def?.conflicts ?? []).length === 0 && dependents.length === 0}
                <span class="muted">самодостаточен</span>
              {/if}
            </div>
          </section>

          <!-- ===== Источники и целостность ===== -->
          {#if sourcesForOs.length > 0}
            <section>
              <h3 class="section-title">
                Источники установки{os ? ` · ${platformName(os)}` : ""}
              </h3>
              <ul class="sources">
                {#each sourcesForOs as src (src.description)}
                  <li class="source">
                    <span>{src.description}</span>
                    <Badge tone={src.verified ? "lime" : "amber"}>
                      {src.verified ? "контроль суммы" : "без контроля целостности"}
                    </Badge>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if def?.docker}
            <section>
              <h3 class="section-title">Docker</h3>
              <p class="docker-line">
                <code class="mono-inline">{def.docker.image ?? "образ не указан"}</code>
                {#if def.docker.notes}<span class="muted"> · {def.docker.notes}</span>{/if}
              </p>
            </section>
          {/if}

          <!-- ===== Служебное ===== -->
          <section>
            <h3 class="section-title">Служебное</h3>
            <dl class="kv">
              {#if def}
                <dt>Размер загрузки</dt>
                <dd>{formatSizeMb(def.size_mb)}</dd>
              {/if}
              {#if def?.needs_admin}
                <dt>Права</dt>
                <dd><Badge tone="amber">требует прав администратора</Badge></dd>
              {/if}
              {#if tool.capabilities.installable}
                <dt>Установка</dt>
                <dd><Badge tone="lime">поддерживается</Badge></dd>
              {:else if tool.applicability.kind === "manual_only"}
                <dt>Установка</dt>
                <dd><Badge tone="violet">только вручную</Badge></dd>
              {:else if tool.applicability.kind === "unsupported_on_platform"}
                <dt>Установка</dt>
                <dd><Badge tone="neutral">не поддерживается на этой ОС</Badge></dd>
              {:else if tool.state.kind === "docker_managed"}
                <dt>Режим</dt>
                <dd><Badge tone="blue">управляется Docker</Badge></dd>
              {/if}
            </dl>
          </section>

          {#if def?.notes}
            <section>
              <h3 class="section-title">Примечания</h3>
              <p class="notes">{def.notes}</p>
            </section>
          {/if}

          {#if tool.state.kind === "manual_install" && def?.manual_install}
            <section>
              <h3 class="section-title">Ручная установка</h3>
              <p class="notes warn-text">{def.manual_install}</p>
            </section>
          {/if}

          {#if def?.docs_url || def?.source_url}
            <section>
              <h3 class="section-title">Ссылки</h3>
              <div class="links">
                {#if def?.docs_url}
                  <a href={def.docs_url} target="_blank" rel="noreferrer noopener">
                    <Icon name="external" size={13} /> Документация
                  </a>
                {/if}
                {#if def?.source_url}
                  <a href={def.source_url} target="_blank" rel="noreferrer noopener">
                    <Icon name="external" size={13} /> Исходный код
                  </a>
                {/if}
              </div>
            </section>
          {/if}

          <!-- ===== Журнал ===== -->
          <section>
            <h3 class="section-title">Журнал операций</h3>
            {#if jobLogLines.length > 0}
              <details class="log-details">
                <summary>{jobLogLines.length} записей</summary>
                <pre class="log-lines">{#each jobLogLines as line}{line.text}
{/each}</pre>
              </details>
            {:else}
              <p class="muted">Операций с этим инструментом в этой сессии не было.</p>
            {/if}
          </section>

          {#if tool.error}
            <section>
              <h3 class="section-title">Ошибка последней проверки</h3>
              <pre class="error-box">{tool.error}</pre>
            </section>
          {/if}
        </div>

        <footer class="foot">
          {#if action?.kind === "plan"}
            <Button
              variant={action.operation === "repair_path" ? "secondary" : "primary"}
              onclick={() => onplan(action.operation, tool!.tool_id)}
            >
              {action.label}
            </Button>
          {/if}
          {#if action?.kind === "recheck"}
            <Button variant="secondary" loading={busyRecheck} onclick={() => onrecheck(tool!.tool_id)}>
              {action.label}
            </Button>
          {/if}
          {#if tool.capabilities.health_checkable && action === null}
            <Button variant="secondary" loading={busyRecheck} onclick={() => onrecheck(tool!.tool_id)}>
              Проверить здоровье
            </Button>
          {/if}
          <span class="spacer"></span>
          {#if !adopted && onadopt && (tool.provenance.kind === "external" || tool.provenance.kind === "unknown")}
            <Button
              variant="ghost"
              size="sm"
              label="Запомнить эту стороннюю установку как наблюдаемую (не «установлено StackPilot»)"
              onclick={() => onadopt(tool!.tool_id)}
            >
              Отслеживать
            </Button>
          {/if}
        </footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .drawer-root {
    position: fixed;
    inset: 0;
    z-index: 900;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(5, 6, 10, 0.5);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    cursor: default;
    animation: sp-fade-in 0.15s ease;
  }

  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(30rem, 100vw);
    display: flex;
    flex-direction: column;
    background: var(--sp-glass-strong);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
    border-left: 1px solid var(--sp-border-strong);
    box-shadow: var(--sp-shadow-3);
    animation: sp-slide-in-right 0.18s ease;
  }

  @media (max-width: 640px) {
    .drawer {
      width: 100vw;
    }
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    padding: var(--sp-5) var(--sp-5) var(--sp-4);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .hero {
    align-items: center;
  }

  .hero-text {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    flex: 1 1 auto;
    min-width: 0;
  }

  .title {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .tool-id {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .badges {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
    margin-top: var(--sp-1);
  }

  .body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: var(--sp-4) var(--sp-5);
    display: flex;
    flex-direction: column;
    gap: var(--sp-5);
  }

  .description {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }

  .section-title {
    margin: 0;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-3);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .kv {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--sp-1) var(--sp-4);
    margin: 0;
    font-size: var(--sp-fs-sm);
  }

  .kv dt {
    color: var(--sp-text-3);
  }

  .kv dd {
    margin: 0;
    color: var(--sp-text-1);
    word-break: break-word;
  }

  .installs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .install {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
  }

  .install-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .mono {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
  }

  .path {
    display: block;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    word-break: break-all;
  }

  .evidence {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .check {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    align-items: baseline;
    gap: var(--sp-2);
    font-size: var(--sp-fs-xs);
    padding: var(--sp-1) 0;
    border-bottom: 1px dashed var(--sp-border-faint);
  }

  .check:last-child {
    border-bottom: none;
  }

  .check-label {
    color: var(--sp-text-1);
    white-space: nowrap;
  }

  .check-detail {
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .check-ms {
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
  }

  /* Классы приходят как prop внутрь компонента Icon — поэтому :global,
     но строго скошопированный областью списка проверок. */
  .checks :global(.ok) {
    color: var(--sp-lime);
  }

  .checks :global(.fail) {
    color: var(--sp-danger);
  }

  .findings {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .finding {
    display: flex;
    gap: var(--sp-2);
    align-items: flex-start;
    font-size: var(--sp-fs-xs);
  }

  .finding-detail {
    margin: var(--sp-1) 0 0;
    color: var(--sp-text-2);
  }

  .chip-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
  }

  .sources {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .source {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
  }

  .docker-line {
    margin: 0;
    font-size: var(--sp-fs-sm);
  }

  .mono-inline {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    padding: 0 var(--sp-1);
  }

  .notes {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  .warn-text {
    color: var(--sp-warning);
  }

  .links {
    display: flex;
    gap: var(--sp-4);
  }

  .links a {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-sm);
  }

  .log-details summary {
    cursor: pointer;
    font-size: var(--sp-fs-xs);
    color: var(--sp-accent);
    user-select: none;
  }

  .log-lines {
    margin: var(--sp-2) 0 0;
    max-height: 10rem;
    overflow: auto;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    background: var(--sp-code-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: var(--sp-2) var(--sp-3);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error-box {
    margin: 0;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.06);
    border: 1px solid rgba(248, 113, 113, 0.25);
    border-radius: var(--sp-radius-md);
    padding: var(--sp-2) var(--sp-3);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 8rem;
    overflow: auto;
  }

  .muted {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .health-line {
    margin: 0;
  }

  .placeholder {
    padding: var(--sp-6) var(--sp-5);
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-4) var(--sp-5);
    border-top: 1px solid var(--sp-border-faint);
  }

  .spacer {
    flex: 1 1 auto;
  }
</style>
