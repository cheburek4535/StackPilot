<script lang="ts">
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="sp-dl">
  <div class="sp-dl-content">
    {@render children()}
  </div>
</div>

<style>
  .sp-dl {
    min-height: 100%;
    display: flex;
    flex-direction: column;
  }

  .sp-dl-content {
    flex: 1 1 auto;
  }
</style>