<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import type { ExecutionPlan, StepStatus, ProjectFileCount } from "$lib/modules/project_creator/types";
  import { countProjectFiles } from "$lib/modules/project_creator/api";
  import { tStepLabel } from "$lib/modules/project_creator/stepI18n";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";

  let {
    execPlan,
    execProjectPath,
    execStatuses,
    execOverallStatus,
    execResult,
    execError,
    execLogs,
    devlProfileExists,
    createdEnvBinding = null,
    oncancel,
    onreset,
    onopenvscode,
    onreopendevl,
    onopenenvterminal,
    onopenworkspace,
  }: {
    execPlan: ExecutionPlan | null;
    execProjectPath: string | null;
    execStatuses: Map<number, { name: string; status: StepStatus; logs: string[] }>;
    execOverallStatus: string;
    execResult: { duration: number; status: string } | null;
    execError: string | null;
    execLogs: string[];
    devlProfileExists: boolean;
    createdEnvBinding?: import("$lib/modules/project_environment/types").EnvironmentBinding | null;
    oncancel: () => void;
    onreset: () => void;
    onopenvscode: () => void;
    onreopendevl: () => void;
    onopenenvterminal?: () => void;
    onopenworkspace?: () => void;
  } = $props();

  let aboutReadme = $derived.by<string | null>(() => {
    const plan = execPlan;
    if (!plan || !Array.isArray(plan.steps)) return null;
    for (const step of plan.steps) {
      if (step && typeof step === "object" && "WriteFile" in step) {
        const w = (step as { WriteFile: { id: string; content: string } }).WriteFile;
        if (w.id === "readme" && w.content) return w.content;
      }
    }
    return null;
  });

  /** Файловые артефакты плана: сколько файлов пишет StackPilot
   *  (WriteFile/RenderTemplate/Generate) и сколько всего артефактов создаёт
   *  план (файлы + каталоги). Считается рекурсивно через Parallel-шаги. */
  let fileStats = $derived.by<{ generated: number; total: number }>(() => {
    const plan = execPlan;
    if (!plan || !Array.isArray(plan.steps)) return { generated: 0, total: 0 };
    const count = (steps: unknown[]): { generated: number; total: number } => {
      let generated = 0;
      let total = 0;
      for (const step of steps) {
        if (!step || typeof step !== "object") continue;
        const s = step as Record<string, unknown>;
        if ("Parallel" in s && Array.isArray((s.Parallel as { steps?: unknown[] }).steps)) {
          const sub = count((s.Parallel as { steps: unknown[] }).steps);
          generated += sub.generated;
          total += sub.total;
          continue;
        }
        if ("WriteFile" in s || "RenderTemplate" in s || "Generate" in s) {
          generated++;
          total++;
        } else if ("CreateDirectory" in s) {
          total++;
        }
      }
      return { generated, total };
    };
    return count(plan.steps);
  });

  /** Реальный подсчёт файлов сгенерированного проекта на диске (включая
   *  node_modules и сторонние артефакты) через быстрый walk на бэкенде.
   *  Запускается при завершении генерации — план считает только файлы
   *  StackPilot, а npm install / скаффолдеры добавляют тысячи своих. */
  let diskFileCount = $state<ProjectFileCount | null>(null);
  let diskCountState = $state<"idle" | "loading" | "done" | "error">("idle");
  const diskPath = $derived(execPlan?.project_path ?? execProjectPath);
  /** Plain flag (not reactive) so the effect below never re-runs and cancels
   *  an in-flight count: it starts the count exactly once per "done" state. */
  let diskCountStarted = false;

  $effect(() => {
    if (execOverallStatus !== "done" || !diskPath || diskCountStarted) return;
    diskCountStarted = true;
    diskCountState = "loading";
    countProjectFiles(diskPath).then(
      (c) => {
        diskFileCount = c;
        diskCountState = "done";
      },
      () => {
        diskCountState = "error";
      },
    );
  });

  /** План-счётчик как фолбэк (если реальный подсчёт ещё идёт / упал). */
  let totalFallback = $derived(fileStats.total);

  /** Форматирование числа файлов: точное значение, либо «круглое N+», если
   *  подсчёт упёрся в лимит на бэкенде (capped). */
  function formatFileCount(c: ProjectFileCount): string {
    if (!c.capped) return c.count.toLocaleString("ru-RU");
    const base = c.count >= 1000 ? Math.floor(c.count / 1000) * 1000 : c.count;
    return `${base.toLocaleString("ru-RU")}+`;
  }
  let totalDisplay = $derived(
    diskFileCount ? formatFileCount(diskFileCount) : String(totalFallback),
  );

  /** Лёгкий рендер markdown-подмножества (заголовки, код, списки, ссылки,
   *  жирный, инлайн-код, hr) — без внешних зависимостей. */
  function renderReadme(md: string): string {
    const esc = (s: string) =>
      s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    const inline = (s: string) =>
      esc(s)
        .replace(/`([^`]+)`/g, "<code class='about-ic'>$1</code>")
        .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
        .replace(
          /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g,
          "<a href='$2' target='_blank' rel='noreferrer'>$1</a>",
        );
    const lines = md.replace(/\r\n/g, "\n").split("\n");
    const out: string[] = [];
    let inCode = false;
    let codeBuf: string[] = [];
    let list: string[] | null = null;
    const flushList = () => {
      if (list) {
        out.push(`<ul class="about-ul">${list.map((l) => `<li>${l}</li>`).join("")}</ul>`);
        list = null;
      }
    };
    for (const raw of lines) {
      if (raw.trim().startsWith("```")) {
        flushList();
        if (inCode) {
          out.push(`<pre class="about-code">${esc(codeBuf.join("\n"))}</pre>`);
          codeBuf = [];
          inCode = false;
        } else {
          inCode = true;
        }
        continue;
      }
      if (inCode) {
        codeBuf.push(raw);
        continue;
      }
      const t = raw.trim();
      if (!t) {
        flushList();
        continue;
      }
      const h = t.match(/^(#{1,4})\s+(.*)$/);
      if (h) {
        flushList();
        const level = h[1].length;
        out.push(`<h${level + 2} class="about-h${level}">${inline(h[2])}</h${level + 2}>`);
        continue;
      }
      const li = t.match(/^[-*]\s+(.*)$/) || t.match(/^\d+[.)]\s+(.*)$/);
      if (li) {
        if (!list) list = [];
        list.push(inline(li[1]));
        continue;
      }
      flushList();
      if (/^-{3,}$/.test(t)) {
        out.push(`<hr class="about-hr" />`);
        continue;
      }
      out.push(`<p class="about-p">${inline(t)}</p>`);
    }
    flushList();
    if (inCode) out.push(`<pre class="about-code">${esc(codeBuf.join("\n"))}</pre>`);
    return out.join("");
  }

  function stepIsRunning(st: StepStatus | undefined) { return st === "Running"; }
  function stepIsSuccess(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Success" in st; }
  function stepIsFailed(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Failed" in st; }
  function stepIsSkipped(st: StepStatus | undefined) { return !!st && typeof st === "object" && "Skipped" in st; }
</script>

<p class="prompt">{i18n.t("create.generating") as TranslationKey}</p>

<div class="exec-steps">
  {#each [...execStatuses.entries()] as [idx, entry]}
    <div class="exec-step"
         class:running={stepIsRunning(entry.status)}
         class:success={stepIsSuccess(entry.status)}
         class:failed={stepIsFailed(entry.status)}
         class:skipped={stepIsSkipped(entry.status)}>
      <div class="exec-icon">
        {#if stepIsRunning(entry.status)}
          ⏳
        {:else if stepIsSuccess(entry.status)}
          ✅
        {:else if stepIsFailed(entry.status)}
          ❌
        {:else if stepIsSkipped(entry.status)}
          ⏭️
        {:else}
          ⏳
        {/if}
      </div>
      <div class="exec-detail">
        <p class="exec-name">{tStepLabel(entry.name)}</p>
        {#if entry.logs.length > 0}
          <pre class="exec-log">{entry.logs.join("\n")}</pre>
        {/if}
      </div>
    </div>
  {/each}
</div>

{#if execLogs.length > 0}
  <details class="exec-full-log">
    <summary>{i18n.t("create.full_log", { n: execLogs.length }) as TranslationKey}</summary>
    <pre>{execLogs.join("\n")}</pre>
  </details>
{/if}

{#if execOverallStatus === "running"}
  <div class="btn-row">
    <button class="btn-secondary" onclick={oncancel}>{i18n.t("create.cancel") as TranslationKey}</button>
  </div>
{:else if execOverallStatus === "done"}
  <div class="exec-finished">
    <p>{i18n.t("create.generated_ms", { x: execResult?.duration ?? 0 }) as TranslationKey}</p>
    <p class="exec-plan-path">{i18n.t("create.location", { path: execPlan?.project_path ?? execProjectPath ?? "" }) as TranslationKey}</p>
  </div>

  {#if createdEnvBinding}
    <div class="sp-env-success-card">
      <div class="sp-env-success-header">
        <div class="sp-env-success-info">
          <div class="sp-env-badge-icon" class:is-isolated={createdEnvBinding.isolation_mode === "isolated"}>
            <Icon name={createdEnvBinding.isolation_mode === "isolated" ? "package" : "globe"} size={22} />
          </div>
          <div>
            <div class="sp-env-title-line">
              <span class="sp-env-card-title">{createdEnvBinding.name || "Окружение проекта"}</span>
              {#if createdEnvBinding.isolation_mode === "isolated"}
                <Badge tone="amber">Изолированное окружение</Badge>
              {:else}
                <Badge tone="cyan">Глобальное системное</Badge>
              {/if}
              <Badge tone="lime">Привязано к проекту ✓</Badge>
            </div>
            <p class="sp-env-card-desc">
              {createdEnvBinding.description || "Инструменты и переменные зафиксированы в профиле StackPilot для этого проекта"}
            </p>
          </div>
        </div>

        <div class="sp-env-header-actions">
          {#if onopenenvterminal}
            <button
              type="button"
              class="sp-env-btn sp-btn-term"
              onclick={onopenenvterminal}
              title="Открыть терминал в изолированном окружении"
            >
              <Icon name="terminal" size={14} />
              <span>Терминал среды</span>
            </button>
          {/if}
          {#if onopenworkspace}
            <button
              type="button"
              class="sp-env-btn sp-btn-ws"
              onclick={onopenworkspace}
              title="Перейти в Workspace проекта"
            >
              <Icon name="home" size={14} />
              <span>Workspace →</span>
            </button>
          {/if}
        </div>
      </div>

      {#if Object.keys(createdEnvBinding.tool_overrides || {}).length > 0}
        <div class="sp-env-tools-strip">
          <span class="sp-env-tools-strip-label">Зафиксированные инструменты:</span>
          <div class="sp-env-tool-tags">
            {#each Object.entries(createdEnvBinding.tool_overrides) as [toolId, tool]}
              <span class="sp-env-tool-chip">
                <span class="sp-tool-name">{toolId}</span>
                {#if tool.version}
                  <span class="sp-tool-ver">v{tool.version}</span>
                {/if}
              </span>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  {/if}

  <div class="about-project-section">
    <h4 class="about-title">{i18n.t("create.preview.about_title") as TranslationKey}</h4>
    {#if aboutReadme}
      <div class="about-markdown">{@html renderReadme(aboutReadme)}</div>
    {:else}
      <p class="about-hint">{i18n.t("create.preview.about_readme") as TranslationKey}</p>
    {/if}
    <div class="about-stats">
      <span class="about-stat">
        <span class="about-stat-num">{execPlan?.steps?.length ?? 0}</span>
        <span class="about-stat-label">{i18n.t("create.preview.tab_steps") as TranslationKey}</span>
      </span>
      <span class="about-stat">
        <span class="about-stat-num">{diskCountState === "loading" ? "…" : (fileStats.generated.toLocaleString("ru-RU"))}</span>
        <span class="about-stat-label">{i18n.t("create.preview.files_by_stackpilot") as TranslationKey}</span>
      </span>
      <span class="about-stat">
        <span class="about-stat-num">{diskCountState === "loading" ? "…" : totalDisplay}</span>
        <span class="about-stat-label">{i18n.t("create.preview.files_total") as TranslationKey}</span>
      </span>
    </div>
  </div>

  <div class="btn-row">
    <button class="btn-secondary" onclick={onopenvscode}>{i18n.t("create.open_vscode") as TranslationKey}</button>
    <button class="btn-primary" onclick={onreset}>{i18n.t("create.create_another") as TranslationKey}</button>
    {#if execPlan}
      <button
        class="devl-chip"
        onclick={onreopendevl}
        title={devlProfileExists
          ? (i18n.t("create.devl_chip_open") as TranslationKey)
          : (i18n.t("create.devl_chip_add") as TranslationKey)}
      >
        <span class="devl-chip-icon" aria-hidden="true">{devlProfileExists ? "▸" : "+"}</span>
        <span class="devl-chip-text">
          <span class="devl-chip-title">DevLauncher</span>
          <span class="devl-chip-sub">
            {devlProfileExists
              ? (i18n.t("create.devl_btn_run") as TranslationKey)
              : (i18n.t("create.devl_btn_add") as TranslationKey)}
          </span>
        </span>
      </button>
    {/if}
  </div>
{:else if execOverallStatus === "error" || execOverallStatus === "cancelled"}
  <div class="exec-finished error">
    <p>{execOverallStatus === "cancelled" ? (i18n.t("create.cancelled") as TranslationKey) : (i18n.t("create.exec_error", { err: execError ?? "" }) as TranslationKey)}</p>
  </div>
  <div class="btn-row">
    <button class="btn-primary" onclick={onreset}>{i18n.t("create.start_over") as TranslationKey}</button>
  </div>
{/if}

<style>
  .prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
  .exec-steps { display: flex; flex-direction: column; gap: 0.5rem; margin: 1rem 0; }
  .exec-step { display: flex; align-items: flex-start; gap: 0.6rem; padding: 0.5rem; border-radius: 6px; background: var(--sp-bg-1); }
  .exec-step.running { border-left: 3px solid var(--sp-accent-strong); }
  .exec-step.success { border-left: 3px solid var(--sp-success); }
  .exec-step.failed { border-left: 3px solid var(--sp-danger); }
  .exec-step.skipped { border-left: 3px solid var(--sp-text-3); opacity: 0.6; }
  .exec-icon { font-size: 1.1rem; min-width: 24px; }
  .exec-detail { flex: 1; min-width: 0; }
  .exec-name { font-weight: 600; margin: 0; font-size: 0.9rem; }
  .exec-log { font-size: 0.75rem; color: var(--sp-text-3); background: var(--sp-bg-2); padding: 0.3rem; border-radius: 4px; max-height: 80px; overflow-y: auto; margin: 0.3rem 0 0; white-space: pre-wrap; word-break: break-all; }
  .exec-full-log { margin: 1rem 0; }
  .exec-full-log summary { cursor: pointer; color: var(--sp-text-3); font-size: 0.85rem; }
  .exec-full-log pre { font-size: 0.75rem; background: var(--sp-bg-2); padding: 0.5rem; border-radius: 6px; max-height: 200px; overflow-y: auto; white-space: pre-wrap; word-break: break-all; }
  .exec-finished { margin: 1rem 0; }
  .exec-finished p { margin: 0.3rem 0; }
  .exec-finished.error { color: var(--sp-danger); }
  .exec-plan-path { font-size: 0.85rem; color: var(--sp-text-3); }
  /* Компактный акцентный чип DevLauncher на финальной странице — заметный,
     но не доминирующий: мягкая подсветка, круглая иконка, авто-ширина. */
  .devl-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.95rem 0.5rem 0.6rem;
    border-radius: var(--sp-radius-full);
    border: 1px solid var(--sp-accent-border);
    background: var(--sp-accent-soft);
    color: var(--sp-text-1);
    cursor: pointer;
    box-shadow: var(--sp-shadow-1);
    transition: background 0.15s, box-shadow 0.15s, border-color 0.15s, transform 0.15s;
  }
  .devl-chip:hover {
    border-color: var(--sp-accent-strong);
    background: color-mix(in srgb, var(--sp-accent-soft) 72%, var(--sp-accent) 7%);
    box-shadow: var(--sp-shadow-accent);
    transform: translateY(-1px);
  }
  .devl-chip:active { transform: translateY(0) scale(0.99); }
  .devl-chip:focus-visible { outline: none; box-shadow: var(--sp-focus-ring); }
  .devl-chip-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.45rem;
    height: 1.45rem;
    border-radius: 50%;
    flex: none;
    background: linear-gradient(180deg, var(--sp-accent-strong), var(--sp-accent));
    color: #fff;
    font-size: 0.8rem;
    font-weight: 700;
    line-height: 1;
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.18), 0 1px 3px rgba(0, 0, 0, 0.35);
  }
  .devl-chip-text { display: flex; flex-direction: column; align-items: flex-start; line-height: 1.15; gap: 0.12rem; }
  .devl-chip-title { font-size: 0.85rem; font-weight: 700; }
  .devl-chip-sub { font-size: 0.7rem; font-weight: 500; color: var(--sp-accent-strong); opacity: 0.9; }
  .about-project-section { border: 1px solid var(--sp-border-strong); border-radius: 10px; padding: 1rem; margin: 1rem 0; background: var(--sp-bg-1); }
  .about-title { margin: 0 0 0.5rem; font-size: 0.95rem; font-weight: 600; color: var(--sp-text-1); }
  .about-hint { margin: 0 0 0.75rem; font-size: 0.8rem; color: var(--sp-text-3); }
  .about-stats { display: flex; gap: 1.5rem; }
  .about-stat { display: flex; flex-direction: column; align-items: center; gap: 0.2rem; }
  .about-stat-num { font-size: 1.5rem; font-weight: 700; color: var(--sp-accent-strong); }
  .about-stat-label { font-size: 0.7rem; color: var(--sp-text-3); text-transform: uppercase; letter-spacing: 0.04em; }
  .about-markdown {
    max-height: 480px;
    overflow-y: auto;
    padding: 0.75rem 1rem;
    margin: 0 0 0.75rem;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: 8px;
    font-size: 0.85rem;
    line-height: 1.55;
    color: var(--sp-text-1);
  }
  .about-markdown :global(.about-h2) { margin: 0.8rem 0 0.4rem; font-size: 1.05rem; font-weight: 700; color: var(--sp-accent-strong); }
  .about-markdown :global(.about-h3) { margin: 0.6rem 0 0.3rem; font-size: 0.95rem; font-weight: 600; color: var(--sp-text-1); }
  .about-markdown :global(.about-h4) { margin: 0.5rem 0 0.25rem; font-size: 0.88rem; font-weight: 600; color: var(--sp-text-1); }
  .about-markdown :global(.about-h5) { margin: 0.5rem 0 0.25rem; font-size: 0.85rem; font-weight: 600; color: var(--sp-text-2); }
  .about-markdown :global(.about-h2:first-child) { margin-top: 0; }
  .about-markdown :global(.about-p) { margin: 0.35rem 0; }
  .about-markdown :global(.about-ul) { margin: 0.35rem 0; padding-left: 1.25rem; }
  .about-markdown :global(.about-ul li) { margin: 0.15rem 0; }
  .about-markdown :global(.about-ic) {
    font-family: var(--sp-font-mono);
    font-size: 0.78em;
    padding: 0.1em 0.35em;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: 4px;
    color: var(--sp-accent-strong);
  }
  .about-markdown :global(.about-code) {
    margin: 0.5rem 0;
    padding: 0.5rem 0.75rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: 6px;
    font-family: var(--sp-font-mono);
    font-size: 0.78rem;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--sp-text-2);
  }
  .about-markdown :global(.about-hr) { border: none; border-top: 1px solid var(--sp-border); margin: 0.75rem 0; }
  .about-markdown :global(a) { color: var(--sp-accent-strong); }
  .btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }
  .btn-secondary { background: var(--sp-accent-soft); color: var(--sp-text-2); padding: 0.6rem 1.5rem; border-radius: 8px; border: 1px solid var(--sp-border-strong); cursor: pointer; font-size: 0.95rem; }
  .btn-primary {
    background: var(--sp-accent-strong);
    color: #fff;
    padding: 0.65rem 1.5rem;
    border-radius: var(--sp-radius-lg);
    border: 1px solid var(--sp-accent-border);
    cursor: pointer;
    font-weight: 600;
    font-size: 0.95rem;
    box-shadow: var(--sp-shadow-1), 0 2px 10px rgba(228, 87, 10, 0.25);
    transition: background 0.15s ease, transform 0.15s ease, box-shadow 0.15s ease;
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--sp-accent);
    box-shadow: var(--sp-shadow-accent), 0 4px 14px rgba(228, 87, 10, 0.35);
    transform: translateY(-1px);
  }
  .btn-primary:active:not(:disabled) {
    transform: translateY(0);
  }

  /* Environment Success Card */
  .sp-env-success-card {
    margin: 1.25rem 0;
    padding: 1.15rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg, 10px);
    box-shadow: var(--sp-shadow-1);
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }
  .sp-env-success-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }
  .sp-env-success-info {
    display: flex;
    align-items: center;
    gap: 0.85rem;
    min-width: 0;
  }
  .sp-env-badge-icon {
    width: 40px;
    height: 40px;
    border-radius: var(--sp-radius-md, 8px);
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    flex-shrink: 0;
  }
  .sp-env-badge-icon.is-isolated {
    background: rgba(245, 158, 11, 0.12);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.25);
  }
  .sp-env-title-line {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .sp-env-card-title {
    font-size: 1.05rem;
    font-weight: 700;
    color: var(--sp-text-1);
  }
  .sp-env-card-desc {
    font-size: 0.82rem;
    color: var(--sp-text-3);
    margin: 0.2rem 0 0;
  }
  .sp-env-header-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .sp-env-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.45rem 0.85rem;
    font-size: 0.82rem;
    font-weight: 600;
    border-radius: var(--sp-radius-md, 6px);
    cursor: pointer;
    transition: all 0.15s ease;
    border: 1px solid var(--sp-border);
  }
  .sp-btn-term {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }
  .sp-btn-term:hover {
    background: var(--sp-bg-3, rgba(255, 255, 255, 0.08));
    border-color: var(--sp-accent-strong);
  }
  .sp-btn-ws {
    background: var(--sp-accent-strong);
    color: #fff;
    border-color: var(--sp-accent-border);
  }
  .sp-btn-ws:hover {
    background: var(--sp-accent);
    transform: translateY(-1px);
  }
  .sp-env-tools-strip {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid var(--sp-border);
    flex-wrap: wrap;
  }
  .sp-env-tools-strip-label {
    font-size: 0.78rem;
    color: var(--sp-text-3);
    font-weight: 600;
  }
  .sp-env-tool-tags {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
  }
  .sp-env-tool-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.2rem 0.55rem;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full, 999px);
    font-size: 0.76rem;
  }
  .sp-tool-name {
    font-weight: 600;
    color: var(--sp-text-1);
  }
  .sp-tool-ver {
    font-family: var(--sp-font-mono);
    color: var(--sp-text-3);
    font-size: 0.72rem;
  }
</style>