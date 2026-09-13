<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import { i18n } from "$lib/core/i18n.svelte";
  import {
    dismissHelpHint,
    helpMode,
    helpProgress,
    isHintVisible,
    markHelpSaw,
    type HelpHintSpec,
  } from "$lib/core/help";

  export type HelpHintVariant = "info" | "tip" | "warning";

  let {
    id,
    resolvedBy = [],
    requiresSaw = [],
    variant = "tip",
    icon,
    title,
    text,
    dismissible = true,
    children,
  }: {
    /** Stable hint id (also the dismissal milestone). */
    id: string;
    /** Milestones that hide the hint once completed (auto mode only). */
    resolvedBy?: string[];
    /** Feature ids required before the hint can appear. */
    requiresSaw?: string[];
    variant?: HelpHintVariant;
    icon?: IconName;
    title?: string;
    text?: string;
    dismissible?: boolean;
    children?: Snippet;
  } = $props();

  const spec = $derived<HelpHintSpec>({ id, resolvedBy, requiresSaw });
  const visible = $derived(isHintVisible($helpProgress, $helpMode, spec));

  const fallbackIcon: IconName = $derived(
    variant === "warning" ? "alert" : variant === "info" ? "info" : "sparkles",
  );

  // Record that the feature was seen so dependent hints can require it.
  $effect(() => {
    if (visible) markHelpSaw(id);
  });
</script>

{#if visible}
  <aside class="sp-help sp-help-{variant}" role="note">
    <span class="sp-help-icon" aria-hidden="true">
      <Icon name={icon ?? fallbackIcon} size={16} />
    </span>
    <div class="sp-help-content">
      {#if title}
        <p class="sp-help-title">{title}</p>
      {/if}
      <div class="sp-help-text">
        {#if children}
          {@render children()}
        {:else if text}
          <p>{text}</p>
        {/if}
      </div>
    </div>
    {#if dismissible}
      <button
        type="button"
        class="sp-help-close"
        aria-label={i18n.t("help.hide")}
        title={i18n.t("help.hide")}
        onclick={() => dismissHelpHint(id)}
      >
        <Icon name="x" size={13} />
      </button>
    {/if}
  </aside>
{/if}

<style>
  .sp-help {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-3) var(--sp-3) var(--sp-4);
    margin: 0 0 var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-left: 3px solid var(--sp-accent);
    border-radius: var(--sp-radius-md);
    box-shadow: var(--sp-shadow-1);
    animation: sp-rise-in 0.18s ease;
  }

  .sp-help-info {
    border-left-color: var(--sp-info);
    background: linear-gradient(135deg, var(--sp-info-soft), var(--sp-bg-1) 60%);
  }

  .sp-help-tip {
    background: linear-gradient(135deg, var(--sp-accent-soft), var(--sp-bg-1) 60%);
  }

  .sp-help-warning {
    border-left-color: var(--sp-warning);
    background: linear-gradient(135deg, var(--sp-warning-soft), var(--sp-bg-1) 60%);
  }

  .sp-help-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-sm);
    color: var(--sp-accent);
    background: var(--sp-accent-soft);
  }

  .sp-help-info .sp-help-icon {
    color: var(--sp-info);
    background: var(--sp-info-soft);
  }

  .sp-help-warning .sp-help-icon {
    color: var(--sp-warning);
    background: var(--sp-warning-soft);
  }

  .sp-help-content {
    flex: 1 1 auto;
    min-width: 0;
  }

  .sp-help-title {
    margin: 0 0 var(--sp-1);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-help-text {
    font-size: var(--sp-fs-xs);
    line-height: var(--sp-lh-normal);
    color: var(--sp-text-2);
  }

  .sp-help-text :global(p) {
    margin: 0;
  }

  .sp-help-text :global(p + p) {
    margin-top: var(--sp-1);
  }

  .sp-help-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    flex: 0 0 auto;
    border: none;
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .sp-help-close:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }
</style>
