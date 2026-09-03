<script lang="ts">
  import Badge from "$lib/components/ui/Badge.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import type { IconName } from "$lib/components/ui/icons";
  import type { WorkspaceAssistantContext } from "$lib/modules/assistant/types";

  let {
    open = false,
    context,
    onclose,
  }: {
    open?: boolean;
    context: WorkspaceAssistantContext;
    onclose?: () => void;
  } = $props();

  const contextRows = $derived<{ label: string; value: string; icon: IconName }[]>([
    { label: "Project", value: context.projectName ?? "—", icon: "folder" },
    { label: "Path", value: context.projectPath ?? "—", icon: "external" },
    { label: "Active tab", value: context.tab, icon: "layers" },
  ]);
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="sp-assistant-backdrop" onclick={onclose} role="presentation"></div>
  <div class="sp-assistant" role="dialog" tabindex="-1" aria-label="AI Assistant (extension point)">
    <header class="sp-assistant-head">
      <div class="sp-assistant-title">
        <span class="sp-assistant-icon" aria-hidden="true">
          <Icon name="sparkles" size={16} />
        </span>
        <span>AI Assistant</span>
        <Badge tone="cyan">extension point</Badge>
      </div>
      <button class="sp-assistant-close" onclick={onclose} aria-label="Close assistant panel">
        <Icon name="x" size={16} />
      </button>
    </header>

    <div class="sp-assistant-body">
      <p class="sp-assistant-note">
        This panel is a reserved integration point for a future AI assistant.
        No AI features are wired up yet — there is no chat, no model call, and
        no project data is sent anywhere.
      </p>

      <div class="sp-assistant-section">
        <h4 class="sp-assistant-section-title">Context the assistant will receive</h4>
        <div class="sp-assistant-rows">
          {#each contextRows as row}
            <div class="sp-assistant-row">
              <span class="sp-assistant-row-icon" aria-hidden="true">
                <Icon name={row.icon} size={14} />
              </span>
              <span class="sp-assistant-row-label">{row.label}</span>
              <span class="sp-assistant-row-value">{row.value}</span>
            </div>
          {/each}
        </div>
      </div>

      <div class="sp-assistant-empty">
        <Badge tone="neutral">not implemented</Badge>
        <p>Nothing here is functional yet — this is scaffolding for later.</p>
      </div>
    </div>
  </div>
{/if}

<style>
  .sp-assistant-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    z-index: 40;
    backdrop-filter: blur(1px);
  }

  .sp-assistant {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(22rem, 92vw);
    z-index: 41;
    display: flex;
    flex-direction: column;
    background: var(--sp-glass-strong);
    border-left: 1px solid var(--sp-border-strong);
    box-shadow: var(--sp-shadow-3);
    animation: sp-slide-in-right 0.18s ease-out;
  }

  .sp-assistant-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    padding: var(--sp-4);
    border-bottom: 1px solid var(--sp-border);
    flex: 0 0 auto;
  }

  .sp-assistant-title {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    min-width: 0;
  }

  .sp-assistant-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--sp-radius-md);
    color: var(--sp-cyan);
    background: rgba(6, 182, 212, 0.12);
    border: 1px solid rgba(6, 182, 212, 0.3);
    flex-shrink: 0;
  }

  .sp-assistant-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border: none;
    border-radius: var(--sp-radius-md);
    background: transparent;
    color: var(--sp-text-2);
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .sp-assistant-close:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-assistant-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: var(--sp-4);
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
  }

  .sp-assistant-note {
    margin: 0;
    font-size: var(--sp-fs-sm);
    line-height: var(--sp-lh-normal);
    color: var(--sp-text-2);
  }

  .sp-assistant-section-title {
    margin: 0 0 var(--sp-2);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--sp-text-3);
  }

  .sp-assistant-rows {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-assistant-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    font-size: var(--sp-fs-sm);
  }

  .sp-assistant-row-icon {
    display: inline-flex;
    color: var(--sp-text-3);
    flex-shrink: 0;
  }

  .sp-assistant-row-label {
    color: var(--sp-text-3);
    flex: 0 0 auto;
  }

  .sp-assistant-row-value {
    color: var(--sp-text-1);
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    margin-left: auto;
  }

  .sp-assistant-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-4);
    border: 1px dashed var(--sp-border-strong);
    border-radius: var(--sp-radius-md);
    text-align: center;
  }

  .sp-assistant-empty p {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }
</style>
