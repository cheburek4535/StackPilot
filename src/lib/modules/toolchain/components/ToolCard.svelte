<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  // Компактная строка инструмента каталога. Действия правдивы относительно
  // возможностей платформы (capabilities бэкенда); мутации НЕ запускаются
  // из клика — они открывают экран проверки плана.
  import Button from "$lib/components/ui/Button.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import type { OperationKind, ToolDefinition, ToolScanResult } from "../types";
  import {
    formatSizeMb,
    provenanceInfo,
    toolStateVersion,
    toolVersionDisplay,
  } from "../format";

  export type CardPlanOp = "install" | "update" | "repair_path";

  let {
    tool,
    def = null,
    busy = false,
    ondetails,
    onplan,
    onrecheck,
    onuninstall,
  }: {
    tool: ToolScanResult;
    /** Метаданные каталога (описание/размер/права). Может отсутствовать. */
    def?: ToolDefinition | null;
    busy?: boolean;
    ondetails: (toolId: string) => void;
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
    onuninstall?: (toolId: string) => void;
  } = $props();

  type PrimaryAction =
    | { kind: "plan"; operation: CardPlanOp; label: string }
    | { kind: "recheck"; label: string }
    | null;

  const action = $derived.by<PrimaryAction>(() => {
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
      return { kind: "recheck", label: busy ? (i18n.t("tc.state.scanning") as TranslationKey) : (i18n.t("tc.op.health_check") as TranslationKey) };
    }
    return null;
  });

  type StatusLine = { tone: string; filled: boolean; label: string; title: string };

  const status = $derived.by<StatusLine>(() => {
    const s = tool.state.kind;
    const base = (tone: string, filled: boolean, label: string, title?: string): StatusLine => ({
      tone,
      filled,
      label,
      title: title ?? label,
    });
    switch (s) {
      case "update_available":
        return base("amber", true, i18n.t("tc.card.status.update_to", { version: tool.state.recommended }) as TranslationKey);
      case "installed_healthy":
        return base("lime", true, i18n.t("tc.state.installed") as TranslationKey);
      case "installed_health_unknown":
        return base("cyan", true, i18n.t("tc.state.health_unknown") as TranslationKey);
      case "installed_unhealthy":
        return base("red", true, i18n.t("tc.state.unhealthy") as TranslationKey);
      case "path_broken":
        return base("red", true, i18n.t("tc.state.path_broken") as TranslationKey, i18n.t("tc.card.path_warning") as TranslationKey);
      case "missing":
        if (tool.bundled_with) {
          return base(
            "cyan",
            true,
            i18n.t("tc.install.bundled_with", { tool: tool.bundled_with }) as TranslationKey,
          );
        }
        return base("neutral", false, i18n.t("tc.state.missing") as TranslationKey);
      case "scan_pending":
        return base("cyan", false, i18n.t("tc.state.scanning") as TranslationKey);
      case "scan_failed":
        return base("amber", true, i18n.t("tc.state.scan_error") as TranslationKey);
      case "manual_install":
        return base("violet", true, i18n.t("tc.state.manual") as TranslationKey);
      case "docker_managed":
        return base("blue", true, i18n.t("tc.state.docker") as TranslationKey);
      case "built_in_system":
        return base("neutral", true, i18n.t("tc.state.builtin") as TranslationKey);
      case "unsupported_platform":
        return base("neutral", true, i18n.t("tc.state.unsupported") as TranslationKey);
      default:
        return base("neutral", false, i18n.t("tc.state.unknown") as TranslationKey);
    }
  });

  const version = $derived(toolStateVersion(tool.state));
  const versionInfo = $derived(toolVersionDisplay(tool));
  const updateTarget = $derived(
    tool.state.kind === "update_available" ? tool.state.recommended : null,
  );
  /** Установлен, но версии нет ни в состоянии, ни в уликах: честное
   *  объяснение вместо молчаливого «—» (контракт: версия или явная
   *  пометка «не распознана»). */
  const versionUnknownExplanation = $derived.by(() => {
    const installed =
      tool.state.kind === "installed_healthy" ||
      tool.state.kind === "installed_health_unknown" ||
      tool.state.kind === "installed_unhealthy";
    if (!installed || version || versionInfo || updateTarget) return null;
    return i18n.t("tc.install.version_unknown") as TranslationKey;
  });
  const recommended = $derived(def?.versions?.recommended ?? null);
  const provenance = $derived(provenanceInfo(tool.provenance));
  const sizeText = $derived(def ? formatSizeMb(def.size_mb) : null);
  const hasPathProblem = $derived(
    tool.path_findings.length > 0 || tool.state.kind === "path_broken",
  );

  const currentVersionText = $derived.by(() => {
    if (version) return version;
    if (versionInfo) return `${versionInfo.text}${versionInfo.parsed ? "" : "*"}`;
    if (versionUnknownExplanation) return versionUnknownExplanation;
    return tool.state.kind === "docker_managed" ? "docker-compose" : "—";
  });
</script>

