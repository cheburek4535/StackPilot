<script lang="ts">
  import "$lib/core/theme.css";
  import AppShell from "$lib/components/ui/AppShell.svelte";

  let { children } = $props();
</script>

<AppShell>
  {@render children()}
</AppShell>