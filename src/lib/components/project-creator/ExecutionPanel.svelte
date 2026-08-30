<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import type { ExecutionPlan, StepStatus, ProjectFileCount } from "$lib/modules/project_creator/types";
  import { countProjectFiles } from "$lib/modules/project_creator/api";

  let {
    execPlan,
    execProjectPath,
    execStatuses,
    execOverallStatus,
    execResult,
    execError,
    execLogs,
    devlProfileExists,
    oncancel,
    onreset,
    onopenvscode,
    onreopendevl,
  }: {
    execPlan: ExecutionPlan | null;
    execProjectPath: string | null;
    execStatuses: Map<number, { name: string; status: StepStatus; logs: string[] }>;
    execOverallStatus: string;
    execResult: { duration: number; status: string } | null;
    execError: string | null;
    execLogs: string[];
    devlProfileExists: boolean;
    oncancel: () => void;
    onreset: () => void;
    onopenvscode: () => void;
    onreopendevl: () => void;
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

  $effect(() => {
    if (execOverallStatus !== "done" || !diskPath || diskCountState !== "idle") return;
    diskCountState = "loading";
    let cancelled = false;
    countProjectFiles(diskPath).then(
      (c) => {
        if (cancelled) return;
        diskFileCount = c;
        diskCountState = "done";
      },
      () => {
        if (!cancelled) diskCountState = "error";
      },
    );
    return () => {
      cancelled = true;
    };
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
        <p class="exec-name">{entry.name}</p>
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
  </div>

  {#if execPlan}
    <button
      class="devl-chip"
      class:missing={!devlProfileExists}
      onclick={onreopendevl}
      title={devlProfileExists
        ? (i18n.t("create.devl_chip_open") as TranslationKey)
        : (i18n.t("create.devl_chip_add") as TranslationKey)}
    >
      <span class="devl-chip-icon">{devlProfileExists ? "→" : "+"}</span>
      <span class="devl-chip-label">DevLauncher</span>
    </button>
  {/if}
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
  /* Маленькая кнопка DevLauncher на финальной странице */
  .devl-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin-top: 1rem;
    padding: 0.35rem 0.8rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: 999px;
    background: var(--sp-bg-1);
    color: var(--sp-text-2);
    font-size: 0.8rem;
    cursor: pointer;
    transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .devl-chip:hover { border-color: var(--sp-accent-strong); color: #fff; }
  .devl-chip.missing { border-style: dashed; }
  .devl-chip-icon { font-weight: 700; line-height: 1; }
  .devl-chip-label { font-weight: 600; }
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
  .btn-primary { background: var(--sp-accent-strong); color: #fff; padding: 0.6rem 1.5rem; border-radius: 8px; border: none; cursor: pointer; font-weight: 600; font-size: 0.95rem; }
</style>