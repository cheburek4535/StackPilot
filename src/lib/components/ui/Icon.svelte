<script lang="ts">
  import { ICON_PATHS } from "./icons";
  import type { IconName } from "./icons";

  let {
    name,
    size = 18,
    strokeWidth = 2,
    class: extraClass = "",
  }: {
    name: IconName;
    size?: number;
    strokeWidth?: number;
    class?: string;
  } = $props();
</script>

<svg
  class="sp-icon {extraClass}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#each ICON_PATHS[name] as d}
    <path d={d} />
  {/each}
</svg>

<style>
  .sp-icon {
    flex: 0 0 auto;
    vertical-align: middle;
  }
</style>