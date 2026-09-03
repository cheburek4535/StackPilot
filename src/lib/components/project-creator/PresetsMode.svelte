<script lang="ts">
  import TechIcon from "$lib/components/TechIcon.svelte";
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

<p class="prompt">{i18n.t("create.templates_title") as TranslationKey}</p>
<p class="hint">{i18n.t("create.templates_desc") as TranslationKey}</p>
<div class="preset-grid">
  {#each tree!.presets as p}
    <div class="preset-card">
      <TechIcon icon={p.icon} alt={i18n.t(p.label as TranslationKey)} size="xl" />
      <h3>{i18n.t(p.label as TranslationKey)}</h3>
      <p class="preset-desc">{i18n.t(p.description as TranslationKey)}</p>
      <div class="preset-stack">
        {#if p.stack.backend_lang}<span class="preset-chip">{p.stack.backend_lang}</span>{/if}
        {#if p.stack.frontend_lang}<span class="preset-chip">{p.stack.frontend_lang}</span>{/if}
        {#each p.stack.frameworks as fw}<span class="preset-chip">{fw}</span>{/each}
        {#if p.stack.tools.length > 0}
          <span class="preset-chip">{i18n.t("create.tools_count", { n: p.stack.tools.length }) as TranslationKey}</span>
        {/if}
      </div>
      <button class="btn-primary preset-apply" onclick={() => onapply(p)}>{i18n.t("create.use_template") as TranslationKey}</button>
    </div>
  {/each}
</div>

<style>
  .prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
  .hint { color: var(--sp-text-3); margin-bottom: 1.5rem; font-size: 0.95rem; }
  .preset-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 0.9rem; }
  .preset-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    padding: 1.1rem;
    border: 1px solid var(--sp-border);
    border-radius: 12px;
    background: var(--sp-bg-1);
    text-align: center;
    color: var(--sp-text-1);
    transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
  }
  .preset-card:hover { border-color: var(--sp-accent-border); background: var(--sp-bg-2); }
  .preset-card h3 { margin: 0; font-size: 1rem; }
  .preset-desc { font-size: 0.8rem; color: var(--sp-text-3); margin: 0; }
  .preset-stack { display: flex; flex-wrap: wrap; gap: 0.3rem; justify-content: center; min-height: 1.4rem; }
  .preset-chip {
    font-size: 0.72rem;
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    border: 1px solid var(--sp-border);
    padding: 0.15rem 0.5rem;
    border-radius: 999px;
  }
  .preset-apply { width: 100%; }
  .btn-primary { background: var(--sp-accent-strong); color: #fff; padding: 0.6rem 1.5rem; border-radius: 8px; border: none; cursor: pointer; font-weight: 600; font-size: 0.95rem; }
</style>