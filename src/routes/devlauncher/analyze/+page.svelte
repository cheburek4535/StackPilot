<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { analyzeProjectV2, saveProfileV2 } from "$lib/modules/devlauncher/api";
  import { goto } from "$app/navigation";
  import {
    applyStepPatch,
    failurePolicyLabel,
    stepKindLabel,
    stepKindSummary,
    visibilityLabel,
    type DraftProfile,
    type FailurePolicy,
    type LaunchStep,
    type Visibility,
  } from "$lib/modules/devlauncher/types";
  import Icon from "$lib/components/ui/Icon.svelte";

  let projectPath = $state("");
  let draft = $state<DraftProfile | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state("");
  let savedOk = $state(false);

  let expanded = $state(new Set<string>());
  let showAddPanel = $state(false);
  let dragIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  let addTpl = $state({
    command: "",
    workdir: "",
    path: "",
    url: "",
    port: "8080",
    host: "127.0.0.1",
    timeout: "90",
    seconds: "5",
  });

  const failurePolicies: Array<{ key: FailurePolicy; label: string }> = [
    { key: "stop_run", label: failurePolicyLabel("stop_run") },
    { key: "skip_dependents", label: failurePolicyLabel("skip_dependents") },
    { key: "warn_and_continue", label: failurePolicyLabel("warn_and_continue") },
  ];

  const visibilities: Array<{ key: Visibility; label: string }> = [
    { key: "captured", label: visibilityLabel("captured") },
    { key: "visible_terminal", label: visibilityLabel("visible_terminal") },
    { key: "detached", label: visibilityLabel("detached") },
  ];

  type AddTemplate =
    | { kind: "terminal_plain" }
    | { kind: "terminal_cmd" }
    | { kind: "run_command" }
    | { kind: "open_folder" }
    | { kind: "open_url" }
    | { kind: "wait_port" }
    | { kind: "delay" };

  /** Build a new step from a micro-template. Working dir defaults to the
   * analyzed project root so terminals/commands open in the right place. */
  function buildStep(tpl: AddTemplate): LaunchStep {
    const id = crypto.randomUUID();
    const wd = projectPath || undefined;
    const workdir = addTpl.workdir.trim() || wd;
    switch (tpl.kind) {
      case "terminal_plain":
        return {
          id,
          label: "Открыть терминал",
          enabled: true,
          kind: { type: "open_terminal", command: "" },
          depends_on: [],
          working_directory: wd,
          visibility: "visible_terminal",
          execution_mode: "long_running",
          completion: { type: "process_started" },
        };
      case "terminal_cmd": {
        const command = addTpl.command.trim();
        return {
          id,
          label: command ? `Терминал: ${command}` : "Открыть терминал",
          enabled: true,
          kind: { type: "open_terminal", command },
          depends_on: [],
          working_directory: workdir,
          visibility: "visible_terminal",
          execution_mode: "long_running",
          completion: { type: "process_started" },
        };
      }
      case "run_command": {
        const command = addTpl.command.trim();
        return {
          id,
          label: `Выполнить: ${command}`,
          enabled: true,
          kind: { type: "run_command", command },
          depends_on: [],
          working_directory: workdir,
          visibility: "visible_terminal",
          execution_mode: "long_running",
          completion: { type: "process_started" },
        };
      }
      case "open_folder": {
        const path = addTpl.path.trim() || wd || "";
        return {
          id,
          label: "Открыть папку",
          enabled: true,
          kind: { type: "open_folder", path },
          depends_on: [],
          working_directory: path || undefined,
          completion: { type: "external_launch_accepted" },
        };
      }
      case "open_url": {
        const url = addTpl.url.trim();
        return {
          id,
          label: `Открыть URL: ${url}`,
          enabled: true,
          kind: { type: "open_url", url },
          depends_on: [],
          completion: { type: "external_launch_accepted" },
        };
      }
      case "wait_port": {
        const port = parseInt(addTpl.port, 10) || 0;
        const host = addTpl.host.trim() || "127.0.0.1";
        const timeout = parseInt(addTpl.timeout, 10) || 90;
        return {
          id,
          label: `Ожидание порта ${host}:${port}`,
          enabled: true,
          kind: { type: "wait_for_port", host, port },
          depends_on: [],
          timeout,
          completion: { type: "port_open", host, port, timeout_secs: timeout },
          retry_policy: { max_retries: 2, delay_ms: 2000, backoff_multiplier: 1.5 },
        };
      }
      case "delay": {
        const seconds = parseInt(addTpl.seconds, 10) || 5;
        return {
          id,
          label: `Пауза ${seconds} сек`,
          enabled: true,
          kind: { type: "delay", seconds },
          depends_on: [],
          timeout: seconds + 10,
          completion: { type: "delay_elapsed", seconds },
        };
      }
    }
  }

  function addStep(tpl: AddTemplate) {
    if (!draft) return;
    const steps = [...draft.profile.steps, buildStep(tpl)];
    draft = { ...draft, profile: { ...draft.profile, steps } };
    addTpl = { ...addTpl, command: "", workdir: "", path: "", url: "" };
  }

  function removeStep(index: number) {
    if (!draft) return;
    const steps = draft.profile.steps.filter((_, i) => i !== index);
    draft = { ...draft, profile: { ...draft.profile, steps } };
  }

  function onDropStep(index: number) {
    if (dragIndex === null || dragIndex === index || !draft) return;
    const steps = [...draft.profile.steps];
    const [moved] = steps.splice(dragIndex, 1);
    steps.splice(index, 0, moved);
    draft = { ...draft, profile: { ...draft.profile, steps } };
    dragIndex = null;
    dragOverIndex = null;
  }

  async function pickFolder() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Выберите папку проекта",
      });
      if (selected) {
        projectPath = selected;
        draft = null;
        error = "";
        savedOk = false;
      }
    } catch (e) {
      error = `Ошибка выбора папки: ${e}`;
    }
  }

  async function handleAnalyze() {
    if (!projectPath) return;
    loading = true;
    error = "";
    savedOk = false;
    draft = null;
    expanded = new Set();
    try {
      draft = await analyzeProjectV2(projectPath);
    } catch (e) {
      error = `Ошибка анализа: ${e}`;
    }
    loading = false;
  }

  async function handleSave() {
    if (!draft) return;
    saving = true;
    error = "";
    savedOk = false;
    try {
      await saveProfileV2(draft.profile);
      savedOk = true;
    } catch (e) {
      error = `Ошибка сохранения: ${e}`;
    }
    saving = false;
  }

  function updateStep(index: number, patch: Partial<LaunchStep>) {
    if (!draft) return;
    const steps = draft.profile.steps.map((s, i) =>
      i === index ? applyStepPatch(s, patch) : s,
    );
    draft = { ...draft, profile: { ...draft.profile, steps } };
  }

  function toggleEnabled(index: number) {
    if (!draft) return;
    updateStep(index, { enabled: !draft.profile.steps[index].enabled });
  }

  function updateTimeout(index: number, raw: string) {
    const value = raw.trim();
    if (value === "") {
      updateStep(index, { timeout: null });
      return;
    }
    const num = parseInt(value, 10);
    if (!Number.isNaN(num) && num > 0) {
      updateStep(index, { timeout: num });
    }
  }

  function toggleExpand(id: string) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function confidenceLabel(confidence: string): string {
    switch (confidence) {
      case "high": return "высокая";
      case "medium": return "средняя";
      default: return "низкая";
    }
  }
