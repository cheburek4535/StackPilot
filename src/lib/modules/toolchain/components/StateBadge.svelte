<script lang="ts">
  // Бейдж презентационного состояния инструмента: тон и подпись —
  // из format.toolStateInfo (бэкенд-факты, без угадывания).
  import Badge from "$lib/components/ui/Badge.svelte";
  import { toolStateInfo, toolStateVersion } from "../format";
  import type { ToolState } from "../types";

  let {
    state,
    dot = true,
    withVersion = false,
    overrideLabel = null,
  }: {
    state: ToolState;
    dot?: boolean;
    withVersion?: boolean;
    /** Полная замена подписи состояния (например «в комплекте с python»
     *  для bundled-тула вместо «не установлен»). */
    overrideLabel?: string | null;
  } = $props();

  const info = $derived(
    overrideLabel
      ? { label: overrideLabel, tone: "cyan" as const, short: undefined as string | undefined }
      : toolStateInfo(state),
  );
  const version = $derived(withVersion ? toolStateVersion(state) : null);
</script>

<span class="state-badge" title={info.label}>
  <Badge tone={info.tone} {dot}>{info.label}</Badge>
  {#if version}
    <span class="version">{version}</span>
  {/if}
</span>

<style>
  .state-badge {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .version {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
    white-space: nowrap;
  }
</style>