<article class="row">
  <div class="main">
    <TechIcon icon={tool.icon} alt="" size="sm" class="tool-icon" />
    <div class="title-wrap">
      <button type="button" class="title-btn" onclick={() => ondetails(tool.tool_id)}>
        <span class="title">{tool.display}</span>
      </button>
      <span class="sub">
        <span class="category">{tool.category || def?.category || ""}</span>
        {#if provenance.label}
          <span class="sub-sep" aria-hidden="true">·</span>
          <span>{provenance.label}</span>
        {/if}
        {#if sizeText}
          <span class="sub-sep" aria-hidden="true">·</span>
          <span>{sizeText}</span>
        {/if}
      </span>
    </div>
  </div>

  <div class="versions">
    {#if updateTarget}
      <span class="v-cur mono">{currentVersionText}</span>
      <span class="v-arrow" aria-hidden="true">→</span>
      <span class="v-target mono">{updateTarget}</span>
    {:else}
      <span class="v-cur mono">{currentVersionText}</span>
      {#if recommended}
        <span class="v-rec">
          <span class="rec-label">{i18n.t("tc.install.recommended") as TranslationKey}:</span>
          <span class="mono rec-value">{recommended}</span>
        </span>
      {/if}
    {/if}
  </div>

  <div
    class={"status st-" + status.tone}
    class:st-hollow={!status.filled}
    title={status.title}
  >
    <span class="dot" aria-hidden="true"></span>
    <span class="status-text">{status.label}</span>
  </div>

  <div class="actions">
    {#if action?.kind === "plan"}
      <Button
        variant={action.operation === "repair_path" ? "secondary" : "primary"}
        size="sm"
        onclick={() => onplan(action.operation, tool.tool_id)}
      >
        {action.label}
      </Button>
    {:else if action?.kind === "recheck"}
      <IconButton
        icon="refresh"
        label={action.label}
        size="sm"
        disabled={busy}
        onclick={() => onrecheck(tool.tool_id)}
      />
    {/if}
    {#if onuninstall && tool.state.kind !== "missing" && tool.state.kind !== "unsupported_platform" && tool.state.kind !== "built_in_system" && tool.state.kind !== "docker_managed"}
      <IconButton icon="trash" label={i18n.t("tc.uninstall.confirm", { tool: tool.display }) as TranslationKey} variant="ghost" onclick={() => onuninstall?.(tool.tool_id)} />
    {/if}
    {#if hasPathProblem && tool.state.kind !== "path_broken"}
      <IconButton
        icon="alert"
        label={i18n.t("tc.card.path_warning") as TranslationKey}
        size="sm"
        variant="ghost"
        onclick={() => ondetails(tool.tool_id)}
      />
    {/if}
    <IconButton
      icon="chevronRight"
      label={i18n.t("tc.card.more_info", { name: tool.display }) as TranslationKey}
      size="sm"
      onclick={() => ondetails(tool.tool_id)}
    />
  </div>
</article>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border-faint);
    border-radius: var(--sp-radius-md);
    transition: border-color 0.15s ease, background-color 0.15s ease;
    min-width: 0;
  }

  .row:hover {
    border-color: var(--sp-border);
    background: var(--sp-bg-2);
  }

  .main {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 1 auto;
    min-width: 13rem;
    max-width: 100%;
  }

  .tool-icon {
    width: 24px !important;
    height: 24px !important;
    flex: 0 0 auto;
  }

  .title-wrap {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
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
    font-size: 0.875rem;
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .category {
    text-transform: capitalize;
  }

  .sub-sep {
    opacity: 0.6;
  }

  .versions {
    display: flex;
    align-items: baseline;
    gap: var(--sp-1);
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }

  .mono {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
  }

  .v-cur {
    color: var(--sp-text-1);
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .v-arrow {
    color: var(--sp-text-3);
    flex: 0 0 auto;
  }

  .v-target {
    color: var(--sp-amber);
    font-weight: var(--sp-fw-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .v-rec {
    display: inline-flex;
    align-items: baseline;
    gap: var(--sp-1);
    margin-left: var(--sp-3);
    min-width: 0;
  }

  .rec-label {
    color: var(--sp-text-3);
  }

  .rec-value {
    color: var(--sp-text-2);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
    min-width: 8rem;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    white-space: nowrap;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--sp-radius-full);
    background: var(--sp-text-3);
    flex: 0 0 auto;
  }

  .st-hollow .dot {
    background: transparent;
    border: 1px solid var(--sp-text-3);
  }

  .status-text {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .st-lime .dot { background: var(--sp-lime); }
  .st-lime { color: var(--sp-lime); }
  .st-cyan .dot { background: var(--sp-cyan); }
  .st-cyan { color: var(--sp-cyan); }
  .st-amber .dot { background: var(--sp-amber); }
  .st-amber { color: var(--sp-amber); }
  .st-red .dot { background: var(--sp-danger); }
  .st-red { color: var(--sp-danger); }
  .st-violet .dot { background: var(--sp-violet); }
  .st-violet { color: var(--sp-violet); }
  .st-blue .dot { background: var(--sp-blue); }
  .st-blue { color: var(--sp-blue); }
  .st-neutral { color: var(--sp-text-2); }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex: 0 0 auto;
    min-width: 0;
  }

  .actions :global(button) {
    min-width: 0;
  }

  @media (max-width: 900px) {
    .row {
      flex-wrap: wrap;
    }

    .versions {
      order: 3;
      flex-basis: 100%;
      padding-left: calc(var(--sp-2) + 24px + var(--sp-2));
    }

    .status {
      margin-left: auto;
    }
  }
</style>