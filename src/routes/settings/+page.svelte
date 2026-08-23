<script lang="ts">
  import { onMount } from "svelte";
  import {
    getSettings,
    updateSettings,
    resetSettings,
    checkPath,
    getAppDataDir,
  } from "$lib/core/api";
  import type { AppSettings } from "$lib/core/types";
  import { i18n, availableLocales } from "$lib/core/i18n.svelte";
  import type { Locale } from "$lib/core/i18n.svelte";
  import { applyTheme, applyUiPrefs, ACCENT_PRESETS } from "$lib/core/theme";
  import { notifySuccess, notifyError, notifyWarning } from "$lib/core/toasts";
  import { APP_NAME, APP_VERSION } from "$lib/core/app";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import type { IconName } from "$lib/components/ui/icons";

  type TabId = "general" | "system" | "display" | "behavior" | "apps" | "ai" | "about";

  let saved = $state<AppSettings | null>(null);
  let draft = $state<AppSettings | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  let activeTab = $state<TabId>("general");
  let keyVisible = $state(false);
  let customHex = $state("#8b5cf6");
  let dataDir = $state("");
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let checkTimer: ReturnType<typeof setTimeout> | undefined;
  let pathChecks = $state<Record<string, boolean | null>>({
    vscode_path: null,
    browser_path: null,
    terminal: null,
  });

  const dirty = $derived(
    !!saved && !!draft && JSON.stringify(saved) !== JSON.stringify(draft),
  );

  const emailInvalid = $derived(
    !!draft &&
      draft.personal.email.trim() !== "" &&
      !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(draft.personal.email.trim()),
  );

  const urlInvalid = $derived(
    !!draft &&
      draft.ai.base_url.trim() !== "" &&
      !/^https?:\/\//i.test(draft.ai.base_url.trim()),
  );

  const keyShort = $derived(
    !!draft && draft.ai.api_key.trim() !== "" && draft.ai.api_key.trim().length < 8,
  );

  const tabs: { id: TabId; label: string; icon: IconName; badge?: boolean }[] = [
    { id: "general", label: i18n.t("settings.tabs.general"), icon: "user" },
    { id: "system", label: i18n.t("settings.tabs.system"), icon: "terminal" },
    { id: "display", label: i18n.t("settings.tabs.display"), icon: "palette" },
    { id: "behavior", label: i18n.t("settings.tabs.behavior"), icon: "sliders" },
    { id: "apps", label: i18n.t("settings.tabs.apps"), icon: "layers" },
    { id: "ai", label: i18n.t("settings.tabs.ai"), icon: "bot", badge: true },
    { id: "about", label: i18n.t("settings.tabs.about"), icon: "info" },
  ];

  const PROVIDER_DEFAULTS: Record<string, string> = {
    openai: "https://api.openai.com/v1",
    anthropic: "https://api.anthropic.com",
    ollama: "http://localhost:11434/v1",
    custom: "",
  };

  onMount(async () => {
    try {
      const s = await getSettings();
      saved = s;
      draft = structuredClone(s);
      customHex = /^#([0-9a-f]{6})$/i.test(s.accent_color)
        ? s.accent_color
        : (ACCENT_PRESETS[s.accent_color] ?? ACCENT_PRESETS.violet);
      preview();
      void refreshPathCheck("vscode_path", s.vscode_path);
      void refreshPathCheck("browser_path", s.browser_path);
      void refreshPathCheck("terminal", s.terminal);
      try {
        dataDir = await getAppDataDir();
      } catch {
        dataDir = "";
      }
    } catch (e) {
      notifyError(i18n.t("error.load"), String(e));
    } finally {
      loading = false;
    }
  });

  function clone(s: AppSettings): AppSettings {
    return structuredClone(s);
  }

  function preview(): void {
    if (!draft) return;
    applyTheme(draft.theme as "dark" | "light" | "system");
    applyUiPrefs(draft);
  }

  function onAnyChange(): void {
    preview();
    if (draft?.auto_save) scheduleSave();
  }

  function scheduleSave(): void {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      void persist(false);
    }, 400);
  }

  async function persist(toastOnSuccess: boolean): Promise<void> {
    if (!draft || saving) return;
    saving = true;
    try {
      saved = await updateSettings(draft);
      preview();
      if (toastOnSuccess) notifySuccess(i18n.t("settings.status.success"));
    } catch (e) {
      notifyError(i18n.t("settings.status.error"), String(e));
    } finally {
      saving = false;
    }
  }

  function handleSave(): void {
    void persist(true);
  }

  function handleDiscard(): void {
    if (!saved) return;
    draft = clone(saved);
    customHex = /^#([0-9a-f]{6})$/i.test(saved.accent_color)
      ? saved.accent_color
      : (ACCENT_PRESETS[saved.accent_color] ?? ACCENT_PRESETS.violet);
    preview();
  }

  function handleAutosaveToggle(): void {
    if (!draft) return;
    preview();
    if (draft.auto_save) {
      void persist(false);
    } else {
      clearTimeout(saveTimer);
    }
  }

  function onLanguageChange(): void {
    if (!draft) return;
    i18n.setLocale(draft.language as Locale);
    preview();
    if (draft.auto_save) scheduleSave();
  }

  function setFontSize(size: string): void {
    if (!draft) return;
    draft.font_size = size;
    onAnyChange();
  }

  function setAccent(value: string): void {
    if (!draft) return;
    draft.accent_color = value;
    if (!/^#/.test(value)) customHex = ACCENT_PRESETS[value] ?? customHex;
    onAnyChange();
  }

  function onProviderChange(): void {
    if (!draft) return;
    const def = PROVIDER_DEFAULTS[draft.ai.provider] ?? "";
    if (!draft.ai.base_url.trim()) draft.ai.base_url = def;
    onAnyChange();
  }

  async function factoryReset(): Promise<void> {
    if (!draft) return;
    if (draft.confirm_before_reset && !confirm(i18n.t("settings.danger.confirm"))) return;
    saving = true;
    try {
      const r = await resetSettings();
      saved = r;
      draft = clone(r);
      customHex = ACCENT_PRESETS[r.accent_color] ?? r.accent_color;
      preview();
      notifySuccess(i18n.t("settings.status.reset_success"));
    } catch (e) {
      notifyError(i18n.t("settings.status.error"), String(e));
    } finally {
      saving = false;
    }
  }

  async function refreshPathCheck(field: string, path: string): Promise<void> {
    if (!path.trim()) {
      pathChecks[field] = null;
      return;
    }
    try {
      pathChecks[field] = await checkPath(path);
    } catch {
      pathChecks[field] = null;
    }
  }

  function onPathInput(field: string, path: string): void {
    onAnyChange();
    clearTimeout(checkTimer);
    checkTimer = setTimeout(() => {
      void refreshPathCheck(field, path);
    }, 350);
  }

  async function browseFor(field: "vscode_path" | "browser_path" | "terminal"): Promise<void> {
    if (!draft) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ multiple: false, directory: false });
      if (selected && typeof selected === "string") {
        draft[field] = selected;
        onAnyChange();
        void refreshPathCheck(field, selected);
      }
    } catch {
      notifyWarning(i18n.t("settings.system.browse_failed"));
    }
  }

  async function openDataDir(): Promise<void> {
    try {
      const dir = dataDir || (await getAppDataDir());
      const { openPath } = await import("@tauri-apps/plugin-opener");
      await openPath(dir);
    } catch {
      notifyWarning(i18n.t("settings.system.open_failed"));
    }
  }

  function addApp(): void {
    if (!draft) return;
    draft.preferred_apps = [...draft.preferred_apps, { name: "", path: "", args: null }];
    onAnyChange();
  }

  function removeApp(index: number): void {
    if (!draft) return;
    draft.preferred_apps = draft.preferred_apps.filter((_, i) => i !== index);
    onAnyChange();
  }

  function updateApp(index: number, field: "name" | "path" | "args", value: string | null): void {
    if (!draft) return;
    const apps = [...draft.preferred_apps];
    apps[index] = { ...apps[index], [field]: value };
    draft.preferred_apps = apps;
  }

  function initials(): string {
    const name = draft?.personal.name.trim() || draft?.personal.username.trim() || "?";
    return name
      .split(/\s+/)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase() ?? "")
      .join("");
  }

  function connectionPreview(): string {
    if (!draft) return "";
    const ai = draft.ai;
    return JSON.stringify(
      {
        provider: ai.provider,
        base_url: ai.base_url || "(empty)",
        model: ai.model || "(empty)",
        temperature: ai.temperature,
        max_tokens: ai.max_tokens,
        timeout_secs: ai.timeout_secs,
        page_context: ai.page_context,
        authorization: ai.api_key ? `Bearer ${ai.api_key.slice(0, 6)}вЂ¦` : null,
        system_prompt: ai.system_prompt || "(empty)",
      },
      null,
      2,
    );
  }
