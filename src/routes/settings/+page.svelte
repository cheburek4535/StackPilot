<script lang="ts">
  import { onMount } from "svelte";
  import { getSettings, updateSettings, resetSettings } from "$lib/core/api";
  import type { AppSettings } from "$lib/core/types";
  import { i18n, availableLocales } from "$lib/core/i18n.svelte";

  let settings = $state<AppSettings | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let status = $state("");
  let statusType = $state("");

  onMount(async () => {
    try {
      settings = await getSettings();
    } catch (e) {
      status = `${i18n.t('error.load')}: ${e}`;
      statusType = "error";
    } finally {
      loading = false;
    }
  });

  async function handleSave() {
    if (!settings) return;
    saving = true;
    status = "";
    try {
      await updateSettings(settings);
      status = i18n.t("settings.status.success");
      statusType = "success";
    } catch (e) {
      status = `${i18n.t('settings.status.error')}: ${e}`;
      statusType = "error";
    } finally {
      saving = false;
      setTimeout(() => (status = ""), 3000);
    }
  }

  async function handleReset() {
    if (!confirm(i18n.t("settings.confirm_reset"))) return;
    saving = true;
    status = "";
    try {
      settings = await resetSettings();
      status = i18n.t("settings.status.reset_success");
      statusType = "success";
    } catch (e) {
      status = `${i18n.t('settings.status.error')}: ${e}`;
      statusType = "error";
    } finally {
      saving = false;
      setTimeout(() => (status = ""), 3000);
    }
  }

  function addApp() {
    if (!settings) return;
    settings = {
      ...settings,
      preferred_apps: [...settings.preferred_apps, { name: "", path: "", args: null }]
    };
  }

  function removeApp(index: number) {
    if (!settings) return;
    const apps = [...settings.preferred_apps];
    apps.splice(index, 1);
    settings = { ...settings, preferred_apps: apps };
  }

  function updateApp(index: number, field: "name" | "path" | "args", value: string | null) {
    if (!settings) return;
    const apps = [...settings.preferred_apps];
    apps[index] = { ...apps[index], [field]: value };
    settings = { ...settings, preferred_apps: apps };
  }
</script>

