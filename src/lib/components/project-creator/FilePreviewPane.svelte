<script lang="ts">
  import type { FileEntry } from "$lib/modules/project_creator/types";
  import CodeEditor from "$lib/components/CodeEditor.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    file,
    onclose,
  }: {
    file: FileEntry | null;
    onclose: () => void;
  } = $props();

  function langFromPath(path: string): string {
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    const map: Record<string, string> = {
      ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript",
      py: "python", rs: "rust", go: "go",
      json: "json", yaml: "yaml", yml: "yaml",
      html: "html", htm: "html", css: "css",
      svelte: "javascript", vue: "javascript",
      toml: "json", sh: "plaintext", md: "plaintext",
      dockerfile: "plaintext", gitignore: "plaintext",
    };
    return map[ext] ?? "plaintext";
  }

  function certaintyLabel(c: string): string {
    switch (c) {
      case "certain": return i18n.t("create.preview.file_certain") as TranslationKey;
      case "expected": return i18n.t("create.preview.file_expected") as TranslationKey;
      case "unknown": return i18n.t("create.preview.file_unknown") as TranslationKey;
      default: return c;
    }
  }

  function certaintyClass(c: string): string {
    switch (c) {
      case "certain": return "fpb-certain";
      case "expected": return "fpb-expected";
      case "unknown": return "fpb-unknown";
      default: return "";
    }
  }
</script>

{#if file}
  <div class="fp-pane">
    <header class="fp-header">
      <div class="fp-title-row">
        <Icon name="file" size={16} />
        <span class="fp-filename">{file.name}</span>
        <span class="fp-path">{file.path}</span>
      </div>
      <div class="fp-meta">
        <span class="fp-badge {certaintyClass(file.certainty)}">
          {certaintyLabel(file.certainty)}
        </span>
        <span class="fp-source">{file.source}</span>
        <button class="fp-close" onclick={onclose} title={i18n.t("create.preview.close") as TranslationKey}>
          <Icon name="x" size={14} />
        </button>
      </div>
    </header>
    {#if file.warning}
      <div class="fp-warning">
        <Icon name="alert" size={14} />
        <span>{file.warning}</span>
      </div>
    {/if}
    <div class="fp-editor">
      {#if file.content}
        <CodeEditor value={file.content} language={langFromPath(file.path)} readonly={true} />
      {:else}
        <div class="fp-empty">
          <Icon name="info" size={20} />
          <p>{i18n.t("create.preview.no_content") as TranslationKey}</p>
          <p class="fp-empty-hint">{i18n.t("create.preview.no_content_hint") as TranslationKey}</p>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .fp-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .fp-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    border-bottom: 1px solid var(--sp-border-faint);
    flex-shrink: 0;
  }

  .fp-title-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .fp-filename {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .fp-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fp-meta {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-shrink: 0;
  }

  .fp-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0.1rem var(--sp-2);
    border-radius: var(--sp-radius-full);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
  }

  .fpb-certain {
    background: rgba(163, 230, 53, 0.14);
    color: var(--sp-success);
    border: 1px solid rgba(163, 230, 53, 0.3);
  }

  .fpb-expected {
    background: rgba(251, 191, 36, 0.14);
    color: var(--sp-warning);
    border: 1px solid rgba(251, 191, 36, 0.3);
  }

  .fpb-unknown {
    background: var(--sp-bg-2);
    color: var(--sp-text-3);
    border: 1px solid var(--sp-border);
  }

  .fp-source {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .fp-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: none;
    color: var(--sp-text-3);
    cursor: pointer;
    border-radius: var(--sp-radius-sm);
    transition: background 0.1s, color 0.1s;
  }

  .fp-close:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .fp-warning {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-4);
    background: rgba(251, 191, 36, 0.08);
    color: var(--sp-warning);
    font-size: var(--sp-fs-xs);
    border-bottom: 1px solid rgba(251, 191, 36, 0.15);
    flex-shrink: 0;
  }

  .fp-editor {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .fp-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: var(--sp-3);
    color: var(--sp-text-3);
    text-align: center;
    padding: var(--sp-8);
  }

  .fp-empty p {
    margin: 0;
    font-size: var(--sp-fs-sm);
  }

  .fp-empty-hint {
    font-size: var(--sp-fs-xs) !important;
    color: var(--sp-text-3);
    max-width: 260px;
  }
</style>
