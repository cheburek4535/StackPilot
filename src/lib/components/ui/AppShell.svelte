<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { page } from "$app/stores";
  import { APP_NAME, APP_VERSION } from "$lib/core/app";
  import { NAV_GROUPS, isNavItemActive, isNavGroupActive } from "$lib/core/navigation";
  import { initTheme } from "$lib/core/theme";
  import { getSettings } from "$lib/core/api";
  import { rememberRoute } from "$lib/core/lastRoute";
  import { onboarding, showOnboarding, reopenOnboarding } from "$lib/core/onboarding";
  import { helpMode, toggleHelpMode } from "$lib/core/help";
  import { workspaceContext } from "$lib/modules/workspace/context";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { Locale, TranslationKey } from "$lib/core/i18n.svelte";
  import { listenExitRequest } from "$lib/core/exit";
  import type { ExitAskPayload } from "$lib/core/exit";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import IconButton from "./IconButton.svelte";
  import ToastRegion from "./ToastRegion.svelte";
  import OnboardingOverlay from "./OnboardingOverlay.svelte";
  import ExitDialog from "./ExitDialog.svelte";

  let { children }: { children: Snippet } = $props();

  let restoreRoute = $state(true);

  /** Запрос выхода с запущенными процессами StackPilot (говорит бэкенд). */
  let exitAsk = $state<ExitAskPayload | null>(null);
  let unlistenExit: (() => void) | null = null;

  onMount(() => {
    initTheme();
    getSettings()
      .then((s) => {
        restoreRoute = s.restore_last_route;
        // The backend is the single source of truth for the UI language.
        i18n.setLocale((s.language as Locale) || "ru");
      })
      .catch(() => {});
    // First-run detection — open the welcome tour unless already seen.
    if (get(onboarding).firstRun) showOnboarding();
    listenExitRequest((payload) => {
      exitAsk = payload;
    }).then((unlisten) => {
      unlistenExit = unlisten;
    }).catch(() => {
      // Exit-диалог недоступен (например, браузерные dev-сборки без Tauri) —
      // закрытие окна обрабатывает бэкенд штатным образом.
    });
  });

  onDestroy(() => {
    if (unlistenExit) unlistenExit();
  });

  const pathname = $derived($page.url.pathname);

  // The project section is emphasized only when a project is actually open.
  const hasProject = $derived($workspaceContext.project !== null);

  // Remember the last visited route (UI-local, for restore-on-restart).
  $effect(() => {
    if (restoreRoute) rememberRoute(pathname);
  });

  function minimizeWindow() {
    getCurrentWindow().minimize();
  }
  function maximizeWindow() {
    getCurrentWindow().toggleMaximize();
  }
  function closeWindow() {
    getCurrentWindow().close();
  }
  
  function startDrag(e: PointerEvent) {
    if (e.target instanceof Element && e.target.closest('button, a')) return;
    getCurrentWindow().startDragging();
  }
</script>

<div class="sp-app">
  <header class="sp-topbar" data-tauri-drag-region onpointerdown={startDrag}>
    <a href="/" class="sp-brand">
      <img src="/images/logo-name.svg" alt={APP_NAME} class="sp-brand-logo" />
    </a>
    <div class="sp-topbar-drag" data-tauri-drag-region onpointerdown={startDrag}></div>
    <div class="sp-topbar-actions">
      <button
        type="button"
        class="sp-topbar-icon sp-help-mode-btn"
        class:sp-help-mode-btn-on={$helpMode}
        aria-pressed={$helpMode}
        aria-label={i18n.t("nav.beginner_mode") as TranslationKey}
        title={i18n.t("nav.beginner_mode") as TranslationKey}
        onclick={() => toggleHelpMode()}
      >
        <Icon name="sparkles" size={17} />
      </button>
      <IconButton
        icon="help"
        label={i18n.t("nav.getting_started") as TranslationKey}
        onclick={() => reopenOnboarding()}
      />
      <IconButton icon="settings" label={i18n.t("nav.settings") as TranslationKey} href="/settings" />
      <div class="sp-window-controls">
        <button class="sp-window-control" onclick={minimizeWindow} aria-label="Minimize">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M 0,5 10,5" stroke="currentColor" stroke-width="1.5"/></svg>
        </button>
        <button class="sp-window-control" onclick={maximizeWindow} aria-label="Maximize">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M 1,1 9,1 9,9 1,9 Z" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>
        </button>
        <button class="sp-window-control sp-window-close" onclick={closeWindow} aria-label="Close">
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M 1,1 9,9 M 1,9 9,1" stroke="currentColor" stroke-width="1.5"/></svg>
        </button>
      </div>
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
              class:sp-nav-group-muted={group.id === "project" && !hasProject}
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
        <a class="sp-getting-started" href="/roadmap">
          <Icon name="map" size={15} />
          <span>{i18n.t("nav.roadmap")}</span>
        </a>
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

