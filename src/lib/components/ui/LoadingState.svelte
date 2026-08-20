<script lang="ts">
  let {
    label = "Loading…",
    size = "md",
    center = true,
  }: {
    label?: string;
    size?: "sm" | "md" | "lg";
    center?: boolean;
  } = $props();
</script>

<div class="sp-loading" class:sp-loading-center={center}>
  <span class="sp-loading-spinner sp-loading-{size}" aria-hidden="true"></span>
  {#if label}
    <span class="sp-loading-label">{label}</span>
  {/if}
</div>

<style>
  .sp-loading {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
  }

  .sp-loading-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--sp-8);
    width: 100%;
  }

  .sp-loading-spinner {
    border-radius: var(--sp-radius-full);
    border: 2px solid var(--sp-accent-soft);
    border-top-color: var(--sp-accent);
    animation: sp-spin 0.9s linear infinite;
  }

  .sp-loading-sm {
    width: 0.9rem;
    height: 0.9rem;
  }

  .sp-loading-md {
    width: 1.25rem;
    height: 1.25rem;
  }

  .sp-loading-lg {
    width: 1.75rem;
    height: 1.75rem;
  }

  .sp-loading-label {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }
</style>