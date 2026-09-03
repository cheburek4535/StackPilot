<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  onMount(() => {
    goto("/workspace", { replaceState: true });
  });
</script>

<div class="sp-redirect">{i18n.t("devl.redirect_profiles") as TranslationKey}</div>

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
