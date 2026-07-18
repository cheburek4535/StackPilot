<script lang="ts">
  // ================================================================
  // Страница настроек DevLauncher.
  //
  // Использует модуль SettingsService в Rust через $lib/api.
  // Демонстрирует типичный паттерн работы с формой:
  //   1. Загрузить данные при монтировании (onMount)
  //   2. Привязать поля формы к реактивным переменным (bind:value)
  //   3. Сохранить через вызов API
  //   4. Показать статус (успех/ошибка)
  // ================================================================

  import { onMount } from "svelte";
  import { getSettings, updateSettings, resetSettings } from "$lib/core/api";
  import type { AppSettings } from "$lib/core/types";

  // ---- Состояние формы ----

  /// Текущие настройки (загружаются с бэкенда)
  let settings = $state<AppSettings | null>(null);

  /// Флаг загрузки
  let loading = $state(true);

  /// Флаг сохранения (блокирует кнопки)
  let saving = $state(false);

  /// Сообщение для пользователя (успех или ошибка)
  let status = $state("");

  /// Тип сообщения: "success" | "error" | ""
  let statusType = $state("");

  // ---- Загрузка ----

  onMount(async () => {
    try {
      settings = await getSettings();
    } catch (e) {
      status = `Ошибка загрузки настроек: ${e}`;
      statusType = "error";
    }
    loading = false;
  });

  // ---- Действия с настройками ----

  /// Сохранить настройки на бэкенде
  async function handleSave() {
    if (!settings) return;
    saving = true;
    status = "";
    statusType = "";
    try {
      await updateSettings(settings);
      status = "Настройки сохранены";
      statusType = "success";
    } catch (e) {
      status = `Ошибка сохранения: ${e}`;
      statusType = "error";
    }
    saving = false;
  }

  /// Сбросить настройки на значения по умолчанию
  async function handleReset() {
    saving = true;
    status = "";
    statusType = "";
    try {
      settings = await resetSettings();
      status = "Настройки сброшены на значения по умолчанию";
      statusType = "success";
    } catch (e) {
      status = `Ошибка сброса: ${e}`;
      statusType = "error";
    }
    saving = false;
  }

  // ---- Управление списком приложений ----

  /// Добавить новое приложение в список
  function addApp() {
    if (!settings) return;
    // Создаём новый массив (Svelte 5 отслеживает перезапись)
    settings = {
      ...settings,
      preferred_apps: [
        ...settings.preferred_apps,
        { name: "", path: "", args: null },
      ],
    };
  }

  /// Удалить приложение из списка по индексу
  function removeApp(index: number) {
    if (!settings) return;
    settings = {
      ...settings,
      preferred_apps: settings.preferred_apps.filter((_, i) => i !== index),
    };
  }

  /// Обновить конкретное поле приложения (для bind:value вложенных объектов)
  function updateApp(index: number, field: "name" | "path" | "args", value: string | null) {
    if (!settings) return;
    const apps = [...settings.preferred_apps];
    apps[index] = { ...apps[index], [field]: value };
    settings = { ...settings, preferred_apps: apps };
  }
</script>

