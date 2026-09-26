<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, tick } from "svelte";
  import { fade, scale } from "svelte/transition";
  import IconButton from "./IconButton.svelte";

  export type ModalSize = "sm" | "md" | "lg";

  let {
    open,
    onclose,
    title,
    description,
    size = "md",
    closeOnBackdrop = true,
    closeOnEscape = true,
    children,
    footer,
  }: {
    open: boolean;
    onclose: () => void;
    title: string;
    description?: string;
    size?: ModalSize;
    closeOnBackdrop?: boolean;
    closeOnEscape?: boolean;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  let panelEl = $state<HTMLDivElement | null>(null);
  let lastFocused: HTMLElement | null = null;

  onMount(() => {
    const onKeydown = (e: KeyboardEvent) => {
      if (!open || !closeOnEscape) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onclose();
      }
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  async function focusPanel() {
    await tick();
    panelEl?.focus();
  }

  $effect(() => {
    if (open) {
      lastFocused = document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
      focusPanel();
    } else if (lastFocused) {
      lastFocused.focus();
      lastFocused = null;
    }
  });

  function customSpring(t: number) {
    const c4 = (2 * Math.PI) / 3;
    return t === 0 ? 0 : t === 1 ? 1 : Math.pow(2, -10 * t) * Math.sin((t * 10 - 0.75) * c4) + 1;
  }
</script>

{#if open}
  <div
    class="sp-modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (closeOnBackdrop && e.target === e.currentTarget) onclose();
    }}
    transition:fade={{ duration: 250 }}
  >
    <div
      class="sp-modal sp-modal-{size}"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      tabindex="-1"
      bind:this={panelEl}
      transition:scale={{ duration: 400, start: 0.95, opacity: 0, easing: customSpring }}
    >
      <header class="sp-modal-header">
        <div class="sp-modal-heading">
          <h2 class="sp-modal-title">{title}</h2>
          {#if description}
            <p class="sp-modal-desc">{description}</p>
          {/if}
        </div>
        <IconButton icon="x" label="Close" size="sm" onclick={onclose} />
      </header>
      <div class="sp-modal-body">{@render children()}</div>
      {#if footer}
        <footer class="sp-modal-footer">{@render footer()}</footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .sp-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-6);
    background: rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
  }

  .sp-modal {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-height: min(85vh, 48rem);
    background: var(--sp-glass-strong);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-xl);
    box-shadow:
      0 16px 40px rgba(0, 0, 0, 0.3),
      inset 0 1px 1px rgba(255, 255, 255, 0.1),
      inset 0 0 0 1px rgba(255, 255, 255, 0.05);
    outline: none;
  }

  .sp-modal-sm {
    max-width: 24rem;
  }

  .sp-modal-md {
    max-width: 34rem;
  }

  .sp-modal-lg {
    max-width: 48rem;
  }

  .sp-modal-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-5) var(--sp-6) var(--sp-3);
    border-bottom: 1px solid var(--sp-border-faint);
  }

  .sp-modal-heading {
    min-width: 0;
  }

  .sp-modal-title {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-modal-desc {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-modal-body {
    padding: var(--sp-5) var(--sp-6);
    overflow-y: auto;
  }

  .sp-modal-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    padding: var(--sp-4) var(--sp-6);
    border-top: 1px solid var(--sp-border-faint);
  }
</style>