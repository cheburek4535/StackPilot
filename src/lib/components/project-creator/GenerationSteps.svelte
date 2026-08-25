<script lang="ts">
  import type { StepPreview } from "$lib/modules/project_creator/types";
  import Icon from "$lib/components/ui/Icon.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    steps,
    removableIds,
    removedIds = $bindable([]),
  }: {
    steps: StepPreview[];
    removableIds: string[];
    removedIds?: string[];
  } = $props();

  function isRemoved(id: string): boolean {
    return removedIds.includes(id);
  }

  /** Кнопка удаления/восстановления: шаг опционален (removable) ИЛИ уже
   *  удалён пользователем (после применения шаг исчезает из removable_ids
   *  плана, но остаётся в списке шагов — восстановление должно работать). */
  function isRemovable(id: string): boolean {
    return removableIds.includes(id) || removedIds.includes(id);
  }

  function toggleRemove(id: string) {
    if (isRemoved(id)) {
      removedIds = removedIds.filter((r) => r !== id);
    } else {
      removedIds = [...removedIds, id];
    }
  }

  function stepIcon(step: StepPreview): "check" | "alert" | "info" {
    if (!step.will_execute) return "alert";
    if (isRemoved(step.id)) return "info";
    return "check";
  }

  function stepClass(step: StepPreview): string {
    if (isRemoved(step.id)) return "gs-step-removed";
    if (!step.will_execute) return "gs-step-skip";
    return "gs-step-active";
  }
</script>

<div class="gs-container">
  <div class="gs-legend">
    <span class="gs-legend-item">
      <Icon name="check" size={12} class="gs-legend-icon-certain" />
      {i18n.t("create.preview.will_execute") as TranslationKey}
    </span>
    <span class="gs-legend-item">
      <Icon name="alert" size={12} class="gs-legend-icon-skip" />
      {i18n.t("create.preview.will_skip") as TranslationKey}
    </span>
    <span class="gs-legend-item">
      <Icon name="info" size={12} class="gs-legend-icon-removed" />
      {i18n.t("create.preview.removed") as TranslationKey}
    </span>
  </div>

  <div class="gs-steps">
    {#each steps as step (step.id)}
      <div class="gs-step {stepClass(step)}">
        <div class="gs-step-icon">
          <Icon name={stepIcon(step)} size={14} />
        </div>
        <div class="gs-step-info">
          <span class="gs-step-label">{step.label}</span>
          <span class="gs-step-action">{step.action}</span>
          {#if step.skip_reason}
            <span class="gs-step-skip-reason">{step.skip_reason}</span>
          {/if}
          {#if isRemovable(step.id)}
            <span class="gs-step-optional">
              <Icon name="info" size={10} />
              {i18n.t("create.preview.optional_note") as TranslationKey}
            </span>
          {/if}
        </div>
{#if isRemovable(step.id)}
            <button
              class="gs-remove-btn"
              class:gs-remove-btn-active={isRemoved(step.id)}
              onclick={() => toggleRemove(step.id)}
              title={isRemoved(step.id)
                ? (i18n.t("create.preview.restore_step") as TranslationKey)
                : (i18n.t("create.preview.remove_step") as TranslationKey)}
            >
              <Icon name={isRemoved(step.id) ? "refresh" : "trash"} size={12} />
            </button>
          {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .gs-container {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .gs-legend {
    display: flex;
    gap: var(--sp-4);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-2);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .gs-legend-item {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }

  :global(.gs-legend-icon-certain) {
    color: var(--sp-success);
  }

  :global(.gs-legend-icon-skip) {
    color: var(--sp-text-3);
  }

  :global(.gs-legend-icon-removed) {
    color: var(--sp-red);
  }

  .gs-steps {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .gs-step {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-sm);
    transition: background 0.1s, opacity 0.15s;
  }

  .gs-step-active {
    background: var(--sp-bg-1);
  }

  .gs-step-skip {
    background: var(--sp-bg-2);
    opacity: 0.6;
  }

  .gs-step-removed {
    background: rgba(248, 113, 113, 0.06);
    opacity: 0.5;
    text-decoration: line-through;
  }

  .gs-step-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
  }

  .gs-step-active .gs-step-icon {
    color: var(--sp-success);
  }

  .gs-step-skip .gs-step-icon {
    color: var(--sp-text-3);
  }

  .gs-step-removed .gs-step-icon {
    color: var(--sp-red);
  }

  .gs-step-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }

  .gs-step-label {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    color: var(--sp-text-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .gs-step-action {
    font-family: var(--sp-font-mono);
    font-size: 0.65rem;
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .gs-step-skip-reason {
    font-size: 0.65rem;
    color: var(--sp-text-3);
    font-style: italic;
  }

  .gs-step-optional {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 0.65rem;
    color: var(--sp-amber);
  }

  .gs-remove-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: none;
    color: var(--sp-text-3);
    cursor: pointer;
    border-radius: var(--sp-radius-sm);
    flex-shrink: 0;
    transition: background 0.1s, color 0.1s;
  }

  .gs-remove-btn:hover {
    background: rgba(248, 113, 113, 0.12);
    color: var(--sp-red);
  }

  .gs-remove-btn-active {
    color: var(--sp-red);
    background: rgba(248, 113, 113, 0.12);
  }

  .gs-remove-btn-active:hover {
    background: rgba(163, 230, 53, 0.12);
    color: var(--sp-success);
  }
</style>
