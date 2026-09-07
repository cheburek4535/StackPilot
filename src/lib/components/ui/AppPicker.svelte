<script lang="ts">
  import type { DetectedApp } from "$lib/core/types";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    apps,
    value,
    onChange,
    title,
    description = "",
    autoLabel = i18n.t("settings.apps.auto") as TranslationKey,
    manualLabel = i18n.t("settings.apps.manual") as TranslationKey,
    notFoundLabel = i18n.t("settings.apps.not_found") as TranslationKey,
    placeholder = "",
  }: {
    apps: DetectedApp[];
    value: string;
    onChange: (value: string) => void;
    title: string;
    description?: string;
    autoLabel?: string;
    manualLabel?: string;
    notFoundLabel?: string;
    placeholder?: string;
  } = $props();

  const isCustom = $derived(
    value.trim() !== "" && !apps.some((a) => a.path === value.trim()),
  );
</script>

<div class="app-picker">
  <div class="app-picker-head">
    <span class="app-picker-title">{title}</span>
    {#if description}
      <span class="app-picker-desc">{description}</span>
    {/if}
  </div>

  <div class="app-options">
    <label class="app-option" class:selected={value.trim() === ""}>
      <input
        type="radio"
        name="app-pick"
        checked={value.trim() === ""}
        onchange={() => onChange("")}
      />
      <span class="app-option-name">{autoLabel}</span>
    </label>

    {#each apps as app}
      <label class="app-option" class:selected={value.trim() === app.path}>
        <input
          type="radio"
          name="app-pick"
          checked={value.trim() === app.path}
          onchange={() => onChange(app.path)}
        />
        <span class="app-option-name">{app.name}</span>
        <code class="app-option-path" title={app.path}>{app.path}</code>
      </label>
    {:else}
      <p class="app-not-found">{notFoundLabel}</p>
    {/each}

    <label class="app-option" class:selected={isCustom}>
      <input
        type="radio"
        name="app-pick"
        checked={isCustom}
        onchange={() => onChange(value || placeholder)}
      />
      <span class="app-option-name">{manualLabel}</span>
      {#if isCustom || value.trim() !== ""}
        <input
          class="app-custom-input"
          type="text"
          placeholder={placeholder}
          value={value}
          oninput={(e) => onChange((e.currentTarget as HTMLInputElement).value)}
        />
      {/if}
    </label>
  </div>
</div>

<style>
  .app-picker {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .app-picker-head {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .app-picker-title {
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
  }
  .app-picker-desc {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }
  .app-options {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .app-option {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    cursor: pointer;
    font-size: var(--sp-fs-xs);
  }
  .app-option.selected {
    border-color: var(--sp-accent-strong);
    background: var(--sp-accent-soft);
  }
  .app-option input[type="radio"] {
    accent-color: var(--sp-accent-strong);
    flex: 0 0 auto;
  }
  .app-option-name {
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-1);
    white-space: nowrap;
  }
  .app-option-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .app-custom-input {
    flex: 1;
    min-width: 140px;
    font-size: var(--sp-fs-xs);
    padding: 0.25rem 0.45rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-1);
    font-family: var(--sp-font-mono);
  }
  .app-not-found {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-style: italic;
  }
</style>