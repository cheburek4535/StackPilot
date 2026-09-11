<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  export type IconButtonVariant = "ghost" | "outline" | "solid" | "danger";
  export type IconButtonSize = "sm" | "md";

  let {
    icon,
    label,
    variant = "ghost",
    size = "md",
    disabled = false,
    href,
    onclick,
  }: {
    icon: IconName;
    label: string;
    variant?: IconButtonVariant;
    size?: IconButtonSize;
    disabled?: boolean;
    href?: string;
    onclick?: (e: MouseEvent) => void;
  } = $props();

  const classes = $derived(
    `sp-icon-btn sp-icon-btn-${variant} sp-icon-btn-${size}`,
  );
  const iconSize = $derived(size === "sm" ? 15 : 17);
</script>

{#if href}
  <a
    href={href}
    class={classes}
    aria-label={label}
    title={label}
    aria-disabled={disabled}
    role="button"
  >
    <Icon name={icon} size={iconSize} />
  </a>
{:else}
  <button
    class={classes}
    aria-label={label}
    title={label}
    disabled={disabled}
    onclick={onclick}
  >
    <Icon name={icon} size={iconSize} />
  </button>
{/if}

<style>
  .sp-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid transparent;
    background: transparent;
    color: var(--sp-text-2);
    cursor: pointer;
    text-decoration: none;
    transition:
      background-color 0.15s ease,
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .sp-icon-btn:hover:not(:disabled) {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-icon-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .sp-icon-btn-sm {
    width: 1.625rem;
    height: 1.625rem;
    border-radius: var(--sp-radius-sm);
  }

  .sp-icon-btn-outline {
    border-color: var(--sp-border);
  }

  .sp-icon-btn-outline:hover:not(:disabled) {
    border-color: var(--sp-border-strong);
  }

  .sp-icon-btn-solid {
    background: var(--sp-accent-strong);
    color: #fff;
  }

  .sp-icon-btn-solid:hover:not(:disabled) {
    background: var(--sp-accent);
    color: #fff;
  }

  .sp-icon-btn-danger {
    color: var(--sp-danger);
  }

  .sp-icon-btn-danger:hover:not(:disabled) {
    background: var(--sp-danger-soft);
    color: var(--sp-danger);
  }
</style>