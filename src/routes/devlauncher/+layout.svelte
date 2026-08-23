<script lang="ts">
  import type { Snippet } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import Tabs from "$lib/components/ui/Tabs.svelte";
  import type { TabDef } from "$lib/components/ui/Tabs.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let { children }: { children: Snippet } = $props();

  const tabs: TabDef[] = [
    { id: "overview", label: i18n.t("devl.overview") as TranslationKey, icon: "home" },
    { id: "analyze", label: i18n.t("devl.analyze") as TranslationKey, icon: "search" },
    { id: "profiles", label: i18n.t("devl.profiles") as TranslationKey, icon: "bookmark" },
    { id: "processes", label: i18n.t("devl.processes") as TranslationKey, icon: "terminal" },
  ];

  const pathname = $derived($page.url.pathname);

  const active = $derived(
    pathname === "/devlauncher"
      ? "overview"
      : pathname.startsWith("/devlauncher/analyze")
        ? "analyze"
        : pathname.startsWith("/devlauncher/profiles")
          ? "profiles"
          : pathname.startsWith("/devlauncher/processes")
            ? "processes"
            : "overview",
  );

  function onTab(id: string) {
    goto(id === "overview" ? "/devlauncher" : `/devlauncher/${id}`);
  }
</script>

<div class="sp-dl">
  <div class="sp-dl-nav">
    <div class="sp-dl-nav-inner">
      <Tabs tabs={tabs} value={active} onchange={onTab} />
    </div>
  </div>
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

  .sp-dl-nav {
    position: sticky;
    top: 0;
    z-index: 5;
    background: var(--sp-bg-0);
    border-bottom: 1px solid var(--sp-border);
  }

  .sp-dl-nav-inner {
    max-width: 64rem;
    margin: 0 auto;
    padding: var(--sp-3) var(--sp-8);
  }

  .sp-dl-content {
    flex: 1 1 auto;
  }
</style>