{#if exitAsk}
  <ExitDialog payload={exitAsk} onclose={() => (exitAsk = null)} />
{/if}

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
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    background: transparent;
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
  }

  /* ---- topbar (glass) ---- */

  .sp-topbar {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    height: var(--sp-topbar-h);
    padding: 0 var(--sp-4);
    background: transparent;
    border-bottom: none;
    box-shadow: none;
    z-index: 10;
  }

  .sp-brand {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    text-decoration: none;
    color: var(--sp-text-1);
    position: relative;
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
    background: linear-gradient(135deg, var(--sp-accent), var(--sp-accent-strong));
    box-shadow:
      var(--sp-gloss-top),
      0 2px 8px rgba(228, 87, 10, 0.35);
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

  .sp-topbar-drag {
    flex: 1;
    height: 100%;
    /* No visual styles needed, this is an invisible drag handle */
  }

  .sp-brand-logo {
    height: 30px;
    width: auto;
  }

  .sp-brand:hover .sp-brand-logo {
    opacity: 0.92;
  }

  .sp-topbar-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .sp-topbar-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid transparent;
    background: transparent;
    color: var(--sp-text-2);
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      border-color 0.15s ease,
      color 0.15s ease;
  }

  .sp-topbar-icon:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-window-controls {
    display: flex;
    align-items: center;
    margin-left: var(--sp-2);
    -webkit-app-region: no-drag;
  }

  .sp-window-control {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2rem;
    border: none;
    background: transparent;
    color: var(--sp-text-2);
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .sp-window-control:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }

  .sp-window-close:hover {
    background: #e81123;
    color: white;
  }

  /* ---- body ---- */

  .sp-body {
    flex: 1 1 auto;
    display: flex;
    min-height: 0;
    position: relative;
    z-index: 1;
  }

  /* ---- sidebar (glass) ---- */

  .sp-sidebar {
    flex: 0 0 var(--sp-sidebar-w);
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: transparent;
    border-right: none;
    box-shadow: none;
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
    color: var(--sp-text-1);
  }

  .sp-nav-group-muted .sp-nav-group-label {
    color: var(--sp-text-3);
  }

  .sp-nav-group-muted .sp-nav-item {
    opacity: 0.55;
  }

  .sp-nav-group-muted .sp-nav-item:hover {
    opacity: 1;
  }

  .sp-nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--sp-radius-sm);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-medium);
    text-decoration: none;
    box-shadow: var(--sp-gloss-top);
    transition:
      background-color 0.15s ease,
      color 0.15s ease,
      box-shadow 0.15s ease;
  }

  .sp-nav-item::before {
    content: "";
    position: absolute;
    left: 0;
    top: 50%;
    width: 2px;
    height: 0.875rem;
    border-radius: 1px;
    transform: translateY(-50%);
    background: var(--sp-accent);
    opacity: 0;
    transition: opacity 0.15s ease;
  }

  .sp-nav-item:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    text-decoration: none;
  }

  .sp-nav-item-active {
    background: var(--sp-surface-grad),
      var(--sp-bg-2);
    color: var(--sp-text-1);
    box-shadow:
      var(--sp-gloss-top-strong),
      inset 0 0 0 1px var(--sp-border);
  }

  .sp-nav-item-active::before {
    opacity: 1;
  }

  .sp-nav-item-active:hover {
    background: var(--sp-surface-grad),
      var(--sp-bg-2);
    color: var(--sp-text-1);
    box-shadow:
      var(--sp-gloss-top-strong),
      inset 0 0 0 1px var(--sp-border);
  }

  .sp-nav-item-label {
    line-height: 1;
  }

  .sp-sidebar-footer {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
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
    text-align: left;
    text-decoration: none;
    cursor: pointer;
    box-shadow: var(--sp-gloss-top);
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
    background: var(--sp-bg-0);
    border-radius: 10px;
    margin: 0 8px 8px 0;
    box-shadow: 
      0 8px 32px rgba(0, 0, 0, 0.4), 
      inset 0 1px 1px rgba(255, 255, 255, 0.1),
      inset 0 0 0 1px rgba(255, 255, 255, 0.05);
    display: flex;
    flex-direction: column;
    backdrop-filter: blur(24px) saturate(150%);
  }
</style>