<main>
  <h1>⚙ {i18n.t("settings.title")}</h1>
  <p class="subtitle">{i18n.t("settings.subtitle")}</p>

  {#if loading}
    <p class="loading">{i18n.t("settings.loading")}...</p>
  {:else if settings}
    <div class="toolbar">
      <button onclick={handleSave} disabled={saving}>
        {saving ? i18n.t("settings.btn.saving") : i18n.t("settings.btn.save")}
      </button>
      <button class="secondary" onclick={handleReset} disabled={saving}>
        {i18n.t("settings.btn.reset")}
      </button>
    </div>

    {#if status}
      <div class="status" class:success={statusType === "success"} class:error={statusType === "error"}>
        {status}
      </div>
    {/if}

    <section>
      <h2>{i18n.t("settings.section.system")}</h2>
      <div class="card">
        <label>
          <span>{i18n.t("settings.system.vscode")}</span>
          <input type="text" bind:value={settings.vscode_path} placeholder="code" />
        </label>
        <label>
          <span>{i18n.t("settings.system.browser")}</span>
          <input type="text" bind:value={settings.browser_path} placeholder={i18n.t("settings.default.system")} />
        </label>
        <label>
          <span>{i18n.t("settings.system.terminal")}</span>
          <input type="text" bind:value={settings.terminal} placeholder={i18n.t("settings.default.system")} />
        </label>
      </div>
    </section>

    <section>
      <h2>{i18n.t("settings.section.appearance")}</h2>
      <div class="card">
        <label>
          <span>{i18n.t("settings.appearance.theme")}</span>
          <select bind:value={settings.theme}>
            <option value="system">{i18n.t("settings.theme.system")}</option>
            <option value="light">{i18n.t("settings.theme.light")}</option>
            <option value="dark">{i18n.t("settings.theme.dark")}</option>
          </select>
        </label>
        <label>
          <span>{i18n.t("settings.appearance.language")}</span>
          <select value={i18n.locale} onchange={(e) => {
            const val = (e.target).value;
            i18n.setLocale(val);
            settings.language = val;
          }}>
            {#each availableLocales as loc}
              <option value={loc.id}>{loc.icon} {loc.name} ({loc.index})</option>
            {/each}
          </select>
        </label>
      </div>
    </section>

    <section>
      <h2>{i18n.t("settings.section.behavior")}</h2>
      <div class="card">
        <label class="checkbox">
          <input type="checkbox" bind:checked={settings.auto_save_profiles} />
          <span>{i18n.t("settings.behavior.autosave")}</span>
        </label>
      </div>
    </section>

    <section>
      <h2>{i18n.t("settings.section.apps")}</h2>
      <p class="hint">{i18n.t("settings.apps.hint")}</p>
      <div class="card">
        {#if settings.preferred_apps.length === 0}
          <p class="empty">{i18n.t("settings.apps.empty")}</p>
        {:else}
          {#each settings.preferred_apps as app, i}
            <div class="app-row">
              <input
                type="text"
                placeholder={i18n.t("settings.apps.name")}
                value={app.name}
                oninput={(e) => updateApp(i, "name", (e.target).value)}
              />
              <input
                type="text"
                placeholder={i18n.t("settings.apps.path")}
                value={app.path}
                oninput={(e) => updateApp(i, "path", (e.target).value)}
              />
              <input
                type="text"
                placeholder={i18n.t("settings.apps.args")}
                value={app.args ?? ""}
                oninput={(e) => updateApp(i, "args", (e.target).value || null)}
              />
              <button class="icon-btn danger" onclick={() => removeApp(i)} title={i18n.t("settings.apps.remove")}>✕</button>
            </div>
          {/each}
        {/if}
        <button class="secondary add-btn" onclick={addApp}>+ {i18n.t("settings.apps.add")}</button>
      </div>
    </section>
  {/if}
</main>
<style>
  :root { font-family: Inter, Avenir, Helvetica, Arial, sans-serif; font-size: 16px; color: #0f0f0f; background-color: #f6f6f6; }
  main { max-width: 680px; margin: 0 auto; padding: 2rem; }
  h1 { margin: 0 0 1.5rem; font-size: 1.3rem; }
  .subtitle { color: #888; font-size: 0.9rem; margin: -1rem 0 1.5rem; }
  .loading { color: #888; font-style: italic; }
  .toolbar { display: flex; gap: 0.75rem; margin-bottom: 1rem; }
  .status { padding: 0.6rem 1rem; border-radius: 8px; font-size: 0.9rem; margin-bottom: 1rem; }
  .status.success { background: #e8f5e9; color: #2e7d32; border: 1px solid #a5d6a7; }
  .status.error { background: #ffebee; color: #c62828; border: 1px solid #ef9a9a; }
  section { margin-bottom: 1.5rem; }
  h2 { font-size: 1rem; margin: 0 0 0.5rem; color: #444; }
  .hint { font-size: 0.85rem; color: #888; margin: -0.25rem 0 0.5rem; }
  .card { background: #fff; border: 1px solid #e0e0e0; border-radius: 10px; padding: 1.25rem; display: flex; flex-direction: column; gap: 1rem; box-shadow: 0 1px 4px rgba(0,0,0,0.06); }
  label { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; font-weight: 500; color: #555; }
  label.checkbox { flex-direction: row; align-items: center; gap: 0.6rem; }
  input[type="text"], select { padding: 0.5rem 0.75rem; border: 1px solid #ccc; border-radius: 6px; font-size: 0.9rem; background: #fafafa; transition: border-color 0.15s; }
  input[type="text"]:focus, select:focus { outline: none; border-color: #396cd8; background: #fff; }
  input[type="checkbox"] { width: 1.1rem; height: 1.1rem; cursor: pointer; }
  .empty { color: #999; font-style: italic; font-size: 0.9rem; margin: 0; }
  .app-row { display: flex; gap: 0.5rem; align-items: center; }
  .app-row input { flex: 1; min-width: 0; }
  .app-row input:first-child { max-width: 160px; }
  .add-btn { align-self: flex-start; }
  button { border-radius: 8px; border: 1px solid transparent; padding: 0.5em 1.2em; font-size: 0.9rem; cursor: pointer; background: #396cd8; color: #fff; font-weight: 500; transition: background 0.15s, opacity 0.15s; }
  button:hover:not(:disabled) { background: #2b5ab0; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  button.secondary { background: #fff; color: #444; border: 1px solid #ccc; }
  button.secondary:hover:not(:disabled) { background: #f0f0f0; }
  button.icon-btn { padding: 0.4rem 0.6rem; font-size: 0.85rem; background: transparent; color: #888; border: 1px solid #ddd; }
  button.icon-btn:hover:not(:disabled) { background: #f5f5f5; color: #c62828; border-color: #ef9a9a; }
  @media (prefers-color-scheme: dark) {
    :root { color: #f6f6f6; background-color: #2f2f2f; }
    .card { background: #0f0f0f98; border-color: #444; }
    h2 { color: #bbb; }
    .subtitle { color: #888; }
    input[type="text"], select { background: #1a1a1a; border-color: #555; color: #eee; }
    input[type="text"]:focus, select:focus { border-color: #5b8def; background: #222; }
    button.secondary { background: #2a2a2a; color: #ccc; border-color: #555; }
    button.secondary:hover:not(:disabled) { background: #333; }
    button.icon-btn { color: #888; border-color: #555; }
    button.icon-btn:hover:not(:disabled) { background: #333; color: #ef9a9a; border-color: #ef9a9a; }
  }
</style>
