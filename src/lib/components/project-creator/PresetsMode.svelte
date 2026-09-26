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
    padding: 1.25rem 2rem;
    background: var(--sp-surface-grad), var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-xl);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
  }

  .prompt {
    margin: 0;
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    letter-spacing: -0.02em;
  }

  .hint {
    margin: var(--sp-1) 0 0;
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
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
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-xl);
    background: var(--sp-surface-grad), var(--sp-bg-1);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
    text-align: center;
    color: var(--sp-text-1);
    transition: all 0.2s ease;
  }

  .preset-card:hover {
    border-color: var(--sp-border-strong);
    background: var(--sp-surface-grad), var(--sp-bg-2);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-2);
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