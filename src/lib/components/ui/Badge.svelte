<script lang="ts">
  import type { Snippet } from "svelte";

  export type BadgeTone =
    | "neutral"
    | "violet"
    | "cyan"
    | "blue"
    | "lime"
    | "amber"
    | "red";

  let {
    tone = "neutral",
    dot = false,
    children,
  }: {
    tone?: BadgeTone;
    dot?: boolean;
    children?: Snippet;
  } = $props();
</script>

<span class="sp-badge sp-badge-{tone}" role="status">
  {#if dot}
    <span class="sp-badge-dot" aria-hidden="true"></span>
  {/if}
  {#if children}
    <span class="sp-badge-label">{@render children()}</span>
  {/if}
</span>

<style>
  .sp-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0.0625rem 0.4375rem;
    border-radius: var(--sp-radius-sm);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    line-height: 1.5;
    letter-spacing: 0.01em;
    white-space: nowrap;
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  }

  .sp-badge-dot {
    width: 0.3125rem;
    height: 0.3125rem;
    border-radius: var(--sp-radius-full);
    background: currentColor;
  }

  .sp-badge-neutral {
    background: rgba(148, 163, 184, 0.06);
    border: 1px solid rgba(148, 163, 184, 0.1);
    color: var(--sp-text-2);
  }

  .sp-badge-violet {
    background: rgba(160, 139, 232, 0.07);
    border: 1px solid rgba(160, 139, 232, 0.15);
    color: var(--sp-violet);
  }

  .sp-badge-cyan {
    background: var(--sp-info-soft);
    border: 1px solid var(--sp-info-soft);
    color: var(--sp-cyan);
  }

  .sp-badge-blue {
    background: rgba(111, 158, 232, 0.07);
    border: 1px solid rgba(111, 158, 232, 0.15);
    color: var(--sp-blue);
  }

  .sp-badge-lime {
    background: var(--sp-success-soft);
    border: 1px solid var(--sp-success-soft);
    color: var(--sp-lime);
  }

  .sp-badge-amber {
    background: var(--sp-warning-soft);
    border: 1px solid var(--sp-warning-border);
    color: var(--sp-amber);
  }

  .sp-badge-red {
    background: var(--sp-danger-soft);
    border: 1px solid rgba(239, 68, 68, 0.15);
    color: var(--sp-red);
  }
</style>