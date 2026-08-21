<script lang="ts">
  // Карточка инструмента каталога. Действия правдивы относительно
  // возможностей платформы (capabilities бэкенда); мутации НЕ запускаются
  // из клика — они открывают экран проверки плана.
  import Badge from "$lib/components/ui/Badge.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import StateBadge from "./StateBadge.svelte";
  import type { OperationKind, ToolDefinition, ToolScanResult } from "../types";
  import {
    formatSizeMb,
    provenanceInfo,
    toolStateVersion,
  } from "../format";

  export type CardPlanOp = "install" | "update" | "repair_path";

  let {
    tool,
    def = null,
    busy = false,
    ondetails,
    onplan,
    onrecheck,
  }: {
    tool: ToolScanResult;
    /** Метаданные каталога (описание/размер/права). Может отсутствовать. */
    def?: ToolDefinition | null;
    busy?: boolean;
    ondetails: (toolId: string) => void;
    onplan: (operation: CardPlanOp, toolId: string) => void;
    onrecheck: (toolId: string) => void;
  } = $props();

  type PrimaryAction =
    | { kind: "plan"; operation: CardPlanOp; label: string }
    | { kind: "recheck"; label: string }
    | { kind: "details"; label: string }
    | null;

  const action = $derived.by<PrimaryAction>(() => {
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
      return { kind: "recheck", label: busy ? "Проверка…" : "Проверить" };
    }
    if (
      (s === "manual_install" || s === "unsupported_platform") &&
      tool.capabilities.manual_instructions_available
    ) {
      return { kind: "details", label: "Инструкция" };
    }
    return null;
  });

  const version = $derived(toolStateVersion(tool.state));
  const updateTarget = $derived(
    tool.state.kind === "update_available" ? tool.state.recommended : null,
  );
  const provenance = $derived(provenanceInfo(tool.provenance));
  const hasPathProblem = $derived(
    tool.path_findings.length > 0 || tool.state.kind === "path_broken",
  );
</script>

<article class="card">
  <header class="head">
    <TechIcon icon={tool.icon} alt="" size="md" />
    <div class="title-wrap">
      <button type="button" class="title-btn" onclick={() => ondetails(tool.tool_id)}>
        <span class="title">{tool.display}</span>
      </button>
      <span class="category">{tool.category || def?.category || ""}</span>
    </div>
    <StateBadge state={tool.state} />
  </header>

  {#if def?.description}
    <p class="description">{def.description}</p>
  {/if}

  <dl class="meta">
    <div class="meta-item">
      <dt>Версия</dt>
      <dd>
        {#if updateTarget}
          <span class="update-line">
            {version || "?"}
            <span aria-hidden="true">→</span>
            <span class="update-target">{updateTarget}</span>
          </span>
        {:else}
          {version ?? (tool.state.kind === "docker_managed" ? "docker-compose" : "—")}
        {/if}
      </dd>
    </div>
    <div class="meta-item" title={provenance.label}>
      <dt>Источник</dt>
      <dd class="ellipsis">{provenance.label}</dd>
    </div>
    <div class="meta-item">
      <dt>Размер</dt>
      <dd>{def ? formatSizeMb(def.size_mb) : "—"}</dd>
    </div>
  </dl>

  <div class="flags">
    {#if def?.needs_admin}
      <Badge tone="amber">нужны права администратора</Badge>
    {/if}
    {#if hasPathProblem}
      <Badge tone="red">PATH</Badge>
    {/if}
    {#if tool.health?.state.kind === "unhealthy"}
      <Badge tone="red" dot>проверки не проходят</Badge>
    {:else if tool.health?.state.kind === "degraded"}
      <Badge tone="amber" dot>деградация</Badge>
    {:else if tool.health?.state.kind === "healthy"}
      <Badge tone="lime" dot>проверки пройдены</Badge>
    {/if}
  </div>

  <footer class="foot">
    {#if action?.kind === "plan"}
      <Button
        variant={action.operation === "repair_path" ? "secondary" : "primary"}
        size="sm"
        onclick={() => onplan(action.operation, tool.tool_id)}
      >
        {action.label}
      </Button>
    {:else if action?.kind === "recheck"}
      <Button
        variant="secondary"
        size="sm"
        loading={busy}
        onclick={() => onrecheck(tool.tool_id)}
      >
        {action.label}
      </Button>
    {:else if action?.kind === "details"}
      <Button variant="secondary" size="sm" onclick={() => ondetails(tool.tool_id)}>
        {action.label}
      </Button>
    {:else}
      <Button variant="ghost" size="sm" onclick={() => ondetails(tool.tool_id)}>
        Подробнее
      </Button>
    {/if}
    <span class="spacer"></span>
    <IconButton
      icon="chevronRight"
      label={`Подробнее о ${tool.display}`}
      size="sm"
      onclick={() => ondetails(tool.tool_id)}
    />
  </footer>
</article>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    height: 100%;
    padding: var(--sp-4);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-glass-bg);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    box-shadow: var(--sp-shadow-1);
    transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
    min-width: 0;
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
  }

  .meta {
    display: grid;
    grid-template-columns: repeat(3, auto);
    justify-content: start;
    gap: var(--sp-2) var(--sp-5);
    margin: 0;
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
  }

  .update-line {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    color: var(--sp-amber);
  }

  .update-target {
    font-weight: var(--sp-fw-semibold);
  }

  .ellipsis {
    max-width: 11rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--sp-font-sans) !important;
  }

  .flags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
    min-height: 1.25rem;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: auto;
  }

  .spacer {
    flex: 1 1 auto;
  }
</style>
