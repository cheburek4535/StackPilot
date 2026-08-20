<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { analyzeProject, saveProfile } from "$lib/modules/devlauncher/api";
  import { goto } from "$app/navigation";
  import type { LaunchProfile, LaunchAction, ActionType } from "$lib/modules/devlauncher/types";

  let projectPath = $state("");
  let profile = $state<LaunchProfile | null>(null);
  let loading = $state(false);
  let saving = $state(false);
  let error = $state("");
  let savedOk = $state(false);

  let dragIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  let expanded = $state(new Set<string>());

  let showAddMenu = $state(false);

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Выберите папку проекта",
    });
    if (selected) {
      projectPath = selected;
      profile = null;
      error = "";
      savedOk = false;
    }
  }

  async function handleAnalyze() {
    if (!projectPath) return;
    loading = true;
    error = "";
    savedOk = false;
    profile = null;
    expanded = new Set();
    try {
      profile = await analyzeProject(projectPath);
    } catch (e) {
      error = `Ошибка анализа: ${e}`;
    }
    loading = false;
  }

  async function handleSave() {
    if (!profile) return;
    saving = true;
    error = "";
    savedOk = false;
    try {
      await saveProfile(profile);
      savedOk = true;
    } catch (e) {
      error = `Ошибка сохранения: ${e}`;
    }
    saving = false;
  }

  function onDragStart(e: DragEvent, index: number) {
    dragIndex = index;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
    }
  }

  function onDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverIndex = index;
  }

  function onDragLeave() {
    dragOverIndex = null;
  }

  function onDrop(index: number) {
    if (dragIndex === null || dragIndex === index || !profile) return;
    const actions = [...profile.actions];
    const [moved] = actions.splice(dragIndex, 1);
    actions.splice(index, 0, moved);
    profile = { ...profile, actions };
    dragIndex = null;
    dragOverIndex = null;
  }

  function onDragEnd() {
    dragIndex = null;
    dragOverIndex = null;
  }

  function toggleExpand(id: string) {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function removeAction(index: number) {
    if (!profile) return;
    const actions = profile.actions.filter((_, i) => i !== index);
    profile = { ...profile, actions };
  }

  function toggleEnabled(index: number) {
    if (!profile) return;
    const actions = profile.actions.map((a, i) =>
      i === index ? { ...a, enabled: !a.enabled } : a,
    );
    profile = { ...profile, actions };
  }

  function updateLabel(index: number, label: string) {
    if (!profile) return;
    const actions = profile.actions.map((a, i) =>
      i === index ? { ...a, label } : a,
    );
    profile = { ...profile, actions };
  }

  function updateActionField(index: number, path: string[], value: unknown) {
    if (!profile) return;
    const actions: LaunchAction[] = profile.actions.map((a, i) => {
      if (i !== index) return a;
      return setNestedField(a, path, value) as LaunchAction;
    });
    profile = { ...profile, actions };
  }

  function setNestedField(obj: unknown, path: string[], value: unknown): unknown {
    if (path.length === 0) return value;
    const [first, ...rest] = path;
    if (typeof obj === "object" && obj !== null && !Array.isArray(obj)) {
      const record = obj as Record<string, unknown>;
      if (first === "action_type" && rest.length > 0) {
        const currentType = record.action_type as Record<string, unknown>;
        const variantKey = Object.keys(currentType)[0];
        if (rest.length === 1 && rest[0] === variantKey) {
          const nested = setNestedField(currentType[variantKey], [], value);
          return { ...record, action_type: { [variantKey]: nested } };
        }
        if (rest[0] === variantKey) {
          const nested = setNestedField(currentType[variantKey], rest.slice(1), value);
          return { ...record, action_type: { [variantKey]: nested } };
        }
        if (rest[0] !== variantKey) {
          return record;
        }
      }
      return { ...record, [first]: setNestedField(record[first], rest, value) };
    }
    if (Array.isArray(obj)) {
      const idx = Number(first);
      const arr = [...obj];
      arr[idx] = setNestedField(arr[idx], rest, value);
      return arr;
    }
    return obj;
  }

  function changeActionType(index: number, newType: string) {
    if (!profile) return;
    const actions = profile.actions.map((a, i) => {
      if (i !== index) return a;
      let action_type: ActionType;
      switch (newType) {
        case "RunCommand":
          action_type = { RunCommand: { command: "", working_dir: projectPath || null } };
          break;
        case "OpenUrl":
          action_type = { OpenUrl: { url: "" } };
          break;
        case "OpenApplication":
          action_type = { OpenApplication: { path: "", args: null } };
          break;
        case "WaitForUrl":
          action_type = { WaitForUrl: { url: "", timeout_secs: 30 } };
          break;
        case "WaitForPort":
          action_type = { WaitForPort: { host: "localhost", port: 3000, timeout_secs: 30 } };
          break;
        case "Delay":
          action_type = { Delay: { seconds: 5 } };
          break;
        case "ExecuteScript":
          action_type = { ExecuteScript: { script: "", shell: "cmd" } };
          break;
        default:
          return a;
      }
      return { ...a, action_type };
    });
    profile = { ...profile, actions };
  }

  function createActionWithType(type: string, projectPath: string): LaunchAction {
    let action_type: ActionType;
    switch (type) {
      case "RunCommand":
        action_type = { RunCommand: { command: "", working_dir: projectPath || null } };
        break;
      case "OpenUrl":
        action_type = { OpenUrl: { url: "" } };
        break;
      case "OpenApplication":
        action_type = { OpenApplication: { path: "", args: null } };
        break;
      case "WaitForUrl":
        action_type = { WaitForUrl: { url: "", timeout_secs: 30 } };
        break;
      case "WaitForPort":
        action_type = { WaitForPort: { host: "localhost", port: 3000, timeout_secs: 30 } };
        break;
      case "Delay":
        action_type = { Delay: { seconds: 5 } };
        break;
      case "ExecuteScript":
        action_type = { ExecuteScript: { script: "", shell: "cmd" } };
        break;
      default:
        action_type = { RunCommand: { command: "", working_dir: projectPath || null } };
    }
    return {
      id: `act_${Date.now()}_${Math.random().toString(36).slice(2)}`,
      label: "Новое действие",
      enabled: true,
      action_type,
    };
  }

  function addAction(actionType: string) {
    if (!profile) return;
    showAddMenu = false;
    const newAction = createActionWithType(actionType, projectPath);
    profile = { ...profile, actions: [...profile.actions, newAction] };
    const next = new Set(expanded);
    next.add(newAction.id);
    expanded = next;
  }

  function actionIcon(act: ActionType): string {
    if ("RunCommand" in act) return "▶";
    if ("OpenUrl" in act) return "🌐";
    if ("OpenApplication" in act) return "⬛";
    if ("WaitForUrl" in act) return "⏳";
    if ("WaitForPort" in act) return "🔌";
    if ("Delay" in act) return "⏱";
    if ("ExecuteScript" in act) return "📜";
    return "?";
  }

  function actionTypeName(act: ActionType): string {
    if ("RunCommand" in act) return "Команда";
    if ("OpenUrl" in act) return "URL";
    if ("OpenApplication" in act) return "Приложение";
    if ("WaitForUrl" in act) return "Ожидание URL";
    if ("WaitForPort" in act) return "Ожидание порта";
    if ("Delay" in act) return "Пауза";
    if ("ExecuteScript" in act) return "Скрипт";
    return "?";
  }

  function actionSummary(act: ActionType): string {
    if ("RunCommand" in act) return act.RunCommand.command || "(пусто)";
    if ("OpenUrl" in act) return act.OpenUrl.url || "(пусто)";
    if ("OpenApplication" in act) return act.OpenApplication.path || "(пусто)";
    if ("WaitForUrl" in act) return act.WaitForUrl.url || "(пусто)";
    if ("WaitForPort" in act) return `${act.WaitForPort.host}:${act.WaitForPort.port}`;
    if ("Delay" in act) return `${act.Delay.seconds}с`;
    if ("ExecuteScript" in act) return act.ExecuteScript.script || "(пусто)";
    return "?";
  }

  function actionVariant(act: ActionType): string {
    return Object.keys(act)[0];
  }

  function variantValue(act: ActionType): Record<string, unknown> {
    const key = actionVariant(act);
    return (act as Record<string, unknown>)[key] as Record<string, unknown>;
  }

  const actionTypes = [
    { key: "RunCommand", icon: "▶", label: "Команда", desc: "Запустить команду в терминале" },
    { key: "OpenUrl", icon: "🌐", label: "URL", desc: "Открыть веб-страницу" },
    { key: "OpenApplication", icon: "⬛", label: "Приложение", desc: "Запустить приложение" },
    { key: "WaitForUrl", icon: "⏳", label: "Ожидание URL", desc: "Ждать пока URL ответит" },
    { key: "WaitForPort", icon: "🔌", label: "Ожидание порта", desc: "Ждать TCP порт" },
    { key: "Delay", icon: "⏱", label: "Пауза", desc: "Подождать N секунд" },
    { key: "ExecuteScript", icon: "📜", label: "Скрипт", desc: "Выполнить скрипт" },
  ];

  function getEditorFields(act: ActionType): Array<{ label: string; path: string[]; type: string; value: unknown; placeholder: string }> {
    const key = Object.keys(act)[0];
    const val = (act as Record<string, unknown>)[key] as Record<string, unknown>;

    switch (key) {
      case "RunCommand": {
        const v = val as { command: string; working_dir: string | null };
        return [
          { label: "Команда", path: ["action_type", "RunCommand", "command"], type: "text", value: v.command ?? "", placeholder: "npm run dev" },
          { label: "Рабочая папка", path: ["action_type", "RunCommand", "working_dir"], type: "text", value: v.working_dir ?? "", placeholder: "оставить пустым для корня проекта" },
        ];
      }
      case "OpenUrl": {
        const v = val as { url: string };
        return [
          { label: "URL", path: ["action_type", "OpenUrl", "url"], type: "text", value: v.url ?? "", placeholder: "http://localhost:3000" },
        ];
      }
      case "OpenApplication": {
        const v = val as { path: string; args: string | null };
        return [
          { label: "Путь к приложению", path: ["action_type", "OpenApplication", "path"], type: "text", value: v.path ?? "", placeholder: "code" },
          { label: "Аргументы", path: ["action_type", "OpenApplication", "args"], type: "text", value: v.args ?? "", placeholder: "." },
        ];
      }
      case "WaitForUrl": {
        const v = val as { url: string; timeout_secs: number };
        return [
          { label: "URL", path: ["action_type", "WaitForUrl", "url"], type: "text", value: v.url ?? "", placeholder: "http://localhost:3000/health" },
          { label: "Таймаут (сек)", path: ["action_type", "WaitForUrl", "timeout_secs"], type: "number", value: v.timeout_secs, placeholder: "30" },
        ];
      }
      case "WaitForPort": {
        const v = val as { host: string; port: number; timeout_secs: number };
        return [
          { label: "Хост", path: ["action_type", "WaitForPort", "host"], type: "text", value: v.host ?? "", placeholder: "localhost" },
          { label: "Порт", path: ["action_type", "WaitForPort", "port"], type: "number", value: v.port, placeholder: "3000" },
          { label: "Таймаут (сек)", path: ["action_type", "WaitForPort", "timeout_secs"], type: "number", value: v.timeout_secs, placeholder: "30" },
        ];
      }
      case "Delay": {
        const v = val as { seconds: number };
        return [
          { label: "Секунд", path: ["action_type", "Delay", "seconds"], type: "number", value: v.seconds, placeholder: "5" },
        ];
      }
      case "ExecuteScript": {
        const v = val as { script: string; shell: string | null };
        return [
          { label: "Скрипт", path: ["action_type", "ExecuteScript", "script"], type: "textarea", value: v.script ?? "", placeholder: "echo Hello" },
          { label: "Оболочка", path: ["action_type", "ExecuteScript", "shell"], type: "text", value: v.shell ?? "cmd", placeholder: "cmd" },
        ];
      }
      default:
        return [];
    }
  }
</script>

<main>
  <h1>📊 Анализ проекта</h1>
  <p class="subtitle">Выберите папку с проектом — DevLauncher проанализирует структуру и предложит готовый профиль запуска</p>

  <div class="picker-card">
    <div class="picker-row">
      <button class="primary" onclick={pickFolder}>📁 Выбрать папку</button>
      {#if projectPath}
        <span class="path-display">{projectPath}</span>
        <button class="secondary" onclick={handleAnalyze} disabled={loading}>
          {loading ? "Анализирую..." : "🔍 Анализировать"}
        </button>
      {/if}
    </div>
    {#if !projectPath}
      <p class="hint">Нажмите «Выбрать папку», затем «Анализировать»</p>
    {/if}

    {#if error}
      <div class="msg error">{error}</div>
    {/if}

    {#if savedOk}
      <div class="msg success">
        ✅ Профиль сохранён!
        <button class="link" onclick={() => goto("/devlauncher/profiles")}>Go to profiles →</button>
      </div>
    {/if}
  </div>

  {#if profile}
    <section class="editor">
      <div class="editor-header">
        <div>
          <h2>📦 {profile.name}</h2>
          <p class="desc">{profile.description}</p>
        </div>
        <button class="primary save-btn" onclick={handleSave} disabled={saving}>
          {saving ? "Сохранение..." : "💾 Сохранить профиль"}
        </button>
      </div>

      <div class="action-list" role="list">
        {#each profile.actions as action, i (action.id)}
          <div
            class="action-card"
            class:expanded={expanded.has(action.id)}
            class:disabled={!action.enabled}
            class:drag-over={dragOverIndex === i}
            draggable="true"
            ondragstart={(e) => onDragStart(e, i)}
            ondragover={(e) => onDragOver(e, i)}
            ondragleave={onDragLeave}
            ondrop={(e) => { e.preventDefault(); onDrop(i); }}
            ondragend={onDragEnd}
          >
            <div class="card-header">
              <span class="drag-handle" title="Перетащить чтобы изменить порядок">⠿</span>
              <span class="card-icon">{actionIcon(action.action_type)}</span>
              <div class="card-info" onclick={() => toggleExpand(action.id)} role="button" tabindex="0" onkeydown={(e) => e.key === "Enter" && toggleExpand(action.id)}>
                <span class="card-label">{action.label || "(без названия)"}</span>
                <span class="card-type">{actionTypeName(action.action_type)}</span>
                <span class="card-summary">{actionSummary(action.action_type)}</span>
              </div>
              <div class="card-controls">
                <button
                  class="toggle-btn"
                  class:on={action.enabled}
                  onclick={() => toggleEnabled(i)}
                  title={action.enabled ? "Выключить" : "Включить"}
                >
                  {action.enabled ? "ON" : "OFF"}
                </button>
                <button class="icon-btn" onclick={() => removeAction(i)} title="Удалить">✕</button>
                <button
                  class="icon-btn expand-btn"
                  onclick={() => toggleExpand(action.id)}
                  title="Редактировать"
                >
                  {expanded.has(action.id) ? "▲" : "▼"}
                </button>
              </div>
            </div>

            {#if expanded.has(action.id)}
              <div class="card-editor">
                <div class="field">
                  <label>Название</label>
                  <input
                    type="text"
                    value={action.label}
                    oninput={(e) => updateLabel(i, (e.target as HTMLInputElement).value)}
                    placeholder="Краткое описание действия"
                  />
                </div>
                <div class="field">
                  <label>Тип действия</label>
                  <select
                    value={actionVariant(action.action_type)}
                    onchange={(e) => changeActionType(i, (e.target as HTMLSelectElement).value)}
                  >
                    {#each actionTypes as at}
                      <option value={at.key}>{at.icon} {at.label}</option>
                    {/each}
                  </select>
                </div>

                {#each getEditorFields(action.action_type) as field}
                  <div class="field">
                    <label>{field.label}</label>
                    {#if field.type === "number"}
                      <input
                        type="number"
                        value={field.value as number}
                        oninput={(e) => {
                          const val = parseInt((e.target as HTMLInputElement).value) || 0;
                          updateActionField(i, field.path, val);
                        }}
                        placeholder={field.placeholder}
                      />
                    {:else if field.type === "textarea"}
                      <textarea
                        value={field.value as string}
                        oninput={(e) => {
                          updateActionField(i, field.path, (e.target as HTMLTextAreaElement).value);
                        }}
                        placeholder={field.placeholder}
                      ></textarea>
                    {:else}
                      <input
                        type="text"
                        value={field.value as string}
                        oninput={(e) => {
                          updateActionField(i, field.path, (e.target as HTMLInputElement).value);
                        }}
                        placeholder={field.placeholder}
                      />
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>

      <div class="add-wrapper">
        <button class="secondary add-btn" onclick={() => (showAddMenu = !showAddMenu)}>
          + Добавить действие
        </button>
        {#if showAddMenu}
          <div class="add-menu">
            {#each actionTypes as at}
              <button class="add-menu-item" onclick={() => addAction(at.key)}>
                <span class="add-icon">{at.icon}</span>
                <div>
                  <strong>{at.label}</strong>
                  <span class="add-desc">{at.desc}</span>
                </div>
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="footer-save">
        <button class="primary" onclick={handleSave} disabled={saving}>
          {saving ? "Сохранение..." : "💾 Сохранить профиль"}
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
    background: rgba(248, 113, 113, 0.12);
    color: var(--sp-danger);
    border: 1px solid rgba(248, 113, 113, 0.35);
  }
  .msg.success {
    background: rgba(163, 230, 53, 0.12);
    color: var(--sp-success);
    border: 1px solid rgba(163, 230, 53, 0.35);
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

  .action-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .action-card {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .action-card.drag-over {
    border-color: var(--sp-accent);
    box-shadow: var(--sp-shadow-accent);
  }

  .action-card.disabled {
    opacity: 0.5;
  }

  .card-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 0.8rem;
  }

  .drag-handle {
    cursor: grab;
    color: var(--sp-text-3);
    font-size: 1rem;
    letter-spacing: 2px;
    user-select: none;
    flex-shrink: 0;
  }

  .drag-handle:active {
    cursor: grabbing;
  }

  .card-icon {
    font-size: 1rem;
    width: 1.4rem;
    text-align: center;
    flex-shrink: 0;
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
    background: rgba(163, 230, 53, 0.14);
    border-color: rgba(163, 230, 53, 0.4);
    color: var(--sp-success);
  }

  .icon-btn {
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

  .card-editor {
    border-top: 1px solid var(--sp-border);
    padding: 0.75rem 0.8rem 1rem 2.7rem;
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

  .field textarea {
    min-height: 3rem;
    resize: vertical;
    font-family: var(--sp-font-mono);
  }

  .add-wrapper {
    position: relative;
    margin-top: 0.75rem;
  }

  .add-btn {
    width: 100%;
    padding: 0.7rem;
    border: 2px dashed var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    background: transparent;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    cursor: pointer;
    transition: all 0.15s;
  }

  .add-btn:hover {
    border-color: var(--sp-accent);
    color: var(--sp-accent);
    background: var(--sp-accent-soft);
  }

  .add-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    background: var(--sp-glass-strong);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-2);
    z-index: 50;
    overflow: hidden;
  }

  .add-menu-item {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.6rem 0.8rem;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    transition: background 0.1s;
  }

  .add-menu-item:hover { background: var(--sp-accent-soft); }

  .add-icon { font-size: 1.1rem; }

  .add-menu-item div { display: flex; flex-direction: column; }

  .add-menu-item strong { font-weight: var(--sp-fw-semibold); font-size: var(--sp-fs-sm); }

  .add-desc { font-size: var(--sp-fs-xs); color: var(--sp-text-3); }

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
