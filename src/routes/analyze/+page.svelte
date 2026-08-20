<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  onMount(() => {
    goto("/devlauncher/analyze", { replaceState: true });
  });
</script>

<div class="sp-redirect">Redirecting to DevLauncher Analyze…</div>

<style>
  .sp-redirect {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 40vh;
    color: var(--sp-text-3);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
  }
</style>
