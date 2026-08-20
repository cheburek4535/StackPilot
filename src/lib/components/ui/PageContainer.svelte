<script lang="ts">
  import type { Snippet } from "svelte";

  export type PageWidth = "narrow" | "default" | "wide" | "full";

  let {
    width = "default",
    children,
  }: {
    width?: PageWidth;
    children: Snippet;
  } = $props();
</script>

<main class="sp-page sp-page-{width}">
  {@render children()}
</main>

<style>
  .sp-page {
    width: 100%;
    margin: 0 auto;
    padding: var(--sp-6) var(--sp-8) var(--sp-16);
  }

  .sp-page-narrow {
    max-width: 42rem;
  }

  .sp-page-default {
    max-width: 64rem;
  }

  .sp-page-wide {
    max-width: 84rem;
  }

  .sp-page-full {
    max-width: none;
  }
</style>