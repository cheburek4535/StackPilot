<script lang="ts">
  import "@fontsource-variable/inter/wght.css";
  import "@fontsource-variable/jetbrains-mono/wght.css";
  import "$lib/core/theme.css";
  import { initTauriMock } from "$lib/core/tauriMock";
  import AppShell from "$lib/components/ui/AppShell.svelte";

  initTauriMock();

  let { children } = $props();
</script>

<AppShell>
  {@render children()}
</AppShell>