</script>

<main>
  <h1>{i18n.t("analyze.title" as TranslationKey)}</h1>
  <p class="subtitle">{i18n.t("analyze.subtitle" as TranslationKey)}</p>

  <div class="picker-card">
    <div class="picker-row">
      <button class="primary" onclick={pickFolder}>{i18n.t("analyze.select_folder" as TranslationKey)}</button>
      {#if projectPath}
        <span class="path-display">{projectPath}</span>
        <button class="secondary" onclick={handleAnalyze} disabled={loading}>
          {loading ? i18n.t("analyze.analyzing" as TranslationKey) : i18n.t("analyze.analyze_btn" as TranslationKey)}
        </button>
      {/if}
    </div>
    {#if !projectPath}
      <p class="hint">{i18n.t("analyze.folder_hint" as TranslationKey)}</p>
    {/if}

    {#if error}
      <div class="msg error">{error}</div>
    {/if}

    {#if savedOk}
      <div class="msg success">
        {i18n.t("analyze.saved" as TranslationKey)}
        <button class="link" onclick={() => goto("/workspace")}>{i18n.t("analyze.go_to_profiles" as TranslationKey)}</button>
      </div>
    {/if}
  </div>

  {#if draft}
    <section class="editor">
      <div class="editor-header">
        <div>
          <h2>{draft.profile.name}</h2>
          <p class="desc">{draft.profile.description}</p>
        </div>
        <button class="primary save-btn" onclick={handleSave} disabled={saving}>
          {saving ? i18n.t("analyze.saving" as TranslationKey) : i18n.t("analyze.save_profile" as TranslationKey)}
        </button>
      </div>

      {#if draft.diagnostics.length > 0}
        <div class="diagnostics">
          <h3>{i18n.t("analyze.diagnostics" as TranslationKey)}</h3>
          {#each draft.diagnostics as d, i (i)}
            <div class="diag-row" class:diag-warning={d.severity === "warning"} class:diag-error={d.severity === "error"}>
              <span class="diag-dot" class:diag-dot-warning={d.severity === "warning"} class:diag-dot-error={d.severity === "error"}></span>
              <span class="diag-text">
                {d.message}
                {#if d.file}
                  <span class="diag-file">({d.file})</span>
                {/if}
              </span>
              <span class="diag-conf">{i18n.t("analyze.confidence") as TranslationKey}: {confidenceLabel(d.confidence)}</span>
            </div>
          {/each}
        </div>
      {/if}

      <div class="add-bar">
        <button class="secondary add-toggle" onclick={() => (showAddPanel = !showAddPanel)}>
          <Icon name={showAddPanel ? "x" : "plus"} size={14} />
          {showAddPanel ? i18n.t("analyze.hide_templates") : i18n.t("analyze.add_action")}
        </button>
        <span class="add-hint">{i18n.t("analyze.drag_hint")}</span>
      </div>

      {#if showAddPanel}
        <div class="template-panel">
          <div class="tpl-row">
            <div class="tpl-body">
              <div class="tpl-name">Пустой терминал</div>
              <div class="tpl-desc">Открыть терминал в корне проекта без команды</div>
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "terminal_plain" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">Терминал с командой</div>
              <input type="text" placeholder="Команда (например: docker logs -f backend)" bind:value={addTpl.command} />
              <input type="text" placeholder={`Рабочая папка (по умолчанию: ${projectPath || "корень проекта"})`} bind:value={addTpl.workdir} />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "terminal_cmd" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">Выполнить команду</div>
              <input type="text" placeholder="Команда (например: npm test)" bind:value={addTpl.command} />
              <input type="text" placeholder={`Рабочая папка (по умолчанию: ${projectPath || "корень проекта"})`} bind:value={addTpl.workdir} />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "run_command" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">Открыть папку</div>
              <input type="text" placeholder={`Путь (по умолчанию: ${projectPath || "корень проекта"})`} bind:value={addTpl.path} />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "open_folder" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">Открыть URL</div>
              <input type="text" placeholder="https://localhost:3000/docs" bind:value={addTpl.url} />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "open_url" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields tpl-inline">
              <div class="tpl-name">Ожидать порт</div>
              <input type="text" placeholder="Хост" bind:value={addTpl.host} class="tpl-sm" />
              <input type="number" placeholder="Порт" bind:value={addTpl.port} class="tpl-sm" />
              <input type="number" placeholder="Таймаут, сек" bind:value={addTpl.timeout} class="tpl-sm" />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "wait_port" })}>Добавить</button>
          </div>

          <div class="tpl-row">
            <div class="tpl-body tpl-fields tpl-inline">
              <div class="tpl-name">Пауза</div>
              <input type="number" placeholder="Секунды" bind:value={addTpl.seconds} class="tpl-sm" />
            </div>
            <button class="secondary" onclick={() => addStep({ kind: "delay" })}>Добавить</button>
          </div>
        </div>
      {/if}

      <div class="action-list" role="list">
        {#each draft.profile.steps as step, i (step.id)}
          <div
            class="action-card"
            class:expanded={expanded.has(step.id)}
            class:disabled={!step.enabled}
            class:drag-over={dragOverIndex === i}
            draggable="true"
            ondragstart={(e) => {
              dragIndex = i;
              if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
            }}
            ondragover={(e) => {
              e.preventDefault();
              if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
              dragOverIndex = i;
            }}
            ondragleave={() => {
              if (dragOverIndex === i) dragOverIndex = null;
            }}
            ondrop={(e) => {
              e.preventDefault();
              onDropStep(i);
            }}
            ondragend={() => {
              dragIndex = null;
              dragOverIndex = null;
            }}
          >
            <div class="card-header">
              <div
                class="card-info"
                onclick={() => toggleExpand(step.id)}
                role="button"
                tabindex="0"
                onkeydown={(e) => e.key === "Enter" && toggleExpand(step.id)}
              >
                <span class="card-label">{step.label || i18n.t("analyze.untitled" as TranslationKey)}</span>
                <span class="card-type">{stepKindLabel(step.kind)}</span>
                <span class="card-summary">{stepKindSummary(step.kind)}</span>
              </div>
              <div class="card-controls">
                <button
                  class="toggle-btn"
                  class:on={step.enabled}
                  onclick={() => toggleEnabled(i)}
                  title={step.enabled ? i18n.t("analyze.disable" as TranslationKey) : i18n.t("analyze.enable" as TranslationKey)}
                >
                  {step.enabled ? i18n.t("analyze.on" as TranslationKey) : i18n.t("analyze.off" as TranslationKey)}
                </button>
                <button
                  class="icon-btn expand-btn"
                  class:expanded={expanded.has(step.id)}
                  onclick={() => toggleExpand(step.id)}
                  title={i18n.t("analyze.expand" as TranslationKey)}
                >
                  <Icon name="chevronRight" size={14} />
                </button>
                <button class="icon-btn delete-btn" onclick={() => removeStep(i)} title="Удалить действие">
                  <Icon name="trash" size={14} />
                </button>
              </div>
            </div>

            {#if expanded.has(step.id)}
              <div class="card-editor">
                <div class="field">
                  <label>{i18n.t("analyze.label" as TranslationKey)}</label>
                  <input
                    type="text"
                    value={step.label}
                    oninput={(e) => updateStep(i, { label: (e.target as HTMLInputElement).value })}
                    placeholder={i18n.t("analyze.label_placeholder" as TranslationKey)}
                  />
                </div>
                <div class="field">
                  <label>Таймаут (сек) — 0 / пусто = по умолчанию</label>
                  <input
                    type="number"
                    value={step.timeout ?? ""}
                    oninput={(e) => updateTimeout(i, (e.target as HTMLInputElement).value)}
                    placeholder="120"
                  />
                </div>
                <div class="field">
                  <label>Политика при ошибке</label>
                  <select
                    value={step.failure_policy ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLSelectElement).value;
                      updateStep(i, { failure_policy: (v as FailurePolicy) || null });
                    }}
                  >
                    <option value="">{failurePolicyLabel(null)}</option>
                    {#each failurePolicies as fp}
                      <option value={fp.key}>{fp.label}</option>
                    {/each}
                  </select>
                </div>
                <div class="field">
                  <label>Видимость</label>
                  <select
                    value={step.visibility ?? ""}
                    onchange={(e) => {
                      const v = (e.target as HTMLSelectElement).value;
                      updateStep(i, { visibility: (v as Visibility) || null });
                    }}
                  >
                    <option value="">{visibilityLabel(null)}</option>
                    {#each visibilities as vis}
                      <option value={vis.key}>{vis.label}</option>
                    {/each}
                  </select>
                </div>
                {#if step.depends_on.length > 0}
                  <div class="field">
                    <label>Запускается после</label>
                    <div class="chips">
                      {#each step.depends_on as dep}
                        <span class="chip">{dep}</span>
                      {/each}
                    </div>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>

      <div class="footer-save">
        <button class="primary" onclick={handleSave} disabled={saving}>
          {saving ? i18n.t("analyze.saving" as TranslationKey) : i18n.t("analyze.save_profile" as TranslationKey)}
        </button>
      </div>
    </section>
  {/if}
</main>

<style>
  main {
    max-width: 780px;
    margin: 0 auto;
    padding: 2rem;
    color: var(--sp-text-1);
  }

  h1 { margin: 0; font-size: var(--sp-fs-xl); color: var(--sp-text-1); }
  .subtitle { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.2rem 0 1.5rem; }

  .picker-card {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    padding: 1.25rem;
    box-shadow: var(--sp-shadow-1);
    margin-bottom: 1.5rem;
  }

  .picker-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .path-display {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    background: var(--sp-code-bg);
    padding: 0.4rem 0.75rem;
    border-radius: var(--sp-radius-sm);
    flex: 1;
    min-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.75rem 0 0; }
  .msg { margin-top: 0.75rem; padding: 0.6rem 1rem; border-radius: var(--sp-radius-md); font-size: var(--sp-fs-sm); }
  .msg.error {
    background: rgba(239, 68, 68, 0.12);
    color: var(--sp-danger);
    border: 1px solid rgba(239, 68, 68, 0.35);
  }
  .msg.success {
    background: rgba(132, 204, 22, 0.12);
    color: var(--sp-success);
    border: 1px solid rgba(132, 204, 22, 0.35);
  }
  .msg .link {
    background: none;
    border: none;
    color: var(--sp-blue);
    cursor: pointer;
    text-decoration: underline;
    font-size: var(--sp-fs-sm);
  }

  .editor { margin-top: 0; }

  .editor-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1rem;
  }

  .editor-header h2 { margin: 0; font-size: var(--sp-fs-lg); color: var(--sp-text-1); }
  .desc { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.15rem 0 0; }
  .save-btn { white-space: nowrap; }

  .diagnostics {
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    padding: 0.75rem 1rem;
    margin-bottom: 1rem;
  }

  .diagnostics h3 {
    margin: 0 0 0.5rem;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
  }

  .diag-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 0.25rem 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .diag-row.diag-warning .diag-text { color: var(--sp-amber); }
  .diag-row.diag-error .diag-text { color: var(--sp-danger); }

  .diag-dot {
    width: 6px;
    height: 6px;
    flex-shrink: 0;
    align-self: center;
    border-radius: var(--sp-radius-full);
    background: var(--sp-info);
    opacity: 0.7;
  }

  .diag-dot-warning { background: var(--sp-warning); }
  .diag-dot-error { background: var(--sp-danger); }

  .diag-text { flex: 1; word-break: break-word; }
  .diag-file { opacity: 0.6; font-family: var(--sp-font-mono); }
  .diag-conf { flex-shrink: 0; opacity: 0.6; }

  .action-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .add-bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.75rem 0;
    flex-wrap: wrap;
  }

  .add-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
  }

  .add-hint {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .template-panel {
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    padding: 0.6rem;
    margin-bottom: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .tpl-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.6rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .tpl-body { flex: 1; min-width: 0; }
  .tpl-name { font-size: var(--sp-fs-sm); font-weight: var(--sp-fw-semibold); color: var(--sp-text-1); }
  .tpl-desc { font-size: var(--sp-fs-xs); color: var(--sp-text-3); }
  .tpl-fields { display: flex; flex-direction: column; gap: 0.3rem; }
  .tpl-fields input,
  .tpl-fields .tpl-sm {
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    font-family: inherit;
    width: 100%;
    box-sizing: border-box;
  }
  .tpl-fields input:focus {
    outline: none;
    border-color: var(--sp-accent);
  }
  .tpl-inline { flex-direction: row; align-items: center; flex-wrap: wrap; gap: 0.4rem; }
  .tpl-inline .tpl-sm { width: auto; min-width: 90px; }

  .action-card {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
    cursor: grab;
    transition: border-color 0.15s, opacity 0.15s;
  }

  .action-card:active { cursor: grabbing; }
  .action-card.drag-over {
    border-color: var(--sp-accent);
    box-shadow: var(--sp-shadow-accent);
  }

  .action-card.disabled { opacity: 0.5; }

  .card-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 0.8rem;
  }

  .card-info {
    flex: 1;
    min-width: 0;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .card-label {
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
    line-height: 1.3;
    color: var(--sp-text-1);
  }

  .card-type {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .card-summary {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
    margin-top: 0.1rem;
  }

  .card-controls {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-shrink: 0;
  }

  .toggle-btn {
    padding: 0.2rem 0.5rem;
    border-radius: var(--sp-radius-xs);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-bold);
    cursor: pointer;
    transition: all 0.15s;
  }

  .toggle-btn.on {
    background: rgba(132, 204, 22, 0.14);
    border-color: rgba(132, 204, 22, 0.4);
    color: var(--sp-success);
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.25rem 0.5rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
    font-size: var(--sp-fs-xs);
    transition: all 0.15s;
  }

  .icon-btn:hover { background: var(--sp-bg-2); color: var(--sp-text-1); }
  .icon-btn:active { background: var(--sp-bg-3); }
  .expand-btn { min-width: 2em; }
  .expand-btn :global(.sp-icon) {
    transition: transform 0.15s ease;
  }
  .expand-btn.expanded :global(.sp-icon) {
    transform: rotate(90deg);
  }
  .delete-btn:hover { background: rgba(239, 68, 68, 0.15); color: var(--sp-danger); }

  .card-editor {
    border-top: 1px solid var(--sp-border);
    padding: 0.75rem 0.8rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    background: var(--sp-bg-2);
    border-radius: 0 0 var(--sp-radius-lg) var(--sp-radius-lg);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .field label {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-2);
  }

  .field input,
  .field select,
  .field textarea {
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-sm);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    transition: border-color 0.15s;
    font-family: inherit;
  }

  .field input:focus,
  .field select:focus,
  .field textarea:focus {
    outline: none;
    border-color: var(--sp-accent);
    box-shadow: var(--sp-shadow-accent);
  }

  .chips { display: flex; flex-wrap: wrap; gap: 0.35rem; }

  .chip {
    padding: 0.15rem 0.5rem;
    border-radius: var(--sp-radius-xs);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-1);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .footer-save {
    margin-top: 1.5rem;
    text-align: center;
  }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: none;
    background: var(--sp-accent-strong);
    color: #fff;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    transition: background 0.15s;
  }
  button.primary:hover:not(:disabled) { background: var(--sp-accent); }
  button.primary:disabled { opacity: 0.5; cursor: not-allowed; }

  button.secondary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    cursor: pointer;
    transition: background 0.15s;
  }
  button.secondary:hover:not(:disabled) { background: var(--sp-bg-3); }
  button.secondary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>