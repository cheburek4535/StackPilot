<script lang="ts">
  import { toasts, dismissToast } from "$lib/core/toasts";
  import Toast from "./Toast.svelte";
</script>

{#if $toasts.length > 0}
  <div class="sp-toast-region" aria-live="polite" aria-atomic="false">
    {#each $toasts as toast (toast.id)}
      <Toast toast={toast} onclose={dismissToast} />
    {/each}
  </div>
{/if}

<style>
  .sp-toast-region {
    position: fixed;
    right: var(--sp-6);
    bottom: var(--sp-6);
    z-index: 1100;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--sp-3);
    pointer-events: none;
  }

  .sp-toast-region :global(.sp-toast) {
    pointer-events: auto;
  }
</style>