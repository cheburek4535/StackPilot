<script lang="ts">
  import TechIcon from "$lib/components/TechIcon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import type { WizardTreeData, ProjectPreset } from "$lib/modules/project_creator/types";

  let {
    tree,
    onapply,
  }: {
    tree: WizardTreeData | null;
    onapply: (p: ProjectPreset) => void;
  } = $props();
</script>

<div class="presets-container">
  <div class="presets-header">
    <h2 class="prompt">{i18n.t("create.templates_title") as TranslationKey}</h2>
    <p class="hint">{i18n.t("create.templates_desc") as TranslationKey}</p>
  </div>

  <div class="preset-grid">
    {#each tree?.presets ?? [] as p}
      <div class="preset-card">
        <TechIcon icon={p.icon} alt={i18n.t(p.label as TranslationKey)} size="xl" />
        <h3 class="preset-title">{i18n.t(p.label as TranslationKey)}</h3>
        <p class="preset-desc">{i18n.t(p.description as TranslationKey)}</p>
        <div class="preset-stack">
          {#if p.stack.backend_lang}<span class="preset-chip">{p.stack.backend_lang}</span>{/if}
          {#if p.stack.frontend_lang}<span class="preset-chip">{p.stack.frontend_lang}</span>{/if}
          {#each p.stack.frameworks as fw}<span class="preset-chip">{fw}</span>{/each}
          {#if p.stack.tools.length > 0}
            <span class="preset-chip">{i18n.t("create.tools_count", { n: p.stack.tools.length }) as TranslationKey}</span>
          {/if}
        </div>
        <div class="preset-apply-wrap">
          <Button
            variant="primary"
            size="sm"
            block
            icon="sparkles"
            onclick={() => onapply(p)}
          >
            {i18n.t("create.use_template") as TranslationKey}
          </Button>
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .presets-container {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }

  .presets-header {
    text-align: center;
    justify-self: center;
    width: 100%;
    max-width: 580px;
    margin: 0 auto 1.5rem;
    padding: 0.85rem 1.5rem 1rem;
    background: var(--sp-glass-bg);
    backdrop-filter: blur(12px) saturate(120%);
    -webkit-backdrop-filter: blur(12px) saturate(120%);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--sp-radius-xl);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  }

  .prompt {
    margin: 0;
    font-size: 1.15rem;
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    letter-spacing: -0.015em;
    line-height: 1.3;
  }

  .hint {
    margin: 0.25rem 0 0;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
    line-height: 1.4;
  }

  .preset-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 1rem;
  }

  .preset-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.65rem;
    padding: 1.25rem;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: var(--sp-radius-xl);
    background: var(--sp-glass-bg);
    backdrop-filter: blur(12px) saturate(120%);
    -webkit-backdrop-filter: blur(12px) saturate(120%);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
    text-align: center;
    color: var(--sp-text-1);
    transition: all 0.25s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .preset-card:hover {
    border-color: rgba(255, 255, 255, 0.2);
    background: rgba(255, 255, 255, 0.05);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    transform: translateY(-2px);
  }

  .preset-title {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .preset-desc {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    margin: 0;
    min-height: 2.4em;
    line-height: var(--sp-lh-normal);
  }

  .preset-stack {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    justify-content: center;
    min-height: 2rem;
    align-items: center;
  }

  .preset-chip {
    font-size: var(--sp-fs-xs);
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    border: 1px solid var(--sp-border);
    padding: 0.15rem 0.55rem;
    border-radius: var(--sp-radius-full);
  }

  .preset-apply-wrap {
    width: 100%;
    margin-top: 0.5rem;
  }
</style>