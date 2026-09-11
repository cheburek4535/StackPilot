<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  export type ButtonVariant =
    | "primary"
    | "secondary"
    | "ghost"
    | "outline"
    | "danger"
    | "subtle";
  export type ButtonSize = "sm" | "md" | "lg";

  let {
    variant = "primary",
    size = "md",
    icon,
    iconRight,
    loading = false,
    disabled = false,
    block = false,
    href,
    type = "button",
    label,
    onclick,
    children,
  }: {
    variant?: ButtonVariant;
    size?: ButtonSize;
    icon?: IconName;
    iconRight?: IconName;
    loading?: boolean;
    disabled?: boolean;
    block?: boolean;
    href?: string;
    type?: "button" | "submit" | "reset";
    label?: string;
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
  } = $props();

  const isDisabled = $derived(disabled || loading);

  const classes = $derived(
    `sp-btn sp-btn-${variant} sp-btn-${size}${block ? " sp-btn-block" : ""}`,
  );
</script>

{#if href}
  <a
    href={href}
    class={classes}
    class:sp-btn-disabled={isDisabled}
    aria-label={label}
    aria-disabled={isDisabled}
    role="button"
  >
    {#if loading}
      <span class="sp-btn-spinner" aria-hidden="true"></span>
    {:else if icon}
      <Icon name={icon} size={size === "sm" ? 14 : size === "lg" ? 18 : 16} />
    {/if}
    {#if children}
      <span class="sp-btn-label">{@render children()}</span>
    {/if}
    {#if iconRight}
      <Icon name={iconRight} size={size === "sm" ? 14 : size === "lg" ? 18 : 16} />
    {/if}
  </a>
{:else}
  <button
    class={classes}
    class:sp-btn-disabled={isDisabled}
    type={type}
    disabled={isDisabled}
    aria-label={label}
    onclick={onclick}
  >
    {#if loading}
      <span class="sp-btn-spinner" aria-hidden="true"></span>
    {:else if icon}
      <Icon name={icon} size={size === "sm" ? 14 : size === "lg" ? 18 : 16} />
    {/if}
    {#if children}
      <span class="sp-btn-label">{@render children()}</span>
    {/if}
    {#if iconRight}
      <Icon name={iconRight} size={size === "sm" ? 14 : size === "lg" ? 18 : 16} />
    {/if}
  </button>
{/if}

<style>
  .sp-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    border-radius: var(--sp-radius-md);
    font-family: var(--sp-font-sans);
    font-weight: var(--sp-fw-semibold);
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
    text-decoration: none;
    user-select: none;
    border: 1px solid transparent;
    transition:
      background 0.15s ease,
      border-color 0.15s ease,
      color 0.15s ease,
      box-shadow 0.15s ease,
      filter 0.1s ease,
      transform 0.1s ease;
  }

  .sp-btn:active:not(.sp-btn-disabled) {
    transform: scale(0.98);
    filter: brightness(0.92);
  }

  .sp-btn-disabled {
    opacity: 0.5;
    cursor: not-allowed;
    pointer-events: none;
  }

  .sp-btn-label {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }

  /* ---- sizes ---- */

  .sp-btn-sm {
    padding: var(--sp-1) var(--sp-3);
    font-size: var(--sp-fs-xs);
    border-radius: var(--sp-radius-sm);
  }

  .sp-btn-md {
    padding: var(--sp-2) var(--sp-4);
    font-size: var(--sp-fs-sm);
  }

  .sp-btn-lg {
    padding: var(--sp-3) var(--sp-5);
    font-size: var(--sp-fs-md);
  }

  .sp-btn-block {
    display: flex;
    width: 100%;
  }

  /* ---- variants ---- */

  .sp-btn-primary {
    background: var(--sp-accent-strong);
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--sp-accent-strong) 72%, black) 0%,
      var(--sp-accent-strong) 45%,
      var(--sp-accent) 100%
    );
    color: #fff;
    border-color: rgba(0, 0, 0, 0.35);
    box-shadow:
      var(--sp-gloss-top),
      var(--sp-shadow-1),
      0 2px 14px rgba(228, 87, 10, 0.2);
    text-shadow:
      0 1px 2px rgba(0, 0, 0, 0.55),
      0 0 1px rgba(0, 0, 0, 0.4);
  }

  .sp-btn-primary:hover:not(.sp-btn-disabled) {
    background: var(--sp-accent-strong);
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--sp-accent-strong) 62%, black) 0%,
      var(--sp-accent-strong) 40%,
      color-mix(in srgb, var(--sp-accent) 88%, var(--sp-accent-strong)) 100%
    );
    box-shadow:
      var(--sp-gloss-top-strong),
      var(--sp-shadow-accent);
  }

  .sp-btn-secondary {
    background: var(--sp-surface-grad),
      var(--sp-bg-2);
    color: var(--sp-text-1);
    border-color: var(--sp-border);
    box-shadow: var(--sp-gloss-top);
  }

  .sp-btn-secondary:hover:not(.sp-btn-disabled) {
    background: var(--sp-bg-3);
    border-color: var(--sp-border-strong);
    box-shadow: var(--sp-gloss-top-strong);
  }

  .sp-btn-ghost {
    background: transparent;
    color: var(--sp-text-2);
    border-color: transparent;
  }

  .sp-btn-ghost:hover:not(.sp-btn-disabled) {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    box-shadow: var(--sp-gloss-top);
  }

  .sp-btn-outline {
    background: transparent;
    color: var(--sp-text-2);
    border-color: var(--sp-border);
    box-shadow: var(--sp-gloss-top);
  }

  .sp-btn-outline:hover:not(.sp-btn-disabled) {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    border-color: var(--sp-border-strong);
    box-shadow: var(--sp-gloss-top-strong);
  }

  .sp-btn-danger {
    background: var(--sp-danger-soft);
    color: var(--sp-danger);
    border-color: var(--sp-danger-border);
    box-shadow: var(--sp-gloss-top);
  }

  .sp-btn-danger:hover:not(.sp-btn-disabled) {
    background: rgba(239, 68, 68, 0.16);
    border-color: rgba(239, 68, 68, 0.45);
    box-shadow: var(--sp-gloss-top-strong);
  }

  .sp-btn-subtle {
    background: transparent;
    color: var(--sp-text-2);
    border-color: transparent;
    padding-left: var(--sp-2);
    padding-right: var(--sp-2);
  }

  .sp-btn-subtle:hover:not(.sp-btn-disabled) {
    color: var(--sp-text-1);
    background: var(--sp-bg-2);
    box-shadow: var(--sp-gloss-top);
  }

  /* ---- spinner ---- */

  .sp-btn-spinner {
    width: 0.85em;
    height: 0.85em;
    border-radius: var(--sp-radius-full);
    border: 2px solid currentColor;
    border-top-color: transparent;
    animation: sp-spin 0.8s linear infinite;
    opacity: 0.8;
  }
</style>