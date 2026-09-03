<script lang="ts">
  import { onMount } from "svelte";
  import type { Toast as ToastData } from "$lib/core/toasts";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import IconButton from "./IconButton.svelte";

  const KIND_ICON: Record<ToastData["kind"], IconName> = {
    info: "info",
    success: "check",
    warning: "alert",
    error: "x",
  };

  let {
    toast,
    onclose,
  }: {
    toast: ToastData;
    onclose: (id: number) => void;
  } = $props();

  let timer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    if (toast.durationMs > 0) {
      timer = setTimeout(() => onclose(toast.id), toast.durationMs);
    }
    return () => {
      if (timer) clearTimeout(timer);
    };
  });
</script>

<div class="sp-toast sp-toast-{toast.kind}" role="status">
  <span class="sp-toast-icon" aria-hidden="true">
    <Icon name={KIND_ICON[toast.kind]} size={16} />
  </span>
  <div class="sp-toast-content">
    <p class="sp-toast-title">{toast.title}</p>
    {#if toast.message}
      <p class="sp-toast-message">{toast.message}</p>
    {/if}
  </div>
  <IconButton
    icon="x"
    label="Dismiss notification"
    size="sm"
    onclick={() => onclose(toast.id)}
  />
</div>

<style>
  .sp-toast {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    width: min(22rem, calc(100vw - 3rem));
    padding: var(--sp-3) var(--sp-3) var(--sp-3) var(--sp-4);
    background: var(--sp-glass-strong);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-2);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    animation: sp-slide-in-right 0.2s ease;
  }

  .sp-toast-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-md);
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
  }

  .sp-toast-success .sp-toast-icon {
    background: rgba(132, 204, 22, 0.12);
    color: var(--sp-lime);
  }

  .sp-toast-warning .sp-toast-icon {
    background: rgba(245, 158, 11, 0.12);
    color: var(--sp-amber);
  }

  .sp-toast-error .sp-toast-icon {
    background: rgba(239, 68, 68, 0.12);
    color: var(--sp-red);
  }

  .sp-toast-content {
    flex: 1;
    min-width: 0;
    padding-top: var(--sp-1);
  }

  .sp-toast-title {
    margin: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
    word-break: break-word;
  }

  .sp-toast-message {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
    word-break: break-word;
  }
</style>