<script lang="ts">
  export type ProgressSize = "sm" | "md" | "lg";

  let {
    value,
    max = 1,
    indeterminate = false,
    size = "md",
    label,
  }: {
    /** Determinate progress value (0..max). Omit or set indeterminate for unknown duration. */
    value?: number;
    max?: number;
    indeterminate?: boolean;
    size?: ProgressSize;
    label?: string;
  } = $props();

  const pct = $derived(
    indeterminate
      ? null
      : Math.max(0, Math.min(100, ((value ?? 0) / Math.max(1, max)) * 100)),
  );
</script>

<div
  class="sp-progress sp-progress-{size}"
  role="progressbar"
  aria-valuemin="0"
  aria-valuemax={max}
  aria-valuenow={indeterminate ? undefined : value ?? 0}
  aria-valuetext={label}
>
  <div class="sp-progress-track">
    {#if indeterminate}
      <div class="sp-progress-fill sp-progress-indeterminate" aria-hidden="true"></div>
    {:else}
      <div class="sp-progress-fill" style="width: {pct}%" aria-hidden="true"></div>
    {/if}
  </div>
  {#if label}
    <span class="sp-progress-label">{label}</span>
  {/if}
</div>

<style>
  .sp-progress {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    width: 100%;
  }

  .sp-progress-track {
    position: relative;
    overflow: hidden;
    width: 100%;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full);
  }

  .sp-progress-sm .sp-progress-track {
    height: 0.3125rem;
  }

  .sp-progress-md .sp-progress-track {
    height: 0.5rem;
  }

  .sp-progress-lg .sp-progress-track {
    height: 0.75rem;
  }

  .sp-progress-fill {
    position: absolute;
    inset: 0 auto 0 0;
    height: 100%;
    border-radius: var(--sp-radius-full);
    background: linear-gradient(90deg, var(--sp-violet-strong), var(--sp-cyan));
    box-shadow: 0 0 12px rgba(160, 139, 232, 0.35);
    transition: width 0.25s ease;
  }

  .sp-progress-indeterminate {
    width: 40%;
    animation: sp-indeterminate 1.4s ease-in-out infinite;
  }

  .sp-progress-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }
</style>