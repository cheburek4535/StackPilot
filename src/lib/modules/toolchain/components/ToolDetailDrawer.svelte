<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
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
    healthCheckLogText,
    healthStateInfo,
    installSourceDescription,
    pathScopeInfo,
    platformName,
    probeLogHasContent,
    probeLogText,
    provenanceInfo,
    toolVersionDisplay,
    versionAssessmentInfo,
  } from "../format";

  let {
    open,
    tool,
    def = null,
    dependents = [],
    jobLogLines = [],
    busyRecheck = false,
    busyDetails = false,
    detailsError = null,
    detailsRetry = null,
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
    /** Живое обновление деталей инструмента в полёте. */
    busyDetails?: boolean;
    /** Ошибка последнего обновления деталей (повторяемая). */
    detailsError?: string | null;
    /** Повтор обновления деталей после ошибки. */
    detailsRetry?: (() => void) | null;
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
    version_probe: i18n.t("tc.drawer.evidence.version_probe") as TranslationKey,
    known_path: i18n.t("tc.drawer.evidence.known_path") as TranslationKey,
    footprint: i18n.t("tc.drawer.evidence.footprint") as TranslationKey,
  };

  function evidenceLabel(kind: string): string {
    return EVIDENCE_LABELS[kind] ?? kind;
  }

  // ---- Полные логи: разворачиваемые панели + копирование ----

  /** Ключ скопированной панели (для подписи «Скопировано»). */
  let copiedKey = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;

  async function copyText(text: string, key: string): Promise<void> {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        // Fallback без Clipboard API (старые webview).
        const area = document.createElement("textarea");
        area.value = text;
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.appendChild(area);
        area.select();
        document.execCommand("copy");
        area.remove();
      }
      copiedKey = key;
      if (copiedTimer) clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copiedKey = null), 1500);
    } catch {
      /* копирование — best-effort; панель с логом остаётся читаемой */
    }
  }

  const version = $derived(tool ? toolVersionDisplay(tool) : null);
  const assessment = $derived(tool ? versionAssessmentInfo(tool.version_assessment) : null);
  const recommended = $derived(def?.versions?.recommended ?? null);
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
      return { kind: "plan", operation: "install", label: i18n.t("tc.op.install") as TranslationKey };
    }
    if (s === "update_available" && tool.capabilities.updatable) {
      return { kind: "plan", operation: "update", label: i18n.t("tc.op.update") as TranslationKey };
    }
    if (s === "path_broken" && tool.capabilities.repairable) {
      return { kind: "plan", operation: "repair_path", label: i18n.t("tc.op.repair_path") as TranslationKey };
    }
    if (
      (s === "installed_healthy" ||
        s === "installed_health_unknown" ||
        s === "installed_unhealthy") &&
      tool.capabilities.health_checkable
    ) {
      return { kind: "recheck", label: busyRecheck ? (i18n.t("tc.state.scanning") as TranslationKey) : (i18n.t("tc.op.health_check") as TranslationKey) };
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
      aria-label={i18n.t("tc.drawer.close_panel") as TranslationKey}
      onclick={onclose}
    ></button>
    <div
      bind:this={drawerEl}
      class="drawer"
      role="dialog"
      aria-modal="true"
      aria-label={tool ? `${i18n.t("tc.drawer.version") as TranslationKey}: ${tool.display}` : (i18n.t("tc.drawer.no_data") as TranslationKey)}
      tabindex="-1"
    >
      {#if !tool}
        <header class="head">
          <h2 class="title">{i18n.t("tc.drawer.no_data") as TranslationKey}</h2>
          <IconButton icon="x" label={i18n.t("tc.drawer.close") as TranslationKey} size="sm" onclick={onclose} />
        </header>
        <p class="placeholder">
          {i18n.t("tc.drawer.not_scanned") as TranslationKey}
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
                <Badge tone="violet" dot>{i18n.t("tc.drawer.tracked") as TranslationKey}</Badge>
              {/if}
              {#if tool.category}
                <Badge tone="neutral">{tool.category}</Badge>
              {/if}
            </div>
          </div>
          <IconButton icon="x" label={i18n.t("tc.drawer.close") as TranslationKey} size="sm" onclick={onclose} />
        </header>

        {#if busyDetails}
          <p class="details-refresh" role="status">
            <Icon name="refresh" size={12} />
            {i18n.t("tc.drawer.refreshing") as TranslationKey}
          </p>
        {:else if detailsError}
          <p class="details-refresh details-error" role="alert">
            <Icon name="alert" size={12} />
            <span class="details-error-text">{detailsError}</span>
            <button
              type="button"
              class="details-retry"
              onclick={() => detailsRetry?.()}
              aria-label={i18n.t("tc.drawer.retry") as TranslationKey}
            >
              {i18n.t("tc.drawer.retry") as TranslationKey}
            </button>
          </p>
        {/if}

        <div class="body">
          {#if def?.description}
            <p class="description">{i18n.t(def.description as TranslationKey)}</p>
          {/if}

          <!-- ===== Версии ===== -->
          <section>
            <h3 class="section-title">{i18n.t("tc.drawer.version") as TranslationKey}</h3>
            <dl class="kv">
              <dt>{i18n.t("tc.drawer.installed") as TranslationKey}</dt>
              <dd>
                {#if version}
                  <span class="mono">{version.text}</span>
                  {#if !version.parsed}
                    <Badge tone="amber">{i18n.t("tc.drawer.raw_output") as TranslationKey}</Badge>
                  {/if}
                {:else}
                  —
                {/if}
              </dd>
              {#if recommended}
                <dt>{i18n.t("tc.drawer.recommended") as TranslationKey}</dt>
                <dd>{recommended}</dd>
              {/if}
              {#if assessment}
                <dt>{i18n.t("tc.drawer.assessment") as TranslationKey}</dt>
                <dd><Badge tone={assessment.tone}>{assessment.label}</Badge></dd>
              {/if}
            </dl>
            {#if tool.version_selected_because && tool.installs.length > 1}
              <p class="muted" title={tool.version_selected_because}>
                {tool.version_selected_because}
              </p>
            {/if}
          </section>

          <!-- ===== Обнаружение ===== -->
          <section>
            <h3 class="section-title">{i18n.t("tc.drawer.detection") as TranslationKey}</h3>
            {#if tool.detection.kind === "pending"}
              <p class="muted">{i18n.t("tc.drawer.not_checked_yet") as TranslationKey}</p>
            {:else if tool.detection.kind === "not_detected"}
              <p class="muted">{i18n.t("tc.drawer.no_installs") as TranslationKey}</p>
            {:else if tool.detection.kind === "detected"}
              <p class="muted">
                {i18n.t("tc.drawer.detected_count", { n: tool.installs.length }) as TranslationKey}
              </p>
            {:else if tool.detection.kind === "failed"}
              <p class="warn-text">{i18n.t("tc.drawer.check_failed", { reason: tool.detection.reason || (i18n.t("tc.state.unknown") as TranslationKey) }) as TranslationKey}</p>
            {:else}
              <!-- Неизвестный kind не роняет UI (контракт §8.1) -->
              <p class="muted">{i18n.t("tc.drawer.unknown_detection") as TranslationKey}</p>
            {/if}

            {#if tool.installs.length > 0}
              <ul class="installs">
                {#each tool.installs as inst, i (i)}
                  {@const scope = pathScopeInfo(inst.path_scope)}
                  <li class="install">
                    <div class="install-head">
                      <span class="mono">
                        {inst.parsed_version || inst.raw_version || "?"}
                        {#if !inst.parsed_version && inst.raw_version}
                          <span class="unparsed">{i18n.t("tc.drawer.raw_marker") as TranslationKey}</span>
                        {/if}
                      </span>
                      {#if tool.canonical_install === i}
                        <Badge tone="cyan">{i18n.t("tc.drawer.canonical") as TranslationKey}</Badge>
                      {/if}
                      <Badge tone={scope.tone}>{scope.label}</Badge>
                    </div>
                    <code class="path">{inst.location || (i18n.t("tc.drawer.path_unknown") as TranslationKey)}</code>
                    <div class="install-meta">
                      <span class="evidence">{evidenceLabel(inst.evidence?.kind ?? "")}</span>
                      {#if probeLogHasContent(inst.probe_log)}
                        <button
                          type="button"
                          class="copy-btn"
                          onclick={() => copyText(probeLogText(inst.probe_log), `probe-${i}`)}
                        >
                          {copiedKey === `probe-${i}` ? (i18n.t("tc.drawer.copied") as TranslationKey) : (i18n.t("tc.drawer.copy") as TranslationKey)}
                        </button>
                      {/if}
                    </div>
                    {#if inst.probe_log && probeLogHasContent(inst.probe_log)}
                      <!-- Полный лог пробы: единственная копия не усекается
                           CSS-эллипсисом — панель прокручивается целиком. -->
                      <details class="log-details">
                        <summary>{i18n.t("tc.drawer.full_probe_log") as TranslationKey}</summary>
                        <pre class="log-lines">{probeLogText(inst.probe_log)}</pre>
                      </details>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
            {#if tool.duration_ms > 0}
              <p class="muted">{i18n.t("tc.drawer.duration", { n: (tool.duration_ms / 1000).toFixed(1) }) as TranslationKey}</p>
            {/if}
          </section>

          <!-- ===== Здоровье ===== -->
          {#if health}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.health") as TranslationKey}</h3>
              <p class="health-line"><Badge tone={health.tone} dot>{health.label}</Badge></p>
              {#if tool.health && tool.health.results.length > 0}
                <ul class="checks">
                  {#each tool.health.results as check, ci (ci)}
                    {@const logText = healthCheckLogText(check)}
                    <li class="check">
                      <div class="check-head">
                        <Icon
                          name={check.passed ? "check" : check.process_failed ? "help" : "alert"}
                          size={13}
                          class={check.passed ? "ok" : "fail"}
                        />
                        <span class="check-label">{check.label}</span>
                        <span class="check-detail" title={logText}>{check.detail}</span>
                        {#if check.exit_code !== null && check.exit_code !== undefined}
                          <span class="check-code">{check.exit_code}</span>
                        {/if}
                        {#if check.timed_out}
                          <Badge tone="amber">{i18n.t("tc.drawer.timeout") as TranslationKey}</Badge>
                        {/if}
                        <span class="check-ms">{check.duration_ms}ms</span>
                      </div>
                      <div class="check-log-row">
                        <code class="path">{(check.command ?? []).join(" ")}</code>
                        <button
                          type="button"
                          class="copy-btn"
                          onclick={() => copyText(logText, `health-${ci}`)}
                        >
                          {copiedKey === `health-${ci}` ? (i18n.t("tc.drawer.copied") as TranslationKey) : (i18n.t("tc.drawer.copy") as TranslationKey)}
                        </button>
                      </div>
                      <!-- Полный лог проверки: stdout/stderr/код выхода/таймаут
                           целиком, без усечения в единственной копии. -->
                      <details class="log-details">
                        <summary>{i18n.t("tc.drawer.full_health_log") as TranslationKey}</summary>
                        <pre class="log-lines">{logText}</pre>
                      </details>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/if}

          <!-- ===== PATH ===== -->
          {#if tool.path_findings.length > 0}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.path") as TranslationKey}</h3>
              <ul class="findings">
                {#each tool.path_findings as f, i (i)}
                  <li class="finding">
                    <Icon name="alert" size={13} class="fail" />
                    <div>
                      <code class="path">{f.entry || (i18n.t("tc.drawer.path_entry") as TranslationKey)}</code>
                      <p class="finding-detail">{f.detail}</p>
                    </div>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          <!-- ===== Зависимости ===== -->
          <section>
              <h3 class="section-title">{i18n.t("tc.drawer.dependencies") as TranslationKey}</h3>
            <div class="chip-row">
              {#if def?.bundled_with}
                <Badge tone="cyan">{i18n.t("tc.drawer.bundled_with", { tool: def.bundled_with }) as TranslationKey}</Badge>
              {/if}
              {#each def?.dependencies ?? [] as dep (dep)}
                <Badge tone="neutral">{i18n.t("tc.drawer.requires", { tool: dep }) as TranslationKey}</Badge>
              {/each}
              {#each def?.conflicts ?? [] as conflict (conflict)}
                <Badge tone="red">{i18n.t("tc.drawer.conflicts_with", { tool: conflict }) as TranslationKey}</Badge>
              {/each}
              {#if dependents.length > 0}
                {#each dependents as dep (dep)}
                  <Badge tone="violet">{i18n.t("tc.drawer.needed_for", { tool: dep }) as TranslationKey}</Badge>
                {/each}
              {/if}
              {#if !def?.bundled_with && (def?.dependencies ?? []).length === 0 && (def?.conflicts ?? []).length === 0 && dependents.length === 0}
                <span class="muted">{i18n.t("tc.drawer.self_sufficient") as TranslationKey}</span>
              {/if}
            </div>
          </section>

          <!-- ===== Источники и целостность ===== -->
          {#if sourcesForOs.length > 0}
            <section>
              <h3 class="section-title">
                {i18n.t("tc.drawer.install_sources") as TranslationKey}{os ? ` · ${platformName(os)}` : ""}
              </h3>
              <ul class="sources">
                {#each sourcesForOs as src (src.description)}
                  <li class="source">
                    <span>{i18n.t(src.description as TranslationKey)}</span>
                    <Badge tone={src.verified ? "lime" : "amber"}>
                      {src.verified ? (i18n.t("tc.drawer.checksum") as TranslationKey) : (i18n.t("tc.drawer.no_checksum") as TranslationKey)}
                    </Badge>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}

          {#if (def?.aliases && def.aliases.length > 0) || (def?.platform_availability && def.platform_availability.length > 0)}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.catalog") as TranslationKey}</h3>
              <div class="chip-row">
                {#if def?.aliases && def.aliases.length > 0}
                  {#each def.aliases as alias (alias)}
                    <Badge tone="neutral">{i18n.t("tc.drawer.alias", { alias }) as TranslationKey}</Badge>
                  {/each}
                {/if}
                {#if def?.platform_availability && def.platform_availability.length > 0}
                  {#each def.platform_availability as p (p)}
                    <Badge tone={p === os ? "cyan" : "neutral"}>{platformName(p)}</Badge>
                  {/each}
                {/if}
              </div>
            </section>
          {/if}

          {#if def?.docker}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.docker") as TranslationKey}</h3>
              <p class="docker-line">
                <code class="mono-inline">{def.docker.image ?? (i18n.t("tc.drawer.no_image") as TranslationKey)}</code>
                {#if def.docker.notes}<span class="muted"> · {i18n.t(def.docker.notes as TranslationKey)}</span>{/if}
              </p>
            </section>
          {/if}

          <!-- ===== Служебное ===== -->
          <section>
            <h3 class="section-title">{i18n.t("tc.drawer.service") as TranslationKey}</h3>
            <dl class="kv">
              {#if def}
                <dt>{i18n.t("tc.drawer.download_size") as TranslationKey}</dt>
                <dd>{formatSizeMb(def.size_mb)}</dd>
              {/if}
              {#if def?.needs_admin}
                <dt>{i18n.t("tc.drawer.permissions") as TranslationKey}</dt>
                <dd><Badge tone="amber">{i18n.t("tc.drawer.needs_admin") as TranslationKey}</Badge></dd>
              {/if}
              {#if tool.capabilities.installable}
                <dt>{i18n.t("tc.drawer.installation") as TranslationKey}</dt>
                <dd><Badge tone="lime">{i18n.t("tc.drawer.supported") as TranslationKey}</Badge></dd>
              {:else if tool.applicability.kind === "manual_only"}
                <dt>{i18n.t("tc.drawer.installation") as TranslationKey}</dt>
                <dd><Badge tone="violet">{i18n.t("tc.drawer.manual_only") as TranslationKey}</Badge></dd>
              {:else if tool.applicability.kind === "unsupported_on_platform"}
                <dt>{i18n.t("tc.drawer.installation") as TranslationKey}</dt>
                <dd><Badge tone="neutral">{i18n.t("tc.drawer.unsupported_os") as TranslationKey}</Badge></dd>
              {:else if tool.state.kind === "docker_managed"}
                <dt>{i18n.t("tc.drawer.mode") as TranslationKey}</dt>
                <dd><Badge tone="blue">{i18n.t("tc.drawer.docker_managed") as TranslationKey}</Badge></dd>
              {/if}
            </dl>
          </section>

          {#if def?.notes}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.notes") as TranslationKey}</h3>
              <p class="notes">{i18n.t(def.notes as TranslationKey)}</p>
            </section>
          {/if}

          {#if tool.state.kind === "manual_install" && def?.manual_install}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.manual_install") as TranslationKey}</h3>
              <p class="notes warn-text">{i18n.t(def.manual_install as TranslationKey)}</p>
            </section>
          {/if}

          {#if def?.docs_url || def?.source_url}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.links") as TranslationKey}</h3>
              <div class="links">
                {#if def?.docs_url}
                  <a href={def.docs_url} target="_blank" rel="noreferrer noopener">
                    <Icon name="external" size={13} /> {i18n.t("tc.drawer.documentation") as TranslationKey}
                  </a>
                {/if}
                {#if def?.source_url}
                  <a href={def.source_url} target="_blank" rel="noreferrer noopener">
                    <Icon name="external" size={13} /> {i18n.t("tc.drawer.source_code") as TranslationKey}
                  </a>
                {/if}
              </div>
            </section>
          {/if}

          <!-- ===== Журнал ===== -->
          <section>
            <h3 class="section-title">{i18n.t("tc.drawer.job_log") as TranslationKey}</h3>
            {#if jobLogLines.length > 0}
              <details class="log-details">
                <summary>{jobLogLines.length} {i18n.t("tc.drawer.entries") as TranslationKey}</summary>
                <pre class="log-lines">{#each jobLogLines as line}{line.text}
{/each}</pre>
              </details>
            {:else}
              <p class="muted">{i18n.t("tc.drawer.no_operations") as TranslationKey}</p>
            {/if}
          </section>

          {#if tool.error}
            <section>
              <h3 class="section-title">{i18n.t("tc.drawer.last_error") as TranslationKey}</h3>
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
              {i18n.t("tc.op.health_check") as TranslationKey}
            </Button>
          {/if}
          <span class="spacer"></span>
          {#if !adopted && onadopt && (tool.provenance.kind === "external" || tool.provenance.kind === "unknown")}
            <Button
              variant="ghost"
              size="sm"
              label={i18n.t("tc.drawer.tracked") as TranslationKey}
              onclick={() => onadopt(tool!.tool_id)}
            >
              {i18n.t("tc.drawer.track") as TranslationKey}
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

  .install-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .unparsed {
    color: var(--sp-warning);
    font-size: var(--sp-fs-xs);
  }

  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .check {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    padding: var(--sp-1) 0;
    border-bottom: 1px dashed var(--sp-border-faint);
  }

  .check:last-child {
    border-bottom: none;
  }

  .check-head {
    display: grid;
    grid-template-columns: auto auto 1fr auto auto auto;
    align-items: baseline;
    gap: var(--sp-2);
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

  /* Компактная сводка может усекаться — ПОЛНЫЙ лог всегда доступен
     ниже в разворачиваемой панели (ellipsis не единственная копия). */
  .check-code {
    color: var(--sp-text-3);
    font-family: var(--sp-font-mono);
    white-space: nowrap;
  }

  .check-ms {
    color: var(--sp-text-3);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .check-log-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .copy-btn {
    border: 1px solid var(--sp-border);
    background: transparent;
    color: var(--sp-text-3);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
    padding: 0 var(--sp-2);
    cursor: pointer;
    white-space: nowrap;
  }

  .copy-btn:hover {
    color: var(--sp-text-1);
    border-color: var(--sp-border-strong);
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

  .details-refresh {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    padding: var(--sp-2) var(--sp-5);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    background: rgba(34, 211, 238, 0.06);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .details-error {
    color: var(--sp-danger);
    background: rgba(248, 113, 113, 0.07);
  }

  .details-error-text {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .details-retry {
    flex: 0 0 auto;
    padding: var(--sp-1) var(--sp-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--sp-fs-xs);
    cursor: pointer;
  }

  .details-retry:hover {
    background: rgba(248, 113, 113, 0.12);
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
