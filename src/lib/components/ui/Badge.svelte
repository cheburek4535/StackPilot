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
    padding: 0.0625rem 0.375rem;
    border-radius: var(--sp-radius-sm);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    line-height: 1.5;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }

  .sp-badge-dot {
    width: 0.3125rem;
    height: 0.3125rem;
    border-radius: var(--sp-radius-full);
    background: currentColor;
  }

  .sp-badge-neutral {
    background: rgba(255, 255, 255, 0.08);
    color: var(--sp-text-2);
  }

  .sp-badge-violet {
    background: rgba(139, 92, 246, 0.12);
    color: var(--sp-violet);
  }

  .sp-badge-cyan {
    background: rgba(34, 211, 238, 0.12);
    color: var(--sp-cyan);
  }

  .sp-badge-blue {
    background: rgba(96, 165, 250, 0.12);
    color: var(--sp-blue);
  }

  .sp-badge-lime {
    background: rgba(163, 230, 53, 0.12);
    color: var(--sp-lime);
  }

  .sp-badge-amber {
    background: rgba(251, 191, 36, 0.12);
    color: var(--sp-amber);
  }

  .sp-badge-red {
    background: rgba(248, 113, 113, 0.12);
    color: var(--sp-red);
  }
</style>