</script>

<PageContainer width="default">
  <PageHeader
    title={i18n.t("settings.title")}
    description={i18n.t("settings.subtitle")}
    icon="sliders"
  />

  {#if loading}
    <LoadingState label={`${i18n.t("settings.loading")}вЂ¦`} />
  {:else if draft}
    <div class="sp-sticky">
      <div class="sp-minitabs" role="tablist">
        {#each tabs as tab}
          <button
            type="button"
            role="tab"
            class="sp-minitab"
            class:sp-minitab-active={activeTab === tab.id}
            aria-selected={activeTab === tab.id}
            onclick={() => (activeTab = tab.id)}
          >
            <Icon name={tab.icon} size={15} />
            <span>{tab.label}</span>
            {#if tab.badge}
              <Badge tone="amber" dot>{i18n.t("settings.tabs.ai_badge")}</Badge>
            {/if}
          </button>
        {/each}
      </div>

      <div class="autosave-bar">
        <div class="autosave-text">
          <span class="autosave-icon" aria-hidden="true">
            <Icon name="save" size={17} />
          </span>
          <div class="autosave-heading">
            <strong>{i18n.t("settings.autosave.title")}</strong>
            <span class="sp-hint autosave-desc">{i18n.t("settings.autosave.desc")}</span>
          </div>
        </div>
        <div class="autosave-actions">
          {#if !draft.auto_save}
            {#if dirty}
              <Badge tone="amber" dot>{i18n.t("settings.dirty")}</Badge>
            {/if}
            <Button variant="primary" size="sm" icon="save" loading={saving} onclick={handleSave}>
              {i18n.t("settings.btn.save")}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              icon="refresh"
              disabled={!dirty || saving}
              onclick={handleDiscard}
            >
              {i18n.t("settings.btn.discard")}
            </Button>
          {/if}
          <label class="switch" title={i18n.t("settings.autosave.title")}>
            <input
              type="checkbox"
              bind:checked={draft.auto_save}
              onchange={handleAutosaveToggle}
            />
            <span class="switch-track"><span class="switch-knob"></span></span>
          </label>
        </div>
      </div>
    </div>

    {#if activeTab === "general"}
      <div class="sp-tab-panel">
        <Card title={i18n.t("settings.general.title")} description={i18n.t("settings.general.desc")}>
          <div class="identity">
            <span class="identity-avatar" aria-hidden="true">{initials()}</span>
            <div class="identity-text">
              <strong>{draft.personal.name || draft.personal.username || "StackPilot"}</strong>
              <span class="sp-hint">
                {draft.personal.username || "вЂ”"}
                {#if draft.personal.email} В· {draft.personal.email}{/if}
              </span>
            </div>
          </div>
          <div class="field field-stack">
            <label class="field-label" for="sp-name">{i18n.t("settings.general.name")}</label>
            <input
              id="sp-name"
              type="text"
              placeholder={i18n.t("settings.general.name_ph")}
              bind:value={draft.personal.name}
              oninput={onAnyChange}
            />
          </div>
          <div class="field field-stack">
            <label class="field-label" for="sp-username">{i18n.t("settings.general.username")}</label>
            <input
              id="sp-username"
              type="text"
              placeholder={i18n.t("settings.general.username_ph")}
              bind:value={draft.personal.username}
              oninput={onAnyChange}
            />
          </div>
          <div class="field field-stack">
            <label class="field-label" for="sp-email">{i18n.t("settings.general.email")}</label>
            <input
              id="sp-email"
              type="email"
              placeholder={i18n.t("settings.general.email_ph")}
              bind:value={draft.personal.email}
              oninput={onAnyChange}
            />
            {#if emailInvalid}
              <span class="warn-hint">{i18n.t("settings.general.email_invalid")}</span>
            {/if}
          </div>
        </Card>
      </div>
    {:else if activeTab === "system"}
      <div class="sp-tab-panel">
        <Card title={i18n.t("settings.section.system")} description={i18n.t("settings.system.hint")}>
          <div class="field field-stack">
            <label class="field-label" for="sp-vscode">{i18n.t("settings.system.vscode")}</label>
            <div class="path-row">
              <input
                id="sp-vscode"
                type="text"
                placeholder="code"
                bind:value={draft.vscode_path}
                oninput={(e) => onPathInput("vscode_path", (e.currentTarget).value)}
              />
              <span
                class="path-dot"
                class:path-ok={pathChecks.vscode_path === true}
                class:path-bad={pathChecks.vscode_path === false}
                title={pathChecks.vscode_path === false
                  ? i18n.t("settings.system.path_missing")
                  : i18n.t("settings.system.path_ok")}
              ></span>
              <Button
                variant="secondary"
                size="sm"
                icon="folder"
                onclick={() => browseFor("vscode_path")}
              >
                {i18n.t("settings.system.browse")}
              </Button>
            </div>
          </div>
          <div class="field field-stack">
            <label class="field-label" for="sp-browser">{i18n.t("settings.system.browser")}</label>
            <div class="path-row">
              <input
                id="sp-browser"
                type="text"
                placeholder={i18n.t("settings.default.system")}
                bind:value={draft.browser_path}
                oninput={(e) => onPathInput("browser_path", (e.currentTarget).value)}
              />
              <span
                class="path-dot"
                class:path-ok={pathChecks.browser_path === true}
                class:path-bad={pathChecks.browser_path === false}
                title={pathChecks.browser_path === false
                  ? i18n.t("settings.system.path_missing")
                  : i18n.t("settings.system.path_ok")}
              ></span>
              <Button
                variant="secondary"
                size="sm"
                icon="folder"
                onclick={() => browseFor("browser_path")}
              >
                {i18n.t("settings.system.browse")}
              </Button>
            </div>
          </div>
          <div class="field field-stack">
            <label class="field-label" for="sp-terminal">{i18n.t("settings.system.terminal")}</label>
            <div class="path-row">
              <input
                id="sp-terminal"
                type="text"
                placeholder={i18n.t("settings.default.system")}
                bind:value={draft.terminal}
                oninput={(e) => onPathInput("terminal", (e.currentTarget).value)}
              />
              <span
                class="path-dot"
                class:path-ok={pathChecks.terminal === true}
                class:path-bad={pathChecks.terminal === false}
                title={pathChecks.terminal === false
                  ? i18n.t("settings.system.path_missing")
                  : i18n.t("settings.system.path_ok")}
              ></span>
              <Button
                variant="secondary"
                size="sm"
                icon="folder"
                onclick={() => browseFor("terminal")}
              >
                {i18n.t("settings.system.browse")}
              </Button>
            </div>
          </div>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.about.data_dir")}</span>
              <span class="sp-hint field-desc">
                <code class="data-dir">{dataDir || "вЂ¦"}</code>
              </span>
            </div>
            <div class="field-ctrl">
              <Button variant="secondary" size="sm" icon="folder" onclick={openDataDir}>
                {i18n.t("settings.system.open_data_dir")}
              </Button>
            </div>
          </div>
        </Card>
      </div>
    {:else if activeTab === "display"}
      <div class="sp-tab-panel">
        <Card title={i18n.t("settings.section.appearance")}>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.appearance.theme")}</span>
            </div>
            <div class="field-ctrl">
              <select bind:value={draft.theme} onchange={onAnyChange}>
                <option value="system">{i18n.t("settings.theme.system")}</option>
                <option value="light">{i18n.t("settings.theme.light")}</option>
                <option value="dark">{i18n.t("settings.theme.dark")}</option>
              </select>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.appearance.language")}</span>
            </div>
            <div class="field-ctrl">
              <select bind:value={draft.language} onchange={onLanguageChange}>
                {#each availableLocales as loc}
                  <option value={loc.id}>
                    {loc.icon} {loc.name} ({loc.index})
                  </option>
                {/each}
              </select>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.display.font_size")}</span>
            </div>
            <div class="field-ctrl">
              <div class="segmented">
                {#each ["sm", "md", "lg"] as size}
                  <button
                    type="button"
                    class="segmented-btn"
                    class:segmented-active={draft.font_size === size}
                    onclick={() => setFontSize(size)}
                  >
                    {i18n.t(`settings.display.font_size.${size}`)}
                  </button>
                {/each}
              </div>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.display.reduced_motion")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.display.reduced_motion_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.reduced_motion}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.display.hints")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.display.hints_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.show_interface_hints}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>

          <div class="field field-accent">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.display.accent")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.display.accent_desc")}</span>
              <code class="accent-hex">{draft.accent_color}</code>
            </div>
            <div class="field-ctrl">
              <div class="swatches">
                {#each Object.entries(ACCENT_PRESETS) as [key, hex]}
                  <button
                    type="button"
                    class="swatch"
                    class:swatch-active={draft.accent_color === key}
                    style={`--sw: ${hex}`}
                    title={i18n.t(`settings.display.accent.preset.${key}`)}
                    onclick={() => setAccent(key)}
                    aria-label={i18n.t(`settings.display.accent.preset.${key}`)}
                  ></button>
                {/each}
                <label
                  class="swatch swatch-custom"
                  class:swatch-active={/^#/.test(draft.accent_color)}
                  title={i18n.t("settings.display.accent.custom")}
                >
                  <input
                    type="color"
                    value={customHex}
                    oninput={(e) => {
                      customHex = (e.currentTarget).value;
                      setAccent(customHex);
                    }}
                  />
                </label>
              </div>
            </div>
          </div>
        </Card>
      </div>
    {:else if activeTab === "behavior"}
      <div class="sp-tab-panel">
        <Card title={i18n.t("settings.section.behavior")}>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.behavior.autosave")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.behavior.autosave_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.auto_save_profiles}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.behavior.restore_route")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.behavior.restore_route_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.restore_last_route}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.behavior.confirm_reset")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.behavior.confirm_reset_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.confirm_before_reset}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>
        </Card>

        <Card
          variant="outline"
          title={i18n.t("settings.danger.title")}
          description={i18n.t("settings.danger.desc")}
          class="danger-card"
        >
          <Button variant="danger" icon="trash" loading={saving} onclick={factoryReset}>
            {i18n.t("settings.danger.reset")}
          </Button>
        </Card>
      </div>
    {:else if activeTab === "apps"}
      <div class="sp-tab-panel">
        <Card
          title={i18n.t("settings.section.apps")}
          description={i18n.t("settings.apps.hint")}
        >
          {#if draft.preferred_apps.length === 0}
            <p class="sp-hint apps-empty">{i18n.t("settings.apps.empty")}</p>
          {:else}
            {#each draft.preferred_apps as app, i}
              <div class="app-row">
                <input
                  type="text"
                  placeholder={i18n.t("settings.apps.name")}
                  value={app.name}
                  oninput={(e) => updateApp(i, "name", (e.currentTarget).value)}
                />
                <input
                  type="text"
                  placeholder={i18n.t("settings.apps.path")}
                  value={app.path}
                  oninput={(e) => updateApp(i, "path", (e.currentTarget).value)}
                />
                <input
                  type="text"
                  placeholder={i18n.t("settings.apps.args")}
                  value={app.args ?? ""}
                  oninput={(e) => updateApp(i, "args", (e.currentTarget).value || null)}
                />
                <IconButton
                  icon="trash"
                  variant="danger"
                  size="sm"
                  label={i18n.t("settings.apps.remove")}
                  onclick={() => removeApp(i)}
                />
              </div>
            {/each}
          {/if}
          <Button variant="secondary" size="sm" icon="plus" onclick={addApp}>
            {i18n.t("settings.apps.add")}
          </Button>
        </Card>
      </div>
    {:else if activeTab === "ai"}
      <div class="sp-tab-panel">
        <div class="ai-banner" role="alert">
          <span class="ai-banner-icon" aria-hidden="true">
            <Icon name="alert" size={22} />
          </span>
          <div class="ai-banner-text">
            <strong>{i18n.t("settings.ai.banner_title")}</strong>
            <span>{i18n.t("settings.ai.banner_desc")}</span>
          </div>
        </div>

        <Card
          title={i18n.t("settings.ai.connection_title")}
          description={i18n.t("settings.ai.connection_desc")}
        >
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.enabled")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.ai.enabled_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input type="checkbox" bind:checked={draft.ai.enabled} onchange={onAnyChange} />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.provider")}</span>
            </div>
            <div class="field-ctrl">
              <select bind:value={draft.ai.provider} onchange={onProviderChange}>
                <option value="openai">{i18n.t("settings.ai.provider.openai")}</option>
                <option value="anthropic">{i18n.t("settings.ai.provider.anthropic")}</option>
                <option value="ollama">{i18n.t("settings.ai.provider.ollama")}</option>
                <option value="custom">{i18n.t("settings.ai.provider.custom")}</option>
              </select>
            </div>
          </div>

          <div class="field field-stack">
            <label class="field-label" for="sp-ai-url">{i18n.t("settings.ai.base_url")}</label>
            <input
              id="sp-ai-url"
              type="text"
              placeholder={i18n.t("settings.ai.base_url_ph")}
              bind:value={draft.ai.base_url}
              oninput={onAnyChange}
            />
            {#if urlInvalid}
              <span class="warn-hint">{i18n.t("settings.ai.base_url_invalid")}</span>
            {/if}
          </div>

          <div class="field field-stack">
            <label class="field-label" for="sp-ai-key">{i18n.t("settings.ai.api_key")}</label>
            <div class="key-row">
              <input
                id="sp-ai-key"
                type={keyVisible ? "text" : "password"}
                placeholder={i18n.t("settings.ai.api_key_ph")}
                autocomplete="off"
                bind:value={draft.ai.api_key}
                oninput={onAnyChange}
              />
              <Button
                variant="subtle"
                size="sm"
                onclick={() => (keyVisible = !keyVisible)}
                label={keyVisible ? i18n.t("settings.ai.key_hide") : i18n.t("settings.ai.key_show")}
              >
                {keyVisible ? i18n.t("settings.ai.key_hide") : i18n.t("settings.ai.key_show")}
              </Button>
            </div>
            {#if keyShort}
              <span class="warn-hint">{i18n.t("settings.ai.key_short")}</span>
            {/if}
          </div>

          <div class="field field-stack">
            <label class="field-label" for="sp-ai-model">{i18n.t("settings.ai.model")}</label>
            <input
              id="sp-ai-model"
              type="text"
              placeholder={i18n.t("settings.ai.model_ph")}
              bind:value={draft.ai.model}
              oninput={onAnyChange}
            />
          </div>
        </Card>

        <Card title={i18n.t("settings.ai.model_params")}>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.temperature")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.ai.temperature_desc")}</span>
            </div>
            <div class="field-ctrl field-slider">
              <input
                type="range"
                min="0"
                max="2"
                step="0.1"
                bind:value={draft.ai.temperature}
                oninput={onAnyChange}
              />
              <code class="slider-value">{draft.ai.temperature.toFixed(1)}</code>
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.max_tokens")}</span>
            </div>
            <div class="field-ctrl">
              <input
                type="number"
                min="1"
                max="128000"
                step="1"
                bind:value={draft.ai.max_tokens}
                oninput={onAnyChange}
              />
            </div>
          </div>

          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.timeout")}</span>
            </div>
            <div class="field-ctrl">
              <input
                type="number"
                min="5"
                max="600"
                step="1"
                bind:value={draft.ai.timeout_secs}
                oninput={onAnyChange}
              />
            </div>
          </div>

          <div class="field field-stack">
            <label class="field-label" for="sp-ai-prompt">{i18n.t("settings.ai.system_prompt")}</label>
            <textarea
              id="sp-ai-prompt"
              rows="4"
              placeholder={i18n.t("settings.ai.system_prompt_ph")}
              bind:value={draft.ai.system_prompt}
              oninput={onAnyChange}
            ></textarea>
            <span class="sp-hint">{i18n.t("settings.ai.system_prompt_desc")}</span>
          </div>
        </Card>

        <Card title={i18n.t("settings.ai.context_title")}>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.ai.page_context")}</span>
              <span class="sp-hint field-desc">{i18n.t("settings.ai.page_context_desc")}</span>
            </div>
            <div class="field-ctrl">
              <label class="switch">
                <input
                  type="checkbox"
                  bind:checked={draft.ai.page_context}
                  onchange={onAnyChange}
                />
                <span class="switch-track"><span class="switch-knob"></span></span>
              </label>
            </div>
          </div>
        </Card>

        <Card
          title={i18n.t("settings.ai.preview_title")}
          description={i18n.t("settings.ai.preview_desc")}
        >
          <pre class="ai-preview">{connectionPreview()}</pre>
        </Card>
      </div>
    {:else if activeTab === "about"}
      <div class="sp-tab-panel">
        <Card title={i18n.t("settings.about.title")} description={i18n.t("settings.about.desc")}>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.about.version")}</span>
            </div>
            <div class="field-ctrl">
              <Badge tone="violet" dot>{APP_NAME} {APP_VERSION}</Badge>
            </div>
          </div>
          <div class="field">
            <div class="field-text">
              <span class="field-label">{i18n.t("settings.about.stack")}</span>
            </div>
            <div class="field-ctrl">
              <span class="about-value">{i18n.t("settings.about.stack_value")}</span>
            </div>
          </div>
        </Card>

        <Card
          title={i18n.t("settings.about.theme_preview")}
          description={i18n.t("settings.about.theme_preview_desc")}
        >
          <div class="theme-previews">
            <div class="tp tp-dark">
              <span class="tp-title">Dark</span>
              <span class="tp-btn"></span>
              <span class="tp-line"></span>
              <span class="tp-line tp-line-short"></span>
            </div>
            <div class="tp tp-light">
              <span class="tp-title">Light</span>
              <span class="tp-btn"></span>
              <span class="tp-line"></span>
              <span class="tp-line tp-line-short"></span>
            </div>
          </div>
        </Card>

        <Card
          title={i18n.t("settings.about.author")}
          description={i18n.t("settings.about.author_desc")}
        >
          <div class="author-links">
            <a
              class="author-link"
              href="https://github.com/cheburek4535"
              target="_blank"
              rel="noreferrer"
            >
              <Icon name="external" size={15} />
              github.com/cheburek4535
            </a>
            <a
              class="author-link"
              href="https://github.com/cheburek4535/StackPilot"
              target="_blank"
              rel="noreferrer"
            >
              <Icon name="external" size={15} />
              {i18n.t("settings.about.project")}
            </a>
          </div>
        </Card>
      </div>
    {/if}
  {/if}
</PageContainer>

<style>
  .sp-sticky {
    position: sticky;
    top: 0;
    z-index: 30;
    padding: var(--sp-2) 0 var(--sp-3);
    margin-bottom: var(--sp-5);
    background: linear-gradient(
      var(--sp-bg-0) 70%,
      color-mix(in srgb, var(--sp-bg-0) 88%, transparent)
    );
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
  }

  .sp-minitabs {
    display: flex;
    gap: var(--sp-1);
    padding: var(--sp-1);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    overflow-x: auto;
    scrollbar-width: none;
  }

  .sp-minitab {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    border: none;
    border-radius: var(--sp-radius-md);
    background: transparent;
    color: var(--sp-text-2);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    white-space: nowrap;
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      color 0.15s ease,
      box-shadow 0.15s ease;
  }

  .sp-minitab:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-minitab-active {
    background: linear-gradient(135deg, var(--sp-accent-soft), rgba(34, 211, 238, 0.06));
    color: var(--sp-accent);
    box-shadow: inset 0 0 0 1px var(--sp-accent-border);
  }

  .sp-minitab-active:hover {
    color: var(--sp-accent);
  }

  .autosave-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    margin-top: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
  }

  .autosave-text {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .autosave-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.25rem;
    height: 2.25rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-md);
    color: var(--sp-accent);
    background: var(--sp-accent-soft);
    border: 1px solid var(--sp-accent-border);
  }

  .autosave-heading {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: var(--sp-fs-sm);
  }

  .autosave-desc {
    max-width: 46rem;
  }

  .autosave-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
  }

  .sp-tab-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-5);
    animation: sp-rise-in 0.18s ease;
  }

  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    padding: var(--sp-3) 0;
  }

  .field + .field {
    border-top: 1px solid var(--sp-border-faint);
  }

  .field-stack {
    flex-direction: column;
    align-items: stretch;
    gap: var(--sp-2);
  }

  .field-text {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 0;
  }

  .field-label {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .field-desc {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    max-width: 34rem;
  }

  .field-ctrl {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    flex: 0 0 auto;
    max-width: 55%;
  }

  .field-slider {
    gap: var(--sp-3);
  }

  .field-slider input[type="range"] {
    width: 10rem;
  }

  .slider-value {
    min-width: 2.25rem;
    text-align: right;
  }

  input[type="text"],
  input[type="email"],
  input[type="number"],
  select,
  textarea {
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
    transition:
      border-color 0.15s ease,
      box-shadow 0.15s ease;
  }

  input[type="text"]:focus,
  input[type="email"]:focus,
  input[type="number"]:focus,
  select:focus,
  textarea:focus {
    outline: none;
    border-color: var(--sp-accent);
    box-shadow: 0 0 0 3px var(--sp-accent-soft);
  }

  textarea {
    resize: vertical;
    line-height: var(--sp-lh-normal);
  }

  .field-ctrl select,
  .field-ctrl input[type="number"] {
    width: auto;
    min-width: 9rem;
  }

  .key-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .key-row input {
    flex: 1;
  }

  .segmented {
    display: inline-flex;
    gap: var(--sp-1);
    padding: var(--sp-1);
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .segmented-btn {
    padding: var(--sp-1) var(--sp-3);
    border: none;
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-2);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      color 0.15s ease;
  }

  .segmented-btn:hover {
    color: var(--sp-text-1);
  }

  .segmented-active {
    background: var(--sp-bg-3);
    color: var(--sp-text-1);
    box-shadow: var(--sp-shadow-1);
  }

  .switch {
    position: relative;
    display: inline-flex;
    flex: 0 0 auto;
    cursor: pointer;
  }

  .switch input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .switch-track {
    display: inline-flex;
    align-items: center;
    width: 2.5rem;
    height: 1.375rem;
    padding: 0.125rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-bg-3);
    border: 1px solid var(--sp-border-strong);
    transition:
      background-color 0.18s ease,
      border-color 0.18s ease;
  }

  .switch-knob {
    width: 1rem;
    height: 1rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-text-2);
    transition:
      transform 0.18s ease,
      background-color 0.18s ease;
  }

  .switch input:checked + .switch-track {
    background: var(--sp-accent-strong);
    border-color: var(--sp-accent-border);
  }

  .switch input:checked + .switch-track .switch-knob {
    transform: translateX(1.125rem);
    background: #fff;
  }

  .switch input:focus-visible + .switch-track {
    box-shadow: var(--sp-focus-ring);
  }

  .swatches {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .swatch {
    width: 1.75rem;
    height: 1.75rem;
    padding: 0;
    border-radius: var(--sp-radius-full);
    border: 2px solid var(--sp-bg-1);
    background: var(--sw, var(--sp-accent));
    cursor: pointer;
    box-shadow: 0 0 0 1px var(--sp-border-strong);
    transition:
      transform 0.12s ease,
      box-shadow 0.12s ease;
  }

  .swatch:hover {
    transform: scale(1.12);
  }

  .swatch-active {
    box-shadow: 0 0 0 2px var(--sp-bg-0), 0 0 0 4px var(--sp-accent);
  }

  .swatch-custom {
    position: relative;
    overflow: hidden;
  }

  .swatch-custom input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .accent-hex {
    font-size: var(--sp-fs-xs);
  }

  .identity {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) 0 var(--sp-4);
  }

  .identity-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3rem;
    height: 3rem;
    border-radius: var(--sp-radius-lg);
    font-weight: var(--sp-fw-bold);
    font-size: var(--sp-fs-lg);
    color: var(--sp-accent);
    background: linear-gradient(135deg, var(--sp-accent-soft), rgba(34, 211, 238, 0.08));
    border: 1px solid var(--sp-accent-border);
  }

  .identity-text {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 0;
  }

  .apps-empty {
    margin: 0;
    padding: var(--sp-2) 0;
  }

  .path-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .path-row input {
    flex: 1;
    min-width: 0;
  }

  .path-dot {
    width: 0.6rem;
    height: 0.6rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-full);
    background: var(--sp-text-3);
    opacity: 0.5;
  }

  .path-dot.path-ok {
    background: var(--sp-success);
    box-shadow: 0 0 8px var(--sp-success);
    opacity: 1;
  }

  .path-dot.path-bad {
    background: var(--sp-danger);
    box-shadow: 0 0 8px var(--sp-danger);
    opacity: 1;
  }

  .data-dir {
    font-size: var(--sp-fs-xs);
  }

  .warn-hint {
    font-size: var(--sp-fs-xs);
    color: var(--sp-amber);
  }

  .about-value {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    text-align: right;
  }

  .theme-previews {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: var(--sp-4);
  }

  .tp {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: var(--sp-4);
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
  }

  .tp-dark {
    background: #0a0b0f;
  }

  .tp-light {
    background: #f4f6fa;
  }

  .tp-title {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-2);
  }

  .tp-btn {
    align-self: flex-start;
    width: 3.5rem;
    height: 1.125rem;
    border-radius: var(--sp-radius-sm);
    background: var(--sp-accent-strong);
  }

  .tp-line {
    height: 0.375rem;
    border-radius: var(--sp-radius-xs);
    background: var(--sp-bg-3);
  }

  .tp-light .tp-line {
    background: #e6e9f1;
  }

  .tp-line-short {
    width: 60%;
  }

  .author-links {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .author-link {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    width: max-content;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
  }

  .app-row {
    display: grid;
    grid-template-columns: 1.2fr 1.6fr 1fr auto;
    gap: var(--sp-2);
    align-items: center;
    padding: var(--sp-1) 0;
  }

  .app-row + .app-row {
    border-top: 1px solid var(--sp-border-faint);
  }

  .app-row input {
    min-width: 0;
  }

  .danger-card {
    border-color: rgba(248, 113, 113, 0.35);
  }

  .ai-banner {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-4);
    padding: var(--sp-5) var(--sp-6);
    border-radius: var(--sp-radius-lg);
    background: linear-gradient(135deg, rgba(251, 191, 36, 0.1), rgba(248, 113, 113, 0.1));
    border: 1px solid rgba(251, 191, 36, 0.4);
    box-shadow: var(--sp-shadow-1);
    animation: sp-rise-in 0.2s ease;
  }

  .ai-banner-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2.5rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-md);
    color: var(--sp-amber);
    background: rgba(251, 191, 36, 0.14);
    border: 1px solid rgba(251, 191, 36, 0.35);
  }

  .ai-banner-text {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  .ai-banner-text strong {
    color: var(--sp-text-1);
    font-size: var(--sp-fs-md);
  }

  .ai-preview {
    margin: 0;
    padding: var(--sp-3);
    border-radius: var(--sp-radius-md);
    background: var(--sp-code-bg);
    border: 1px solid var(--sp-border);
    font-size: var(--sp-fs-xs);
    line-height: var(--sp-lh-normal);
    overflow-x: auto;
  }

  @media (max-width: 720px) {
    .autosave-bar {
      flex-direction: column;
      align-items: stretch;
    }

    .autosave-actions {
      justify-content: flex-end;
    }

    .field {
      flex-direction: column;
      align-items: stretch;
    }

    .field-ctrl {
      justify-content: flex-start;
      max-width: none;
    }

    .app-row {
      grid-template-columns: 1fr;
    }
  }
</style>