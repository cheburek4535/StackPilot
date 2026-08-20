<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";

  onMount(() => {
    goto(`/devlauncher/profiles/${encodeURIComponent($page.params.name ?? "")}`, {
      replaceState: true,
    });
  });
</script>

<div class="sp-redirect">Redirecting to DevLauncher profile…</div>

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
