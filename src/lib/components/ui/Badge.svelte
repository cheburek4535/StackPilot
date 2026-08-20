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
    padding: 0.125rem var(--sp-2);
    border-radius: var(--sp-radius-full);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    line-height: 1.4;
    letter-spacing: 0.01em;
    white-space: nowrap;
    border: 1px solid transparent;
  }

  .sp-badge-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: var(--sp-radius-full);
    background: currentColor;
  }

  .sp-badge-neutral {
    background: var(--sp-bg-2);
    color: var(--sp-text-2);
    border-color: var(--sp-border);
  }

  .sp-badge-violet {
    background: rgba(139, 92, 246, 0.14);
    color: var(--sp-violet);
    border-color: rgba(139, 92, 246, 0.3);
  }

  .sp-badge-cyan {
    background: rgba(34, 211, 238, 0.12);
    color: var(--sp-cyan);
    border-color: rgba(34, 211, 238, 0.3);
  }

  .sp-badge-blue {
    background: rgba(96, 165, 250, 0.12);
    color: var(--sp-blue);
    border-color: rgba(96, 165, 250, 0.3);
  }

  .sp-badge-lime {
    background: rgba(163, 230, 53, 0.12);
    color: var(--sp-lime);
    border-color: rgba(163, 230, 53, 0.3);
  }

  .sp-badge-amber {
    background: rgba(251, 191, 36, 0.12);
    color: var(--sp-amber);
    border-color: rgba(251, 191, 36, 0.3);
  }

  .sp-badge-red {
    background: rgba(248, 113, 113, 0.12);
    color: var(--sp-red);
    border-color: rgba(248, 113, 113, 0.3);
  }
</style>