<main>
  <h1>⚙ Настройки</h1>
  <p class="subtitle">Глобальные параметры DevLauncher</p>

  {#if loading}
    <p class="loading">Загрузка настроек...</p>

  {:else if settings}
    <!-- ===== Панель действий ===== -->
    <div class="toolbar">
      <button onclick={handleSave} disabled={saving}>
        {saving ? "Сохранение..." : "Сохранить настройки"}
      </button>
      <button class="secondary" onclick={handleReset} disabled={saving}>
        Сбросить на defaults
      </button>
    </div>

    {#if status}
      <div class="status" class:success={statusType === "success"} class:error={statusType === "error"}>
        {status}
      </div>
    {/if}

    <!-- ===== Система ===== -->
    <section>
      <h2>Система</h2>
      <div class="card">
        <label>
          <span>VS Code path</span>
          <input type="text" bind:value={settings.vscode_path} placeholder="code" />
        </label>
        <label>
          <span>Браузер</span>
          <input type="text" bind:value={settings.browser_path} placeholder="Системный по умолчанию" />
        </label>
        <label>
          <span>Терминал</span>
          <input type="text" bind:value={settings.terminal} placeholder="Системный по умолчанию" />
        </label>
      </div>
    </section>

    <!-- ===== Внешний вид ===== -->
    <section>
      <h2>Внешний вид</h2>
      <div class="card">
        <label>
          <span>Тема</span>
          <select bind:value={settings.theme}>
            <option value="system">Системная</option>
            <option value="light">Светлая</option>
            <option value="dark">Тёмная</option>
          </select>
        </label>
        <label>
          <span>Язык</span>
          <select bind:value={settings.language}>
            <option value="ru">Русский</option>
            <option value="en">English</option>
          </select>
        </label>
      </div>
    </section>

    <!-- ===== Поведение ===== -->
    <section>
      <h2>Поведение</h2>
      <div class="card">
        <label class="checkbox">
          <input type="checkbox" bind:checked={settings.auto_save_profiles} />
          <span>Автоматически сохранять профили при изменении</span>
        </label>
      </div>
    </section>

    <!-- ===== Предпочитаемые приложения ===== -->
    <section>
      <h2>Предпочитаемые приложения</h2>
      <p class="hint">
        Приложения, которые DevLauncher может открывать.
        Например: VS Code, браузер, терминал.
      </p>

      <div class="card">
        {#if settings.preferred_apps.length === 0}
          <p class="empty">Нет добавленных приложений.</p>
        {:else}
          {#each settings.preferred_apps as app, i}
            <div class="app-row">
              <input
                type="text"
                placeholder="Название"
                value={app.name}
                oninput={(e) => updateApp(i, "name", (e.target as HTMLInputElement).value)}
              />
              <input
                type="text"
                placeholder="Путь к исполняемому файлу"
                value={app.path}
                oninput={(e) => updateApp(i, "path", (e.target as HTMLInputElement).value)}
              />
              <input
                type="text"
                placeholder="Аргументы (опционально)"
                value={app.args ?? ""}
                oninput={(e) => updateApp(i, "args", (e.target as HTMLInputElement).value || null)}
              />
              <button class="icon-btn danger" onclick={() => removeApp(i)} title="Удалить">✕</button>
            </div>
          {/each}
        {/if}

        <button class="secondary add-btn" onclick={addApp}>+ Добавить приложение</button>
      </div>
    </section>

  {/if}
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    font-size: 16px;
    color: #0f0f0f;
    background-color: #f6f6f6;
  }

  main {
    max-width: 680px;
    margin: 0 auto;
    padding: 2rem;
  }

  h1 {
    margin: 0 0 1.5rem;
    font-size: 1.3rem;
  }

  .subtitle {
    color: #888;
    font-size: 0.9rem;
    margin: -1rem 0 1.5rem;
  }

  .loading {
    color: #888;
    font-style: italic;
  }

  .toolbar {
    display: flex;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .status {
    padding: 0.6rem 1rem;
    border-radius: 8px;
    font-size: 0.9rem;
    margin-bottom: 1rem;
  }
  .status.success {
    background: #e8f5e9;
    color: #2e7d32;
    border: 1px solid #a5d6a7;
  }
  .status.error {
    background: #ffebee;
    color: #c62828;
    border: 1px solid #ef9a9a;
  }

  section {
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.5rem;
    color: #444;
  }

  .hint {
    font-size: 0.85rem;
    color: #888;
    margin: -0.25rem 0 0.5rem;
  }

  .card {
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 10px;
    padding: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    box-shadow: 0 1px 4px rgba(0,0,0,0.06);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: #555;
  }

  label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.6rem;
  }

  input[type="text"],
  select {
    padding: 0.5rem 0.75rem;
    border: 1px solid #ccc;
    border-radius: 6px;
    font-size: 0.9rem;
    background: #fafafa;
    transition: border-color 0.15s;
  }

  input[type="text"]:focus,
  select:focus {
    outline: none;
    border-color: #396cd8;
    background: #fff;
  }

  input[type="checkbox"] {
    width: 1.1rem;
    height: 1.1rem;
    cursor: pointer;
  }

  .empty {
    color: #999;
    font-style: italic;
    font-size: 0.9rem;
    margin: 0;
  }

  .app-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .app-row input {
    flex: 1;
    min-width: 0;
  }
  .app-row input:first-child {
    max-width: 160px;
  }

  .add-btn {
    align-self: flex-start;
  }

  button {
    border-radius: 8px;
    border: 1px solid transparent;
    padding: 0.5em 1.2em;
    font-size: 0.9rem;
    cursor: pointer;
    background: #396cd8;
    color: #fff;
    font-weight: 500;
    transition: background 0.15s, opacity 0.15s;
  }

  button:hover:not(:disabled) {
    background: #2b5ab0;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  button.secondary {
    background: #fff;
    color: #444;
    border: 1px solid #ccc;
  }

  button.secondary:hover:not(:disabled) {
    background: #f0f0f0;
  }

  button.icon-btn {
    padding: 0.4rem 0.6rem;
    font-size: 0.85rem;
    background: transparent;
    color: #888;
    border: 1px solid #ddd;
  }

  button.icon-btn:hover:not(:disabled) {
    background: #f5f5f5;
    color: #c62828;
    border-color: #ef9a9a;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    .card {
      background: #0f0f0f98;
      border-color: #444;
    }

    h2 {
      color: #bbb;
    }

    .subtitle {
      color: #888;
    }

    input[type="text"],
    select {
      background: #1a1a1a;
      border-color: #555;
      color: #eee;
    }

    input[type="text"]:focus,
    select:focus {
      border-color: #5b8def;
      background: #222;
    }

    button.secondary {
      background: #2a2a2a;
      color: #ccc;
      border-color: #555;
    }

    button.secondary:hover:not(:disabled) {
      background: #333;
    }

    button.icon-btn {
      color: #888;
      border-color: #555;
    }

    button.icon-btn:hover:not(:disabled) {
      background: #333;
      color: #ef9a9a;
      border-color: #ef9a9a;
    }
  }
</style>
