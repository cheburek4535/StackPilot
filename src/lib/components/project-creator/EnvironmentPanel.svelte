<script lang="ts">
  import TechIcon from "$lib/components/TechIcon.svelte";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import {
    helpMode,
    helpProgress,
    isHintVisible,
    HINT_ENV_AUTO_INSTALL,
  } from "$lib/core/help";
  import { statusKind, statusLabel, taskStateKind, taskStateLabel } from "$lib/modules/toolchain/compat";
  import type {
    EnvironmentCheck,
    InstallPlan,
    CheckProgressEvent,
    TaskState,
    ToolRequirement,
  } from "$lib/modules/toolchain/compat";
  import type { WizardTreeData } from "$lib/modules/project_creator/types";
  import type { ToolVersionCandidate } from "$lib/modules/toolchain/types";
  import Modal from "$lib/components/ui/Modal.svelte";
  import { listEnvironments } from "$lib/modules/project_environment/api";
  import type { EnvironmentBinding } from "$lib/modules/project_environment/types";

  let {
    tree,
    envCheck,
    envChecking,
    envCheckProgress,
    envSelectedIds,
    envLocalInfra,
    envPlan,
    envInstalling,
    envInstallDone,
    envErrors,
    envLogs,
    envTaskStates,
    envRestartHint,
    envDownload,
    envSpeed,
    envPhaseStart,
    envNow,
    envRechecking,
    envError,
    newSecrets,
    secretCopied,
    installedTools,
    envIsolationMode = "isolated",
    envSubMode = "new",
    envReuseTarget = null,
    envAvailableVersions = {},
    envSelectedVersions = {},
    envLoadingVersions = {},
    onselectToolVersion,
    onchangeIsolationMode,
    onchangeSubMode,
    onselectReuseTarget,
    ontoggleEnvTool,
    onselectAll,
    onoptInLocalInfra,
    onrevertLocalInfra,
    onstartInstall,
    oncancelInstall,
    onrecheck,
    oncheck,
    oncontinue,
    onback,
    onbackReview,
    oncopySecret,
    ondismissSecrets,
  }: {
    tree: WizardTreeData | null;
    envCheck: EnvironmentCheck | null;
    envChecking: boolean;
    envCheckProgress: CheckProgressEvent[];
    envSelectedIds: Set<string>;
    envLocalInfra: Set<string>;
    envPlan: InstallPlan | null;
    envInstalling: boolean;
    envInstallDone: boolean;
    envErrors: string[];
    envLogs: string[];
    envTaskStates: Map<string, TaskState>;
    envRestartHint: boolean;
    envDownload: Map<string, { received: number; total: number }>;
    envSpeed: Map<string, number>;
    envPhaseStart: Map<string, number>;
    envNow: number;
    envRechecking: boolean;
    envError: string | null;
    newSecrets: Record<string, string> | null;
    secretCopied: string | null;
    installedTools: Set<string>;
    envIsolationMode?: "isolated" | "global";
    envSubMode?: "new" | "existing";
    envReuseTarget?: EnvironmentBinding | null;
    envAvailableVersions?: Record<string, ToolVersionCandidate[]>;
    envSelectedVersions?: Record<string, ToolVersionCandidate>;
    envLoadingVersions?: Record<string, boolean>;
    onselectToolVersion?: (toolId: string, candidate: ToolVersionCandidate) => void;
    onchangeIsolationMode?: (mode: "isolated" | "global") => void;
    onchangeSubMode?: (mode: "new" | "existing") => void;
    onselectReuseTarget?: (target: EnvironmentBinding | null) => void;
    ontoggleEnvTool: (toolId: string) => void;
    onselectAll: () => void;
    onoptInLocalInfra: (toolId: string) => void;
    onrevertLocalInfra: (toolId: string) => void;
    onstartInstall: () => void;
    oncancelInstall: () => void;
    onrecheck: () => void;
    oncheck: () => void;
    oncontinue: () => void;
    onback: () => void;
    onbackReview: () => void;
    oncopySecret: (key: string, value: string) => void;
    ondismissSecrets: () => void;
  } = $props();

  /** Highlight the install CTA while the beginner hint is active. */
  const envHintVisible = $derived(
    isHintVisible($helpProgress, $helpMode, HINT_ENV_AUTO_INSTALL),
  );

  /** Иконка тула из wizard_tree по tool_id (для requirements/tasks окружения) */
  function sourceKindBadge(kind: string): { label: string; tone: "primary" | "secondary" | "accent" | "success" | "warning" | "danger" | "ghost" | "cyan" | "violet" | "amber" | "emerald" } {
    switch (kind) {
      case "shared_store":
        return { label: "StackPilot Store", tone: "violet" };
      case "fnm":
        return { label: "fnm", tone: "cyan" };
      case "nvm":
        return { label: "nvm", tone: "cyan" };
      case "uv":
        return { label: "uv", tone: "emerald" };
      case "pyenv":
        return { label: "pyenv", tone: "emerald" };
      case "windows_py_launcher":
        return { label: "py", tone: "primary" };
      case "system":
        return { label: "System PATH", tone: "secondary" };
      case "standalone_download":
        return { label: "Standalone", tone: "amber" };
      default:
        return { label: kind, tone: "secondary" };
    }
  }

  function toolIcon(toolId: string): string | null {
    return tree?.tools.find((t) => t.id === toolId)?.icon ?? null;
  }

  function allMissingTools(): ToolRequirement[] {
    return (
      envCheck?.requirements.filter(
        (r) => statusKind(r.status) !== "ok" && statusKind(r.status) !== "manual",
      ) ?? []
    );
  }

  function selectedMissingTools(): ToolRequirement[] {
    return allMissingTools().filter((r) => envSelectedIds.has(r.tool_id));
  }

  /** Инструмент установлен локально: заявлен в общем стейте приложения
   *  (state.json) или только что подтверждён текущей проверкой окружения. */
  function isLocallyInstalled(toolId: string): boolean {
    if (installedTools.has(toolId)) return true;
    return (
      envCheck?.requirements.some(
        (r) => r.tool_id === toolId && statusKind(r.status) === "ok",
      ) ?? false
    );
  }

  function formatMb(mb: number): string {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)}${i18n.t("create.format.gb")}`;
    return `${mb}${i18n.t("create.format.mb")}`;
  }

  function downloadStatus(taskId: string): string {
    const dl = envDownload.get(taskId);
    if (!dl || dl.total <= 0) return "";
    const percent = Math.min(100, Math.round((dl.received / dl.total) * 100));
    return `${percent}% (${formatMb(Math.floor(dl.received / 1024 / 1024))} / ${formatMb(Math.floor(dl.total / 1024 / 1024))})`;
  }

  function dlPercent(taskId: string): number | null {
    const dl = envDownload.get(taskId);
    if (!dl || dl.total <= 0) return null;
    return Math.min(100, Math.round((dl.received / dl.total) * 100));
  }

  /** Мгновенная скорость скачивания (байт/с → МБ/с или КБ/с). */
  function speedLabel(taskId: string): string {
    const s = envSpeed.get(taskId);
    if (!s || s <= 0) return "";
    if (s >= 1024 * 1024) {
      return i18n.t("create.speed_mb", { speed: (s / 1024 / 1024).toFixed(1) });
    }
    return i18n.t("create.speed_kb", { speed: Math.max(1, Math.round(s / 1024)) });
  }

  /** Таймеры фаз живут на envNow (тикер страницы, раз в секунду) — время
   *  течёт, даже когда от установщика не приходит ни одного события. */
  function phaseElapsed(taskId: string): string {
    const started = envPhaseStart.get(taskId);
    if (!started) return "…";
    const secs = Math.max(0, Math.floor((envNow - started) / 1000));
    if (secs < 60) return i18n.t("create.phase_elapsed_secs", { secs });
    return i18n.t("create.phase_elapsed_min", { m: Math.floor(secs / 60), s: secs % 60 });
  }

  // Живой журнал установки: открыт по умолчанию, автоскролл к последней
  // строке на каждом обновлении envLogs.
  let logBox: HTMLDivElement | null = null;
  let logOpen = $state(true);
  $effect(() => {
    void envLogs.length;
    if (!logOpen || !logBox) return;
    logBox.scrollTop = logBox.scrollHeight;
  });

  function secretToolName(toolId: string): string {
    const req = envCheck?.requirements.find((r) => r.tool_id === toolId);
    return req?.display ?? toolId;
  }
  let showEnvPicker = $state(false);
  let availableEnvList = $state<EnvironmentBinding[]>([]);
  let envLoadingList = $state(false);
  let envFilterQuery = $state("");

  async function openEnvPicker() {
    showEnvPicker = true;
    envLoadingList = true;
    try {
      const list = await listEnvironments();
      availableEnvList = list;
    } catch (e) {
      console.error("Failed to load environments:", e);
      availableEnvList = [];
    } finally {
      envLoadingList = false;
    }
  }

  function selectEnvironment(env: EnvironmentBinding) {
    onchangeSubMode?.("existing");
    onselectReuseTarget?.(env);
    showEnvPicker = false;
  }

  function projectCountWord(n: number): string {
    const mod10 = n % 10;
    const mod100 = n % 100;
    if (mod100 >= 11 && mod100 <= 19) return "проектов";
    if (mod10 === 1) return "проект";
    if (mod10 >= 2 && mod10 <= 4) return "проекта";
    return "проектов";
  }
</script>

<p class="prompt">{i18n.t("create.env_check") as TranslationKey}</p>
<HelpHint
  id={HINT_ENV_AUTO_INSTALL.id}
  resolvedBy={HINT_ENV_AUTO_INSTALL.resolvedBy}
  icon="sparkles"
  title={i18n.t("help.env_auto_install.title") as TranslationKey}
  text={i18n.t("help.env_auto_install.body") as TranslationKey}
/>
<p class="hint">{i18n.t("create.env_check_desc") as TranslationKey}</p>

<!-- Compact Environment Mode Segmented Bar -->
<div class="env-mode-strip">
  <div class="env-mode-left">
    <div class="env-mode-meta">
      <Icon name="package" size={14} />
      <span class="env-mode-label">{i18n.t("create.env_isolation_title" as TranslationKey) || "Режим окружения"}:</span>
    </div>
    <div class="env-mode-pills" role="radiogroup" aria-label="Environment Mode">
      <button
        type="button"
        class="env-pill-btn"
        class:active={envIsolationMode === "isolated"}
        onclick={() => onchangeIsolationMode?.("isolated")}
      >
        <span class="env-pill-icon">⚡</span>
        <span class="env-pill-text">{i18n.t("create.env_mode_isolated" as TranslationKey) || "Sandbox"}</span>
        <span class="env-pill-tag">{i18n.t("create.env_mode_isolated_tag" as TranslationKey) || "Rec"}</span>
      </button>
      <button
        type="button"
        class="env-pill-btn"
        class:active={envIsolationMode === "global"}
        onclick={() => onchangeIsolationMode?.("global")}
      >
        <span class="env-pill-icon">🌐</span>
        <span class="env-pill-text">{i18n.t("create.env_mode_global" as TranslationKey) || "Global PATH"}</span>
      </button>
    </div>
  </div>

  <div class="env-mode-hint-inline">
    {#if envIsolationMode === "isolated"}
      <span class="hint-dot ok"></span>
      <span class="hint-text">{i18n.t("create.env_mode_isolated_summary" as TranslationKey) || "Изолированная песочница проекта (чистый PATH Windows)"}</span>
    {:else}
      <span class="hint-dot warn"></span>
      <span class="hint-text">{i18n.t("create.env_mode_global_summary" as TranslationKey) || "Системные версии из PATH хоста"}</span>
    {/if}
  </div>
</div>

{#if envIsolationMode === "isolated"}
  <div class="env-submode-strip">
    <div class="env-submode-left">
      <span class="env-submode-label">Песочница:</span>
      <div class="env-submode-pills" role="radiogroup" aria-label="Target Sandbox">
        <button
          type="button"
          class="env-subpill-btn"
          class:active={envSubMode === "new"}
          onclick={() => onchangeSubMode?.("new")}
        >
          <span class="subpill-icon">➕</span>
          <span>Новая среда</span>
        </button>
        <button
          type="button"
          class="env-subpill-btn"
          class:active={envSubMode === "existing"}
          onclick={() => {
            onchangeSubMode?.("existing");
            if (!envReuseTarget) {
              openEnvPicker();
            }
          }}
        >
          <Icon name="folder" size={13} />
          <span>Существующая среда</span>
        </button>
      </div>

      {#if envSubMode === "existing"}
        {#if envReuseTarget}
          <div class="env-target-badge">
            <span class="env-target-dot"></span>
            <span class="env-target-name">{envReuseTarget.name || envReuseTarget.binding_id}</span>
            <span class="env-target-count">({(envReuseTarget.bound_projects || []).length} {projectCountWord((envReuseTarget.bound_projects || []).length)})</span>
            <button
              type="button"
              class="btn-change-env"
              onclick={() => openEnvPicker()}
              title="Сменить песочницу"
            >
              Сменить
            </button>
          </div>
        {:else}
          <button
            type="button"
            class="btn-pick-env"
            onclick={() => openEnvPicker()}
          >
            <Icon name="search" size={12} />
            <span>Выбрать среду...</span>
          </button>
        {/if}
      {/if}
    </div>

    <div class="env-advice-box">
      <span class="advice-icon">💡</span>
      <span class="advice-text">
        <strong>Совет:</strong> песочница разделяет общие бинарники тулчейнов (Node, Python) без дублирования на диске, но зависимости проекта (<code>venv</code>, <code>node_modules</code>) изолированы отдельно.
      </span>
    </div>
  </div>
{/if}

{#if envChecking}
  <p class="muted">{i18n.t("create.checking_tools") as TranslationKey}</p>
  {#if envCheckProgress.length > 0}
    <div class="env-progress-list">
      {#each envCheckProgress as ev}
        <div class="env-progress-row">
          <span class="env-icon">{statusKind(ev.status) === "ok" ? "✅" : "🔍"}</span>
          <TechIcon icon={ev.icon ?? toolIcon(ev.tool_id)} alt={ev.display} size="sm" />
          <span class="env-name">{ev.display}</span>
          <span class="env-status muted">
            {statusKind(ev.status) === "ok"
              ? `✓ ${(ev.status as { Installed: { version: string } }).Installed.version}`
              : (i18n.t("create.checking") as TranslationKey)}
          </span>
        </div>
      {/each}
    </div>
  {/if}
{:else if envCheck}
  {@const missingAll = allMissingTools()}
  {@const missingSelected = selectedMissingTools()}
  {#if envInstallDone && envPlan}
    {#if envRechecking}
      <p class="env-rechecking"><span class="spin" aria-hidden="true"></span> {i18n.t("create.env_rechecking") as TranslationKey}</p>
    {/if}
    <div class="env-summary">
      <span>{i18n.t("create.installed_count", { x: envPlan.tasks.filter((t) => taskStateKind(t.state) === "success").length, y: envPlan.tasks.length }) as TranslationKey}</span>
      <span>{i18n.t("create.failed_count", { x: envPlan.tasks.filter((t) => taskStateKind(t.state) === "failed").length }) as TranslationKey}</span>
      {#if envErrors.length > 0}
        <span class="env-warn">{i18n.t("create.errors_count", { n: envErrors.length }) as TranslationKey}</span>
      {/if}
    </div>
    <div class="env-install">
      <p class="group-label">{i18n.t("create.install_finished") as TranslationKey}</p>
      {#each envPlan.tasks as task}
        {@const st = envTaskStates.get(task.task_id) ?? task.state}
        <div class="env-row">
          <span class="env-icon">
            {#if taskStateKind(st) === "success"}✅
            {:else if taskStateKind(st) === "failed"}❌
            {:else if taskStateKind(st) === "skipped"}⏭️
            {:else}•{/if}
          </span>
          <TechIcon icon={task.icon ?? toolIcon(task.tool_id)} alt={task.display} size="sm" />
          <span class="env-name">{task.display}</span>
          <span class="env-source">{task.size_mb} MB · {i18n.t(task.source_description as TranslationKey)}</span>
          <span
            class="env-status"
            class:ok={taskStateKind(st) === "success"}
            class:broken={taskStateKind(st) === "failed"}
            class:missing={taskStateKind(st) === "skipped"}
          >{taskStateLabel(st)}</span>
        </div>
      {/each}
      {#if envErrors.length > 0}
        <details class="exec-full-log">
          <summary>{i18n.t("create.env_errors", { n: envErrors.length }) as TranslationKey}</summary>
          <pre>{envErrors.join("\n")}</pre>
        </details>
      {/if}
      {#if envLogs.length > 0}
        <details class="exec-full-log">
          <summary>{i18n.t("create.log_lines", { n: envLogs.length }) as TranslationKey}</summary>
          <pre>{envLogs.join("\n")}</pre>
        </details>
      {/if}
    </div>
    <div class="btn-row">
      <button class="btn-back" onclick={onbackReview}>{i18n.t("create.back") as TranslationKey}</button>
      <button class="btn-secondary" onclick={onrecheck}>{i18n.t("create.recheck_env") as TranslationKey}</button>
      <button class="btn-primary" onclick={oncontinue}>{i18n.t("create.continue") as TranslationKey}</button>
    </div>
    {#if envRestartHint}
      <p class="env-warn">{i18n.t("create.path_restart_hint") as TranslationKey}</p>
    {/if}
  {:else}
    {#if envIsolationMode === "isolated" && envSubMode === "existing"}
    {#if envReuseTarget}
      <div class="env-reused-card">
        <div class="env-reused-head">
          <div class="env-reused-title-row">
            <span class="reused-icon">⚡</span>
            <div>
              <h4 class="reused-title">{envReuseTarget.name || envReuseTarget.binding_id}</h4>
              <p class="reused-subtitle">ID: {envReuseTarget.binding_id} · Проект будет подключен к этой песочнице</p>
            </div>
          </div>
          <button type="button" class="btn-change-reused" onclick={() => openEnvPicker()}>
            Сменить песочницу
          </button>
        </div>

        <div class="env-reused-tools-section">
          <span class="reused-section-title">Готовые тулчейны окружения:</span>
          <div class="reused-tools-chips">
            {#each Object.entries(envReuseTarget.tool_overrides || {}) as [tid, ov]}
              <div class="reused-tool-chip">
                <TechIcon icon={toolIcon(tid)} alt={tid} size="xs" />
                <span class="chip-name">{tid}</span>
                <span class="chip-ver">{ov.version || "по умолчанию"}</span>
              </div>
            {/each}
            {#if Object.keys(envReuseTarget.tool_overrides || {}).length === 0}
              <span class="muted text-sm">Используются настройки тулчейнов по умолчанию</span>
            {/if}
          </div>
        </div>

        <div class="env-reused-footer-note">
          <span class="ok-dot">✓</span>
          <span>Повторное скачивание не требуется — инструменты уже установлены в Shared Store.</span>
        </div>
      </div>
    {:else}
      <div class="env-reused-empty-prompt">
        <div class="prompt-icon">🔍</div>
        <div class="prompt-body">
          <h4>Песочница не выбрана</h4>
          <p>Для подключения выберите одно из ранее созданных изолированных окружений.</p>
        </div>
        <button type="button" class="btn-primary" onclick={() => openEnvPicker()}>
          Выбрать песочницу
        </button>
      </div>
    {/if}
  {:else}
  <div class="env-summary">
    <span>{i18n.t("create.ready_count", { x: envCheck.requirements.filter((r) => statusKind(r.status) === "ok" || statusKind(r.status) === "manual").length, y: envCheck.requirements.length }) as TranslationKey}</span>
    {#if missingAll.length > 0}
      <span>{i18n.t("create.to_install", { x: missingSelected.length, y: missingAll.length }) as TranslationKey}</span>
    {/if}
    <span>{i18n.t("create.download_mb", { x: missingSelected.reduce((sum, r) => sum + r.size_mb, 0) }) as TranslationKey}</span>
    <span>{i18n.t("create.free_space", { x: envCheck.free_space_mb }) as TranslationKey}</span>
    {#if !envCheck.enough_space}
      <span class="env-warn">{i18n.t("create.not_enough_disk") as TranslationKey}</span>
    {/if}
    {#if envCheck.needs_admin_any}
      <span class="env-warn">{i18n.t("create.admin_maybe") as TranslationKey}</span>
    {/if}
  </div>

  <div class="env-list">
    {#each envCheck.requirements as req}
      {@const kind = statusKind(req.status)}
      <div
        class="env-row"
        class:ok={kind === "ok"}
        class:update={kind === "update"}
        class:broken={kind === "broken"}
        class:missing={kind === "missing"}
        class:manual={kind === "manual"}
      >
        {#if kind === "ok"}
          <span class="env-select">✅</span>
        {:else if kind === "manual"}
          <span class="env-select manual-badge" title={i18n.t("create.manual_install") as TranslationKey}><TechIcon alt="" size="sm" /></span>
        {:else}
          <label class="env-select">
            <input
              type="checkbox"
              checked={envSelectedIds.has(req.tool_id)}
              onchange={() => ontoggleEnvTool(req.tool_id)}
            />
          </label>
        {/if}
        <span class="env-icon"><TechIcon icon={req.icon ?? toolIcon(req.tool_id)} alt={req.display} size="sm" /></span>
        <span class="env-name">{req.display}</span>
        <span class="env-source">{i18n.t(req.source_description as TranslationKey)}</span>
        {#if envAvailableVersions?.[req.tool_id] && envAvailableVersions[req.tool_id].length > 0}
          {@const candidates = envAvailableVersions[req.tool_id]}
          {@const selected = envSelectedVersions?.[req.tool_id] ?? candidates[0]}
          <div class="env-version-picker" title={i18n.t("create.version_selector_title" as TranslationKey)}>
            <span class="version-label"><Icon name="wrench" size={13} /> {i18n.t("create.version_label" as TranslationKey)}:</span>
            <select
              class="version-select"
              value={selected?.version ?? candidates[0]?.version}
              onchange={(e) => {
                const targetId = (e.currentTarget as HTMLSelectElement).value;
                const match = candidates.find((c) => c.version === targetId);
                if (match && onselectToolVersion) {
                  onselectToolVersion(req.tool_id, match);
                }
              }}
            >
              {#each candidates as cand}
                {@const badge = sourceKindBadge(cand.source_kind)}
                <option value={cand.version}>
                  v{cand.version} — {badge.label}{cand.is_installed ? ' (Ready)' : ' (~' + (cand.size_mb ?? 30) + ' MB)'}
                </option>
              {/each}
            </select>
            {#if selected}
              {@const b = sourceKindBadge(selected.source_kind)}
              <span class="source-tag" class:tag-store={selected.source_kind === "shared_store"} class:tag-manager={selected.source_kind !== "shared_store" && selected.source_kind !== "system"}>
                {b.label}
              </span>
            {/if}
          </div>
        {:else if envLoadingVersions?.[req.tool_id]}
          <div class="env-version-loading">
            <span class="spinner-mini"></span>
            <span>{i18n.t("create.loading_versions" as TranslationKey)}…</span>
          </div>
        {/if}
        <span
          class="env-status"
          class:ok={kind === "ok"}
          class:update={kind === "update"}
          class:broken={kind === "broken"}
          class:missing={kind === "missing"}
          class:manual={kind === "manual"}
        >{statusLabel(req.status)}</span>
      </div>
    {/each}
  </div>

  {#if (envCheck.optional_requirements ?? []).length > 0 || envLocalInfra.size > 0}
    <div class="env-optional" class:env-optional-local={envLocalInfra.size > 0}>
      <p class="group-label">{i18n.t("create.docker_optional") as TranslationKey}</p>
      <p class="hint">
        {i18n.t("create.docker_host_hint") as TranslationKey}
      </p>
      {#snippet infraToggle(toolId: string, onHost: boolean)}
        <span class="infra-toggle" role="group" aria-label={i18n.t("create.docker_host_aria") as TranslationKey}>
          <button
            class="infra-toggle-opt"
            class:active={!onHost}
            title={i18n.t("create.docker_compose_title") as TranslationKey}
            onclick={() => {
              if (onHost) onrevertLocalInfra(toolId);
            }}
          >
            {i18n.t("create.run_docker") as TranslationKey}
          </button>
          <button
            class="infra-toggle-opt"
            class:active={onHost}
            class:host={onHost}
            title={i18n.t("create.host_install_title") as TranslationKey}
            onclick={() => {
              if (!onHost) onoptInLocalInfra(toolId);
            }}
          >
            {i18n.t("create.use_host") as TranslationKey}
          </button>
        </span>
      {/snippet}
      {#each envCheck.optional_requirements ?? [] as req}
        {@const installedHere = isLocallyInstalled(req.tool_id)}
        <div class="env-row" class:ok={installedHere}>
          <span class="env-select"><TechIcon icon="docker.svg" alt={i18n.t("create.docker_badge") as TranslationKey} size="sm" /></span>
          <span class="env-icon"><TechIcon icon={req.icon ?? toolIcon(req.tool_id)} alt={req.display} size="sm" /></span>
          <span class="env-name">{req.display}</span>
          <span class="env-source">
            {installedHere ? (i18n.t("create.running_local") as TranslationKey) : (i18n.t("create.running_docker") as TranslationKey)}
          </span>
          {#if installedHere}
            <span class="env-status ok">{i18n.t("create.installed_host") as TranslationKey}</span>
          {/if}
          {@render infraToggle(req.tool_id, false)}
        </div>
      {/each}
      {#each [...envLocalInfra] as toolId}
        {@const req = envCheck.requirements.find((r) => r.tool_id === toolId)}
        {@const installedHere = isLocallyInstalled(toolId)}
        <div class="env-row ok">
          <span class="env-select"><TechIcon icon="docker.svg" alt={i18n.t("create.docker_badge") as TranslationKey} size="sm" /></span>
          <span class="env-icon"><TechIcon icon={req?.icon ?? toolIcon(toolId)} alt={req?.display ?? toolId} size="sm" /></span>
          <span class="env-name">{req?.display ?? toolId}</span>
          <span class="env-source">
            {installedHere ? (i18n.t("create.running_local") as TranslationKey) : (i18n.t("create.local_pending") as TranslationKey)}
          </span>
          {#if installedHere}
            <span class="env-status ok">{i18n.t("create.installed_host") as TranslationKey}</span>
          {/if}
          {@render infraToggle(toolId, true)}
        </div>
      {/each}
    </div>
  {/if}
  {/if}

  {#if envPlan && envInstalling}
    {@const totalN = envPlan.tasks.length}
    {@const doneN = envPlan.tasks.filter((task) => taskStateKind(envTaskStates.get(task.task_id) ?? task.state) === "success").length}
    {@const failN = envPlan.tasks.filter((task) => taskStateKind(envTaskStates.get(task.task_id) ?? task.state) === "failed").length}
    <div class="env-install">
      <div class="env-install-head">
        <p class="group-label">{i18n.t("create.installing") as TranslationKey}</p>
        <span class="env-chips">
          <span class="chip">{i18n.t("create.install_progress", { done: doneN, total: totalN }) as TranslationKey}</span>
          {#if failN > 0}
            <span class="chip chip-danger">{i18n.t("create.failed_count", { x: failN }) as TranslationKey}</span>
          {/if}
        </span>
      </div>
      {#each envPlan.tasks as task}
        {@const st = envTaskStates.get(task.task_id) ?? task.state}
        {@const kind = taskStateKind(st)}
        {@const running = kind === "running" && typeof st === "object" && "Running" in st}
        {@const downloading = running && st.Running.phase === "Downloading"}
        {@const pct = downloading ? dlPercent(task.task_id) : null}
        {@const dl = envDownload.get(task.task_id)}
        <div class="env-task">
          <div class="env-row">
            <span class="env-icon">
              {#if kind === "running"}⏳
              {:else if kind === "success"}✅
              {:else if kind === "failed"}❌
              {:else if kind === "skipped"}⏭️
              {:else}•{/if}
            </span>
            <TechIcon icon={task.icon ?? toolIcon(task.tool_id)} alt={task.display} size="sm" />
            <span class="env-name">{task.display}</span>
            <span class="env-source">{task.size_mb} MB · {i18n.t(task.source_description as TranslationKey)}</span>
            <span class="env-status">
              {#if running}
                {#if downloading && pct !== null && dl}
                  {i18n.t("create.downloading", { pct, received: formatMb(Math.floor(dl.received / 1024 / 1024)), total: formatMb(Math.floor(dl.total / 1024 / 1024)) }) as TranslationKey}
                  {#if speedLabel(task.task_id)} <span class="env-speed">· {speedLabel(task.task_id)}</span>{/if}
                {:else}
                  <span class="spin" aria-hidden="true"></span> {taskStateLabel(st)} · {phaseElapsed(task.task_id)}
                {/if}
              {:else}
                {taskStateLabel(st)}
              {/if}
            </span>
          </div>
          {#if running}
            {#if downloading && pct !== null}
              <div class="dl-bar" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
                <div class="dl-fill" style="width: {pct}%"></div>
              </div>
            {:else}
              <!-- Фаза без своего прогресса (установка/проверка): индетерминантная
                   полоса показывает, что задача жива, а не зависла. -->
              <div class="dl-bar indeterminate" aria-hidden="true"><div class="dl-fill"></div></div>
            {/if}
          {/if}
        </div>
      {/each}
      {#if envLogs.length > 0}
        <div class="live-log">
          <button class="live-log-toggle" onclick={() => (logOpen = !logOpen)} aria-expanded={logOpen}>
            <span class="chevron" class:open={logOpen}>▸</span>
            {i18n.t("create.live_log", { n: envLogs.length }) as TranslationKey}
          </button>
          {#if logOpen}
            <div class="live-log-body" bind:this={logBox}>
              {#each envLogs as line, i (i)}
                <div
                  class:log-error={line.startsWith("tc:error")}
                  class:log-warn={line.startsWith("tc:warn")}
                  class:log-info={line.startsWith("tc:info")}
                  class:log-ok={line.startsWith("tc:ok")}
                >{line}</div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if envError}
    <p class="error">{envError}</p>
  {/if}

  {#if envInstalling && !envPlan}
    <p class="env-rechecking"><span class="spin" aria-hidden="true"></span> {i18n.t("create.building_plan") as TranslationKey}</p>
  {/if}

  <div class="btn-row">
    <button class="btn-back" onclick={onbackReview} disabled={envInstalling}>{i18n.t("create.back") as TranslationKey}</button>
    {#if envInstalling}
      <button class="btn-secondary" onclick={oncancelInstall}>{i18n.t("create.abort") as TranslationKey}</button>
    {:else if envIsolationMode === "isolated" && envSubMode === "existing"}
      {#if !envReuseTarget}
        <button class="btn-primary" onclick={() => openEnvPicker()}>
          Выбрать песочницу...
        </button>
      {:else}
        <button class="btn-primary" onclick={oncontinue}>
          {i18n.t("create.create_project") as TranslationKey}
        </button>
      {/if}
    {:else if missingAll.length > 0}
      <button class="btn-primary" class:sp-help-anchor={envHintVisible} onclick={onstartInstall} disabled={missingSelected.length === 0}>
        {i18n.t("create.install_selected", { n: missingSelected.length }) as TranslationKey}
      </button>
      {#if missingSelected.length < missingAll.length}
        <button class="btn-secondary" onclick={onselectAll}>{i18n.t("create.select_all", { n: missingAll.length }) as TranslationKey}</button>
      {/if}
      <button class="btn-secondary" onclick={oncontinue}>{i18n.t("create.continue_anyway") as TranslationKey}</button>
    {:else}
      <button class="btn-primary" onclick={oncontinue}>{i18n.t("create.create_project") as TranslationKey}</button>
    {/if}
  </div>

  {#if envRestartHint}
    <p class="env-warn">
      {i18n.t("create.path_restart_hint") as TranslationKey}
    </p>
  {/if}
  {/if}
{:else}
  <p class="error">
    {envError
      ? (i18n.t("create.env_check_failed", { err: envError }) as TranslationKey)
      : (i18n.t("create.env_not_checked") as TranslationKey)}
  </p>
  <div class="btn-row">
    <button class="btn-back" onclick={onback}>{i18n.t("create.back") as TranslationKey}</button>
    <button class="btn-primary" onclick={oncheck}>
      {envError ? (i18n.t("create.retry") as TranslationKey) : (i18n.t("create.check_env") as TranslationKey)}
    </button>
  </div>
{/if}

{#if newSecrets}
  <div class="conflict-overlay" onclick={ondismissSecrets}>
    <div class="conflict-dialog" onclick={(e) => e.stopPropagation()}>
      <h3>{i18n.t("create.generated_passwords") as TranslationKey}</h3>
      <p class="hint">{i18n.t("create.save_passwords_hint") as TranslationKey}</p>
      {#each Object.entries(newSecrets) as [toolId, value]}
        <div class="secret-row">
          <span class="secret-name">{secretToolName(toolId)}</span>
          <code class="secret-value">{value}</code>
          <button class="btn-secondary" onclick={() => oncopySecret(toolId, value)}>
            {secretCopied === toolId ? (i18n.t("create.copied") as TranslationKey) : (i18n.t("create.copy") as TranslationKey)}
          </button>
        </div>
      {/each}
      <div class="btn-row">
        <button class="btn-primary" onclick={ondismissSecrets}>{i18n.t("create.got_it") as TranslationKey}</button>
      </div>
    </div>
  </div>
{/if}

<Modal
  open={showEnvPicker}
  onclose={() => (showEnvPicker = false)}
  title="Выбор существующей изолированной песочницы"
  description="Выберите песочницу, тулчейны которой будут переиспользованы для нового проекта."
  size="lg"
>
  <div class="env-modal-content">
    <div class="env-modal-search">
      <Icon name="search" size={14} />
      <input
        type="search"
        class="env-search-input"
        placeholder="Поиск по названию среды или инструментам..."
        bind:value={envFilterQuery}
      />
    </div>

    {#if envLoadingList}
      <div class="env-modal-loading">
        <span class="spin"></span> Загрузка окружений...
      </div>
    {:else if availableEnvList.length === 0}
      <div class="env-modal-empty">
        <Icon name="box" size={32} />
        <p>Нет доступных сохраненных окружений.</p>
        <span class="muted">Вы можете продолжить с режимом "Новая среда" — песочница создастся автоматически.</span>
      </div>
    {:else}
      {@const filtered = availableEnvList.filter((e) => {
        const q = envFilterQuery.toLowerCase().trim();
        if (!q) return true;
        const name = (e.name || e.binding_id).toLowerCase();
        const tools = Object.keys(e.tool_overrides || {}).join(" ").toLowerCase();
        return name.includes(q) || tools.includes(q);
      })}
      {#if filtered.length === 0}
        <div class="env-modal-empty">
          <p>Ничего не найдено по запросу "{envFilterQuery}".</p>
        </div>
      {:else}
        <div class="env-modal-grid">
          {#each filtered as env (env.binding_id)}
            {@const isSelected = envReuseTarget?.binding_id === env.binding_id}
            <div
              class="env-card-pick"
              class:selected={isSelected}
              onclick={() => selectEnvironment(env)}
              role="button"
              tabindex="0"
              onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") selectEnvironment(env); }}
            >
              <div class="env-card-header">
                <div class="env-card-title-group">
                  <span class="env-card-avatar">⚡</span>
                  <div>
                    <div class="env-card-name">{env.name || env.binding_id}</div>
                    <div class="env-card-sub">
                      ID: {env.binding_id} · {env.isolation_mode === "isolated" ? "Песочница" : "Global"}
                    </div>
                  </div>
                </div>
                {#if isSelected}
                  <span class="selected-badge">Выбрана</span>
                {/if}
              </div>

              <div class="env-card-tools">
                {#each Object.entries(env.tool_overrides || {}) as [toolId, ov]}
                  <span class="tool-tag">
                    <strong>{toolId}</strong> {ov.version || ""}
                  </span>
                {/each}
                {#if Object.keys(env.tool_overrides || {}).length === 0}
                  <span class="tool-tag muted">Тулчейны по умолчанию</span>
                {/if}
              </div>

              <div class="env-card-footer">
                <span class="env-card-projects">
                  📁 {(env.bound_projects || []).length} {projectCountWord((env.bound_projects || []).length)}
                </span>
                <button
                  type="button"
                  class="btn-pick-action"
                  class:active={isSelected}
                  onclick={(e) => {
                    e.stopPropagation();
                    selectEnvironment(env);
                  }}
                >
                  {isSelected ? "Выбрано ✓" : "Выбрать"}
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  {#snippet footer()}
    <div class="env-modal-actions">
      <button type="button" class="btn-secondary" onclick={() => (showEnvPicker = false)}>
        Отмена
      </button>
      {#if envReuseTarget}
        <button type="button" class="btn-primary" onclick={() => (showEnvPicker = false)}>
          Применить выбор
        </button>
      {/if}
    </div>
  {/snippet}
</Modal>

<style>
  .prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
  .hint { color: var(--sp-text-3); margin-bottom: 1.5rem; font-size: 0.95rem; }
  .muted { color: var(--sp-text-3); }
  .error { color: var(--sp-danger); }
  .env-summary { display: flex; flex-wrap: wrap; gap: 0.75rem; align-items: center; padding: 0.75rem 1rem; border: 1px solid var(--sp-border); border-radius: 10px; background: var(--sp-bg-1); margin-bottom: 1rem; font-size: 0.85rem; color: var(--sp-text-2); }
  .env-warn { color: var(--sp-warning); font-weight: 600; }
  .env-list { display: flex; flex-direction: column; gap: 0.4rem; margin-bottom: 1rem; }
  .env-row { display: flex; align-items: center; gap: 0.6rem; padding: 0.5rem 0.75rem; border-radius: 6px; background: var(--sp-bg-1); border-left: 3px solid var(--sp-border-strong); }
  .env-row.ok { border-left-color: var(--sp-success); }
  .env-row.update { border-left-color: var(--sp-warning); }
  .env-row.broken { border-left-color: var(--sp-danger); }
  .env-row.missing { border-left-color: var(--sp-danger); opacity: 0.8; }
  .env-row.manual { border-left-color: var(--sp-warning); }
  .env-optional { margin-bottom: 1rem; padding: 0.75rem 1rem; border: 1px dashed var(--sp-border-strong); border-radius: 10px; background: var(--sp-bg-2); }
  .env-optional .group-label { color: var(--sp-danger); margin: 0 0 0.35rem; }
  .env-optional .env-row { background: var(--sp-bg-1); }
  .env-optional .env-row.ok { border-left-color: var(--sp-success); }
  .env-optional-local { border-color: var(--sp-success-border); background: var(--sp-success-soft); }
  .env-optional-local .group-label { color: var(--sp-success); }
  .infra-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0;
    flex: 0 0 auto;
    border: 1px solid var(--sp-border-strong);
    border-radius: 999px;
    overflow: hidden;
    background: var(--sp-bg-1);
  }
  .infra-toggle-opt {
    border: none;
    background: transparent;
    color: var(--sp-text-3);
    font-size: 0.72rem;
    font-weight: 600;
    padding: 0.3rem 0.75rem;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s, color 0.15s;
  }
  .infra-toggle-opt:hover { color: var(--sp-text-1); background: var(--sp-accent-soft); }
  .infra-toggle-opt.active { background: var(--sp-accent-strong); color: #fff; }
  .infra-toggle-opt.active.host { background: var(--sp-success); }
  .env-select { min-width: 22px; display: flex; align-items: center; justify-content: center; cursor: pointer; }
  .env-select input { accent-color: var(--sp-accent-strong); cursor: pointer; width: 15px; height: 15px; }
  .manual-badge { cursor: help; font-size: 0.95rem; }
  .env-icon { min-width: 20px; font-size: 0.95rem; }
  .env-name { font-weight: 600; font-size: 0.9rem; flex: 0 0 auto; }
  .env-source { font-size: 0.75rem; color: var(--sp-text-3); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .env-version-picker { display: inline-flex; align-items: center; gap: 0.4rem; flex: 0 0 auto; margin-left: auto; }
  .version-label { font-size: 0.72rem; color: var(--sp-text-3); display: inline-flex; align-items: center; gap: 0.2rem; }
  .version-select {
    border: 1px solid var(--sp-border-strong);
    background: var(--sp-bg-2);
    color: var(--sp-text);
    padding: 0.2rem 0.5rem;
    border-radius: 5px;
    font-size: 0.75rem;
    font-family: var(--sp-font-mono, monospace);
    cursor: pointer;
    outline: none;
    transition: border-color 0.15s ease;
  }
  .version-select:focus { border-color: var(--sp-accent-strong); }
  .source-tag {
    font-size: 0.68rem;
    padding: 2px 6px;
    border-radius: 4px;
    font-weight: 500;
    white-space: nowrap;
    border: 1px solid var(--sp-border-strong);
    background: var(--sp-bg-3);
    color: var(--sp-text-2);
  }
  .tag-store { background: rgba(168, 85, 247, 0.15); color: #c084fc; border-color: rgba(168, 85, 247, 0.3); }
  .tag-manager { background: rgba(6, 182, 212, 0.15); color: #38bdf8; border-color: rgba(6, 182, 212, 0.3); }
  .env-version-loading { display: inline-flex; align-items: center; gap: 0.4rem; font-size: 0.72rem; color: var(--sp-text-3); margin-left: auto; }
  .spinner-mini { width: 10px; height: 10px; border: 2px solid var(--sp-text-3); border-top-color: transparent; border-radius: 50%; animation: spin 0.8s linear infinite; }
  .env-status { font-size: 0.8rem; font-weight: 600; flex: 0 0 auto; }
  .env-status.ok { color: var(--sp-success); }
  .env-status.update { color: var(--sp-warning); }
  .env-status.broken { color: var(--sp-danger); }
  .env-status.missing { color: var(--sp-danger); }
  .env-status.manual { color: var(--sp-warning); }
  .env-install { margin-top: 0.5rem; }
  .env-progress-list { display: flex; flex-direction: column; gap: 0.35rem; margin-top: 0.75rem; }
  .env-progress-row { display: flex; align-items: center; gap: 0.6rem; font-size: 0.85rem; }
  .env-progress-row .env-status { margin-left: auto; }
  .env-task { display: flex; flex-direction: column; gap: 0.2rem; }
  .dl-bar { height: 6px; border-radius: 3px; background: var(--sp-bg-3); overflow: hidden; margin-left: 1.9rem; margin-right: 0.4rem; position: relative; }
  .dl-fill { height: 100%; background: var(--sp-accent-strong); border-radius: 3px; transition: width 0.3s ease; }
  .dl-bar.indeterminate .dl-fill { width: 35%; transition: none; animation: tc-indeterminate 1.4s ease-in-out infinite; }
  @keyframes tc-indeterminate {
    0% { margin-left: -35%; }
    100% { margin-left: 100%; }
  }
  .env-speed { color: var(--sp-text-3); font-weight: 400; }
  .env-install-head { display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; margin-bottom: 0.4rem; }
  .env-install-head .group-label { margin: 0; }
  .env-chips { display: flex; gap: 0.4rem; margin-left: auto; }
  .chip { font-size: 0.75rem; font-weight: 600; color: var(--sp-text-2); background: var(--sp-bg-3); border: 1px solid var(--sp-border); padding: 0.25rem 0.65rem; border-radius: 999px; white-space: nowrap; }
  .chip-danger { color: var(--sp-danger); border-color: color-mix(in srgb, var(--sp-danger) 40%, transparent); }
  .live-log { margin-top: 0.75rem; border: 1px solid var(--sp-border); border-radius: 8px; overflow: hidden; background: var(--sp-bg-1); }
  .live-log-toggle {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 0.45rem;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--sp-text-2);
    font-size: 0.8rem;
    font-weight: 600;
    padding: 0.5rem 0.75rem;
    text-align: left;
    font-family: inherit;
  }
  .live-log-toggle:hover { color: var(--sp-text-1); background: var(--sp-bg-2); }
  .chevron { display: inline-block; transition: transform 0.15s; color: var(--sp-text-3); font-size: 0.7rem; }
  .chevron.open { transform: rotate(90deg); }
  .live-log-body {
    max-height: 220px;
    overflow-y: auto;
    padding: 0.5rem 0.75rem;
    font-family: Consolas, monospace;
    font-size: 0.72rem;
    line-height: 1.55;
    background: var(--sp-bg-2);
    border-top: 1px solid var(--sp-border);
  }
  .live-log-body > div { white-space: pre-wrap; word-break: break-all; color: var(--sp-text-2); }
  .live-log-body .log-error { color: var(--sp-danger); }
  .live-log-body .log-warn { color: var(--sp-warning); }
  .live-log-body .log-info { color: var(--sp-text-3); }
  .live-log-body .log-ok { color: var(--sp-success); }
  .env-rechecking { display: flex; align-items: center; gap: 0.5rem; color: var(--sp-text-3); font-size: 0.85rem; margin: 0 0 0.75rem; }
  .spin { display: inline-block; width: 0.8rem; height: 0.8rem; border: 2px solid var(--sp-border-strong); border-top-color: var(--sp-blue); border-radius: 50%; animation: tc-spin 0.8s linear infinite; vertical-align: -2px; margin-right: 0.3rem; }
  @keyframes tc-spin { to { transform: rotate(360deg); } }
  .group-label { font-size: 0.9rem; font-weight: 600; margin-bottom: 0.4rem; color: var(--sp-text-2); text-transform: capitalize; }
  .secret-row { display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.6rem; }
  .secret-name { flex: 0 0 110px; font-size: 0.85rem; color: var(--sp-text-2); font-weight: 600; }
  .secret-value { flex: 1; font-family: Consolas, monospace; font-size: 0.85rem; background: var(--sp-bg-2); border: 1px solid var(--sp-border-strong); border-radius: 6px; padding: 0.4rem 0.6rem; color: var(--sp-accent-strong); overflow-x: auto; white-space: nowrap; user-select: all; }
  .secret-row .btn-secondary { flex: 0 0 auto; }
  .conflict-overlay { position: fixed; inset: 0; background: rgba(2,3,6,0.72); backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .conflict-dialog { background: var(--sp-bg-1); border: 1px solid var(--sp-accent-strong); border-radius: 12px; padding: 1.5rem; max-width: 480px; width: 90%; }
  .conflict-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
  .conflict-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
  .conflict-dialog code { color: var(--sp-accent-strong); }
  .exec-full-log { margin: 1rem 0; }
  .exec-full-log summary { cursor: pointer; color: var(--sp-text-3); font-size: 0.85rem; }
  .exec-full-log pre { font-size: 0.75rem; background: var(--sp-bg-2); padding: 0.5rem; border-radius: 6px; max-height: 200px; overflow-y: auto; white-space: pre-wrap; word-break: break-all; }
  .btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }
  .btn-back { background: none; border: 1px solid var(--sp-border-strong); color: var(--sp-text-3); padding: 0.4rem 0.9rem; border-radius: 6px; cursor: pointer; font-size: 0.85rem; }
  .btn-back:hover { border-color: var(--sp-accent-strong); color: #fff; }
  .btn-primary {
    background: var(--sp-accent-strong);
    color: #fff;
    padding: 0.6rem 1.5rem;
    border-radius: var(--sp-radius-lg);
    border: 1px solid rgba(0, 0, 0, 0.35);
    cursor: pointer;
    font-weight: 600;
    font-size: 0.95rem;
    box-shadow: var(--sp-shadow-1), 0 2px 10px rgba(228, 87, 10, 0.25);
    transition: background 0.15s, box-shadow 0.15s;
  }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.06); }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-secondary { background: var(--sp-accent-soft); color: var(--sp-text-2); padding: 0.6rem 1.5rem; border-radius: 8px; border: 1px solid var(--sp-border-strong); cursor: pointer; font-size: 0.95rem; }

  /* Compact Environment Mode Strip */
  .env-mode-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
    background: var(--sp-bg-2, #161821);
    border: 1px solid var(--sp-border, #262c3c);
    border-radius: 8px;
    padding: 0.45rem 0.75rem;
    margin-bottom: 1rem;
  }
  .env-mode-left {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .env-mode-meta {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--sp-text-3, #94a3b8);
    font-size: 0.8rem;
    font-weight: 500;
  }
  .env-mode-label {
    white-space: nowrap;
  }
  .env-mode-pills {
    display: inline-flex;
    align-items: center;
    background: var(--sp-bg-1, #0f1117);
    border: 1px solid var(--sp-border-strong, #2c3242);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }
  .env-pill-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.6rem;
    border-radius: 4px;
    border: none;
    background: transparent;
    color: var(--sp-text-3, #94a3b8);
    font-size: 0.78rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }
  .env-pill-btn:hover {
    color: var(--sp-text-1, #f1f5f9);
    background: rgba(255, 255, 255, 0.05);
  }
  .env-pill-btn.active {
    background: var(--sp-accent-soft, rgba(245, 158, 11, 0.15));
    color: var(--sp-accent-strong, #f59e0b);
    font-weight: 600;
  }
  .env-pill-icon {
    font-size: 0.8rem;
  }
  .env-pill-tag {
    font-size: 0.65rem;
    padding: 1px 4px;
    border-radius: 3px;
    background: rgba(245, 158, 11, 0.2);
    color: #fbbf24;
    text-transform: uppercase;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .env-mode-hint-inline {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: var(--sp-text-3, #94a3b8);
  }
  .hint-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .hint-dot.ok {
    background: #10b981;
    box-shadow: 0 0 6px rgba(16, 185, 129, 0.5);
  }
  .hint-dot.warn {
    background: #f59e0b;
    box-shadow: 0 0 6px rgba(245, 158, 11, 0.5);
  }
  .hint-text {
    line-height: 1.2;
  }
  /* Submode strip & advice */
  .env-submode-strip {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin-bottom: 1.2rem;
    padding: 0.6rem 0.85rem;
    background: var(--sp-bg-2, #161821);
    border: 1px solid var(--sp-border, #262c3c);
    border-radius: 8px;
  }
  .env-submode-left {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .env-submode-label {
    font-size: 0.78rem;
    color: var(--sp-text-3, #94a3b8);
    font-weight: 500;
  }
  .env-submode-pills {
    display: inline-flex;
    align-items: center;
    background: var(--sp-bg-1, #0f1117);
    border: 1px solid var(--sp-border-strong, #2c3242);
    border-radius: 6px;
    padding: 2px;
    gap: 2px;
  }
  .env-subpill-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.55rem;
    border-radius: 4px;
    border: none;
    background: transparent;
    color: var(--sp-text-3, #94a3b8);
    font-size: 0.76rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .env-subpill-btn:hover {
    color: var(--sp-text-1, #f1f5f9);
    background: rgba(255, 255, 255, 0.05);
  }
  .env-subpill-btn.active {
    background: var(--sp-accent-soft, rgba(245, 158, 11, 0.15));
    color: var(--sp-accent-strong, #f59e0b);
    font-weight: 600;
  }
  .env-target-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.2rem 0.5rem;
    border-radius: 5px;
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(16, 185, 129, 0.3);
    font-size: 0.76rem;
    color: #10b981;
  }
  .env-target-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #10b981;
    box-shadow: 0 0 6px rgba(16, 185, 129, 0.6);
  }
  .env-target-name {
    font-weight: 600;
    color: var(--sp-text-1, #f1f5f9);
  }
  .env-target-count {
    font-size: 0.7rem;
    color: var(--sp-text-3, #94a3b8);
  }
  .btn-change-env {
    border: none;
    background: rgba(255, 255, 255, 0.08);
    color: var(--sp-text-2, #cbd5e1);
    border-radius: 3px;
    padding: 1px 5px;
    font-size: 0.7rem;
    cursor: pointer;
    margin-left: 0.2rem;
    transition: background 0.15s;
  }
  .btn-change-env:hover {
    background: rgba(255, 255, 255, 0.16);
    color: #fff;
  }
  .btn-pick-env {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    border: 1px dashed var(--sp-accent-strong, #f59e0b);
    background: rgba(245, 158, 11, 0.08);
    color: var(--sp-accent-strong, #f59e0b);
    border-radius: 5px;
    padding: 0.2rem 0.55rem;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }
  .btn-pick-env:hover {
    background: rgba(245, 158, 11, 0.18);
  }

  .env-advice-box {
    display: flex;
    align-items: flex-start;
    gap: 0.45rem;
    padding: 0.4rem 0.6rem;
    border-radius: 6px;
    background: rgba(59, 130, 246, 0.08);
    border: 1px solid rgba(59, 130, 246, 0.2);
    font-size: 0.75rem;
    color: var(--sp-text-2, #cbd5e1);
    line-height: 1.35;
  }
  .advice-icon {
    font-size: 0.85rem;
    flex-shrink: 0;
    margin-top: 1px;
  }
  .advice-text code {
    background: rgba(0, 0, 0, 0.25);
    padding: 1px 4px;
    border-radius: 3px;
    font-family: var(--sp-font-mono, monospace);
    color: #93c5fd;
  }

  /* Reused Card */
  .env-reused-card {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
    border: 1px solid var(--sp-border-strong, #2c3242);
    border-radius: 10px;
    background: var(--sp-bg-1, #0f1117);
    margin-bottom: 1.2rem;
  }
  .env-reused-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  .env-reused-title-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .reused-icon {
    font-size: 1.3rem;
  }
  .reused-title {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 700;
    color: var(--sp-text-1, #f1f5f9);
  }
  .reused-subtitle {
    margin: 0.15rem 0 0;
    font-size: 0.76rem;
    color: var(--sp-text-3, #94a3b8);
  }
  .btn-change-reused {
    border: 1px solid var(--sp-border-strong, #2c3242);
    background: var(--sp-bg-2, #161821);
    color: var(--sp-text-2, #cbd5e1);
    border-radius: 6px;
    padding: 0.35rem 0.75rem;
    font-size: 0.78rem;
    cursor: pointer;
    transition: all 0.15s;
  }
  .btn-change-reused:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--sp-text-1, #f1f5f9);
  }
  .env-reused-tools-section {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .reused-section-title {
    font-size: 0.76rem;
    font-weight: 600;
    color: var(--sp-text-3, #94a3b8);
  }
  .reused-tools-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .reused-tool-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.55rem;
    border-radius: 5px;
    background: var(--sp-bg-2, #161821);
    border: 1px solid var(--sp-border, #262c3c);
    font-size: 0.76rem;
  }
  .reused-tool-chip .chip-name {
    font-weight: 600;
    color: var(--sp-text-1, #f1f5f9);
  }
  .reused-tool-chip .chip-ver {
    color: var(--sp-accent-strong, #f59e0b);
    font-family: var(--sp-font-mono, monospace);
  }
  .env-reused-footer-note {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.76rem;
    color: #10b981;
    padding-top: 0.3rem;
    border-top: 1px solid rgba(255, 255, 255, 0.05);
  }
  .ok-dot {
    font-weight: 700;
  }

  /* Empty Prompt for Reused */
  .env-reused-empty-prompt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.2rem;
    border: 1px dashed var(--sp-border-strong, #2c3242);
    border-radius: 10px;
    background: var(--sp-bg-1, #0f1117);
    margin-bottom: 1.2rem;
  }
  .prompt-icon {
    font-size: 1.8rem;
  }
  .prompt-body h4 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--sp-text-1, #f1f5f9);
  }
  .prompt-body p {
    margin: 0.2rem 0 0;
    font-size: 0.78rem;
    color: var(--sp-text-3, #94a3b8);
  }

  /* Modal content */
  .env-modal-content {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    max-height: 520px;
  }
  .env-modal-search {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0.75rem;
    background: var(--sp-bg-1, #0f1117);
    border: 1px solid var(--sp-border-strong, #2c3242);
    border-radius: 8px;
  }
  .env-search-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--sp-text-1, #f1f5f9);
    font-size: 0.85rem;
    outline: none;
  }
  .env-modal-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 2rem;
    color: var(--sp-text-3, #94a3b8);
    font-size: 0.9rem;
  }
  .env-modal-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: 2.5rem 1rem;
    color: var(--sp-text-3, #94a3b8);
    text-align: center;
  }
  .env-modal-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 0.75rem;
    overflow-y: auto;
    max-height: 400px;
    padding-right: 4px;
  }
  .env-card-pick {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.85rem;
    border-radius: 8px;
    background: var(--sp-bg-1, #0f1117);
    border: 1px solid var(--sp-border, #262c3c);
    cursor: pointer;
    transition: all 0.15s ease;
    text-align: left;
  }
  .env-card-pick:hover {
    border-color: var(--sp-accent-strong, #f59e0b);
    background: var(--sp-bg-2, #161821);
    transform: translateY(-1px);
  }
  .env-card-pick.selected {
    border-color: #10b981;
    background: rgba(16, 185, 129, 0.06);
    box-shadow: 0 0 10px rgba(16, 185, 129, 0.15);
  }
  .env-card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .env-card-title-group {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .env-card-avatar {
    font-size: 1.1rem;
  }
  .env-card-name {
    font-weight: 700;
    font-size: 0.9rem;
    color: var(--sp-text-1, #f1f5f9);
    line-height: 1.2;
  }
  .env-card-sub {
    font-size: 0.68rem;
    color: var(--sp-text-3, #94a3b8);
  }
  .selected-badge {
    font-size: 0.65rem;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(16, 185, 129, 0.2);
    color: #10b981;
    font-weight: 600;
  }
  .env-card-tools {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .tool-tag {
    font-size: 0.7rem;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--sp-bg-2, #161821);
    border: 1px solid var(--sp-border-strong, #2c3242);
    color: var(--sp-text-2, #cbd5e1);
  }
  .tool-tag strong {
    color: var(--sp-text-1, #f1f5f9);
  }
  .env-card-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: auto;
    padding-top: 0.4rem;
    border-top: 1px solid rgba(255, 255, 255, 0.05);
  }
  .env-card-projects {
    font-size: 0.72rem;
    color: var(--sp-text-3, #94a3b8);
  }
  .btn-pick-action {
    border: none;
    background: rgba(255, 255, 255, 0.08);
    color: var(--sp-text-2, #cbd5e1);
    border-radius: 4px;
    padding: 0.25rem 0.6rem;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }
  .btn-pick-action:hover {
    background: var(--sp-accent-strong, #f59e0b);
    color: #000;
  }
  .btn-pick-action.active {
    background: #10b981;
    color: #000;
  }
  .env-modal-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
  }
</style>