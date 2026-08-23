<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import { page } from "$app/stores";
  import { APP_NAME, APP_VERSION } from "$lib/core/app";
  import { NAV_GROUPS, isNavItemActive, isNavGroupActive } from "$lib/core/navigation";
  import { initTheme } from "$lib/core/theme";
  import { rememberRoute } from "$lib/core/lastRoute";
  import { onboarding, showOnboarding, reopenOnboarding } from "$lib/core/onboarding";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import IconButton from "./IconButton.svelte";
  import ToastRegion from "./ToastRegion.svelte";
  import OnboardingOverlay from "./OnboardingOverlay.svelte";

  let { children }: { children: Snippet } = $props();

  onMount(() => {
    initTheme();
    // First-run detection — open the welcome tour unless already seen.
    if (get(onboarding).firstRun) showOnboarding();
  });

  const pathname = $derived($page.url.pathname);

  // Remember the last visited route (UI-local, for restore-on-restart).
  $effect(() => {
    rememberRoute(pathname);
  });
</script>

<div class="sp-app">
  <header class="sp-topbar">
    <a href="/" class="sp-brand">
      <span class="sp-brand-mark" aria-hidden="true">
        <Icon name="rocket" size={16} />
      </span>
      <span class="sp-brand-name">{APP_NAME}</span>
      <span class="sp-brand-version">{APP_VERSION}</span>
    </a>
    <div class="sp-topbar-actions">
      <IconButton
        icon="help"
        label={i18n.t("nav.getting_started") as TranslationKey}
        onclick={() => reopenOnboarding()}
      />
      <IconButton icon="settings" label={i18n.t("nav.settings") as TranslationKey} href="/settings" />
    </div>
  </header>

  <div class="sp-body">
    <aside class="sp-sidebar">
      <nav class="sp-nav" aria-label="Main navigation">
        {#each NAV_GROUPS as group}
          {#if group.label}
            <div
              class="sp-nav-group"
              class:sp-nav-group-active={isNavGroupActive(group, pathname)}
            >
              <span class="sp-nav-group-label">{i18n.t(group.label as TranslationKey)}</span>
              {#each group.items as item}
                {@render navItem(item)}
              {/each}
            </div>
          {:else}
            {#each group.items as item}
              {@render navItem(item)}
            {/each}
          {/if}
        {/each}
      </nav>

      <div class="sp-sidebar-footer">
        <button
          class="sp-getting-started"
          onclick={() => reopenOnboarding()}
        >
          <Icon name="help" size={15} />
          <span>{i18n.t("nav.getting_started")}</span>
        </button>
      </div>
    </aside>

    <div class="sp-content">
      {@render children()}
    </div>
  </div>

  <ToastRegion />
  <OnboardingOverlay />
</div>

{#snippet navItem(item: { id: string; label: string; href: string; icon: string; match: (p: string) => boolean })}
  <a
    href={item.href}
    class="sp-nav-item"
    class:sp-nav-item-active={isNavItemActive(item, pathname)}
    aria-current={isNavItemActive(item, pathname) ? "page" : undefined}
  >
    <Icon name={item.icon as IconName} size={16} />
    <span class="sp-nav-item-label">{i18n.t(item.label as TranslationKey)}</span>
  </a>
{/snippet}

<style>
  .sp-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    background: var(--sp-bg-0);
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
  }

  /* ---- topbar ---- */

  .sp-topbar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    height: var(--sp-topbar-h);
    padding: 0 var(--sp-4);
    background: var(--sp-glass-strong);
    border-bottom: 1px solid var(--sp-border);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    z-index: 10;
  }

  .sp-brand {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    text-decoration: none;
    color: var(--sp-text-1);
  }

  .sp-brand:hover {
    text-decoration: none;
  }

  .sp-brand-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--sp-radius-md);
    color: #fff;
    background: linear-gradient(135deg, var(--sp-violet-strong), var(--sp-blue-strong));
    box-shadow: var(--sp-shadow-1);
  }

  .sp-brand-name {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.01em;
  }

  .sp-brand-version {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    padding: 0.125rem var(--sp-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-full);
  }

  .sp-topbar-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }

  /* ---- body ---- */

  .sp-body {
    flex: 1 1 auto;
    display: flex;
    min-height: 0;
  }

  .sp-sidebar {
    flex: 0 0 var(--sp-sidebar-w);
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--sp-bg-1);
    border-right: 1px solid var(--sp-border);
  }

  .sp-nav {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: var(--sp-4) var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-nav-group {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-nav-group-label {
    padding: var(--sp-2) var(--sp-3) var(--sp-1);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--sp-text-3);
  }

  .sp-nav-group-active .sp-nav-group-label {
    color: var(--sp-accent);
  }

  .sp-nav-item {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-md);
    border: 1px solid transparent;
    color: var(--sp-text-2);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    text-decoration: none;
    transition:
      background-color 0.15s ease,
      color 0.15s ease,
      border-color 0.15s ease;
  }

  .sp-nav-item:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    text-decoration: none;
  }

  .sp-nav-item-active {
    background: var(--sp-accent-soft);
    border-color: var(--sp-accent-border);
    color: var(--sp-accent);
  }

  .sp-nav-item-active:hover {
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
  }

  .sp-nav-item-label {
    line-height: 1;
  }

  .sp-sidebar-footer {
    flex: 0 0 auto;
    padding: var(--sp-3);
    border-top: 1px solid var(--sp-border-faint);
  }

  .sp-getting-started {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    border: none;
    border-radius: var(--sp-radius-md);
    background: transparent;
    color: var(--sp-text-3);
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-medium);
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .sp-getting-started:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  /* ---- content ---- */

  .sp-content {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
</style>