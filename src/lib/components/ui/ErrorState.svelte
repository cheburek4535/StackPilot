<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import Button from "./Button.svelte";

  let {
    title = "Something went wrong",
    message,
    retry,
    action,
  }: {
    title?: string;
    message?: string;
    retry?: () => void;
    action?: Snippet;
  } = $props();
</script>

<div class="sp-error" role="alert">
  <span class="sp-error-icon" aria-hidden="true">
    <Icon name="alert" size={24} />
  </span>
  <p class="sp-error-title">{title}</p>
  {#if message}
    <pre class="sp-error-message">{message}</pre>
  {/if}
  {#if retry || action}
    <div class="sp-error-actions">
      {#if retry}
        <Button variant="secondary" size="sm" icon="refresh" onclick={retry}>
          Try again
        </Button>
      {/if}
      {#if action}
        {@render action()}
      {/if}
    </div>
  {/if}
</div>

<style>
  .sp-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: var(--sp-2);
    padding: var(--sp-10) var(--sp-6);
    border: 1px solid var(--sp-danger-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-danger-soft);
    width: 100%;
  }

  .sp-error-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3.25rem;
    height: 3.25rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-danger-soft);
    color: var(--sp-danger);
    border: 1px solid var(--sp-danger-border);
    margin-bottom: var(--sp-1);
  }

  .sp-error-title {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-error-message {
    margin: 0;
    max-width: 34rem;
    max-height: 8rem;
    overflow: auto;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    background: var(--sp-code-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
    padding: var(--sp-2) var(--sp-3);
    text-align: left;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .sp-error-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
  }
</style>