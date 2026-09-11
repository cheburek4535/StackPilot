<script lang="ts">
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import type { AnalysisReport } from "$lib/modules/project_creator/types";

  let {
    analyzing,
    analyzedPath,
    analysisError,
    analysisResult,
    onrun,
    onapply,
  }: {
    analyzing: boolean;
    analyzedPath: string | null;
    analysisError: string | null;
    analysisResult: AnalysisReport | null;
    onrun: () => void;
    onapply: () => void;
  } = $props();
</script>

<div class="analysis-panel">
  <p class="prompt">{i18n.t("create.analyze_title") as TranslationKey}</p>
  <p class="hint">{i18n.t("create.analyze_desc") as TranslationKey}</p>
  <button class="btn-primary" onclick={onrun} disabled={analyzing}>
    {analyzing ? (i18n.t("create.analyzing") as TranslationKey) : (i18n.t("create.select_folder") as TranslationKey)}
  </button>
  {#if analyzedPath}
    <p class="analyzed-path">{i18n.t("create.selected", { path: analyzedPath }) as TranslationKey}</p>
  {/if}
  {#if analysisError}
    <p class="error">{analysisError}</p>
  {/if}
  {#if analysisResult}
    <div class="analysis-result">
      <p class="analysis-summary">{analysisResult.summary}</p>
      <div class="analysis-section">
        <p class="section-title">{i18n.t("create.detected") as TranslationKey}</p>
        <div class="tech-tags">
          {#each analysisResult.detected_technologies as tech}
            <span class="tech-tag" class:certaion={tech.confidence === "Certain"}
                                  class:likely={tech.confidence === "Likely"}
                                  class:possible={tech.confidence === "Possible"}>
              {tech.name}{#if tech.version} ({tech.version}){/if}
            </span>
          {/each}
        </div>
      </div>
      <div class="analysis-section">
        <p class="section-title">{i18n.t("create.hints") as TranslationKey}</p>
        <div class="hint-tags">
          {#each analysisResult.project_type_hints as hint}
            <span class="hint-tag">{hint}</span>
          {/each}
          {#if analysisResult.has_docker}<span class="hint-tag docker">Docker</span>{/if}
          {#if analysisResult.has_git}<span class="hint-tag">Git</span>{/if}
          {#if analysisResult.has_ci}<span class="hint-tag">CI</span>{/if}
          {#if analysisResult.has_tests}<span class="hint-tag">Tests</span>{/if}
        </div>
      </div>
      <button class="btn-primary" onclick={onapply}>
        {i18n.t("create.use_detected") as TranslationKey}
      </button>
    </div>
  {/if}
</div>

<style>
  .prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; }
  .hint { color: var(--sp-text-3); margin-bottom: 1.5rem; font-size: 0.95rem; }
  .error { color: var(--sp-danger); }
  .analysis-panel { margin-bottom: 2rem; }
  .analysis-summary { font-size: 1rem; font-weight: 600; margin-bottom: 0.5rem; }
  .analysis-section { margin: 0.75rem 0; }
  .section-title { font-weight: 600; font-size: 0.9rem; color: var(--sp-text-2); margin-bottom: 0.3rem; }
  .tech-tags, .hint-tags { display: flex; flex-wrap: wrap; gap: 0.4rem; }
  .tech-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: var(--sp-accent-soft); color: var(--sp-text-2); }
  .tech-tag.certaion { background: var(--sp-success-soft); color: #fff; }
  .tech-tag.likely { background: var(--sp-accent-soft); }
  .tech-tag.possible { background: var(--sp-bg-2); }
  .hint-tag { padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; background: var(--sp-bg-2); color: var(--sp-text-3); }
  .hint-tag.docker { background: var(--sp-info-soft); color: var(--sp-info); }
  .analyzed-path { font-size: 0.85rem; color: var(--sp-accent-strong); margin-top: 0.3rem; }
  .btn-primary {
    background: var(--sp-accent-strong);
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--sp-accent-strong) 72%, black) 0%,
      var(--sp-accent-strong) 45%,
      var(--sp-accent) 100%
    );
    color: #fff;
    padding: 0.6rem 1.5rem;
    border-radius: var(--sp-radius-lg);
    border: 1px solid rgba(0, 0, 0, 0.35);
    cursor: pointer;
    font-weight: 600;
    font-size: 0.95rem;
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1), 0 2px 14px rgba(228, 87, 10, 0.2);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.55), 0 0 1px rgba(0, 0, 0, 0.4);
    transition: background 0.15s, box-shadow 0.15s;
  }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.06); }
</style>