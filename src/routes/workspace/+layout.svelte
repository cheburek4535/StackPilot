<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import Tabs from "$lib/components/ui/Tabs.svelte";
  import type { TabDef } from "$lib/components/ui/Tabs.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import AssistantPanel from "$lib/components/workspace/AssistantPanel.svelte";
  import {
    workspaceContext,
    reloadWorkspaceContext,
    clearWorkspaceProject,
  } from "$lib/modules/workspace/context";
  import { notifyInfo } from "$lib/core/toasts";
  import { markHelpDid, HELP, HINT_WORKSPACE_WELCOME, helpMode, helpProgress, isHintVisible } from "$lib/core/help";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";
  import type { WorkspaceAssistantContext } from "$lib/modules/assistant/types";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let { children }: { children: Snippet } = $props();

  const tabs: TabDef[] = [
    { id: "dashboard", label: i18n.t("ws.dashboard") as TranslationKey, icon: "home" },
    { id: "files", label: i18n.t("ws.files") as TranslationKey, icon: "folder" },
    { id: "logs", label: i18n.t("ws.logs") as TranslationKey, icon: "terminal" },
    { id: "problems", label: i18n.t("ws.problems") as TranslationKey, icon: "alert" },
  ];

  let assistantOpen = $state(false);

  onMount(() => {
    reloadWorkspaceContext();
  });

  const pathname = $derived($page.url.pathname);

  const activeTab = $derived(
    pathname === "/workspace"
      ? "dashboard"
      : pathname.startsWith("/workspace/files")
        ? "files"
        : pathname.startsWith("/workspace/logs")
          ? "logs"
          : pathname.startsWith("/workspace/problems")
            ? "problems"
            : "dashboard",
  );

  const project = $derived($workspaceContext.project);

  const workspaceHintVisible = $derived(
    isHintVisible($helpProgress, $helpMode, HINT_WORKSPACE_WELCOME),
  );

  const assistantContext = $derived<WorkspaceAssistantContext>({
    projectName: project?.profile_name ?? null,
    projectPath: project?.project_path ?? null,
    tab: activeTab,
  });

  function onTab(id: string) {
    markHelpDid(HELP.workspaceExplored);
    goto(id === "dashboard" ? "/workspace" : `/workspace/${id}`);
  }

  async function closeWorkspace() {
    await clearWorkspaceProject();
    notifyInfo(i18n.t("ws.toast_workspace"), i18n.t("ws.toast_closed"));
  }
</script>

<div class="sp-ws">
  <header class="sp-ws-head">
    <div class="sp-ws-head-inner">
      <div class="sp-ws-title">
        <span class="sp-ws-logo" aria-hidden="true">
          <Icon name="layers" size={20} />
        </span>
        <div class="sp-ws-title-text">
          <h1 class="sp-ws-title-main">{i18n.t("ws.title") as TranslationKey}</h1>
          <p class="sp-ws-title-sub">
            {#if project}
              {project.profile_name}
            {:else}
              {i18n.t("ws.no_project") as TranslationKey}
            {/if}
          </p>
        </div>
      </div>

      <div class="sp-ws-head-actions">
        <Button
          variant="secondary"
          size="sm"
          icon="sparkles"
          label={i18n.t("ws.assistant_aria") as TranslationKey}
          onclick={() => (assistantOpen = true)}
        >
          {i18n.t("ws.assistant") as TranslationKey}
        </Button>
        {#if project}
          <Button variant="ghost" size="sm" icon="x" onclick={closeWorkspace}>
            {i18n.t("ws.close_workspace") as TranslationKey}
          </Button>
        {/if}
      </div>
    </div>

    {#if project}
      <div class="sp-ws-head-context">
        {#if project.project_path}
          <span class="sp-ws-path">{project.project_path}</span>
        {/if}
        {#if project.stack.length > 0}
          <span class="sp-ws-tags">
            {#each project.stack as tech}
              <Badge tone="blue">{tech}</Badge>
            {/each}
          </span>
        {/if}
        <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
      </div>
    {/if}
  </header>

  <nav class="sp-ws-tabs" aria-label={i18n.t("ws.sections_aria") as TranslationKey}>
    <Tabs tabs={tabs} value={activeTab} onchange={onTab} />
  </nav>

  <div class="sp-ws-content">
    {#if workspaceHintVisible}
      <div class="sp-ws-help">
        <HelpHint
          id={HINT_WORKSPACE_WELCOME.id}
          resolvedBy={HINT_WORKSPACE_WELCOME.resolvedBy}
          icon="layers"
          title={i18n.t("help.workspace.title") as TranslationKey}
          text={i18n.t("help.workspace.body") as TranslationKey}
        />
      </div>
    {/if}
    {@render children()}
  </div>

  <AssistantPanel
    open={assistantOpen}
    context={assistantContext}
    onclose={() => (assistantOpen = false)}
  />
</div>

<style>
  .sp-ws {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }

  /* ---- header ---- */

  .sp-ws-head {
    position: sticky;
    top: 0;
    z-index: 5;
    background: var(--sp-solid-chrome);
    border-bottom: 1px solid var(--sp-border);
  }

  .sp-ws-head-inner {
    max-width: 84rem;
    margin: 0 auto;
    padding: var(--sp-4) var(--sp-8);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .sp-ws-title {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .sp-ws-logo {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2.5rem;
    flex: 0 0 auto;
    border-radius: var(--sp-radius-lg);
    color: var(--sp-text-2);
    background: var(--sp-surface-grad), var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
  }

  .sp-ws-title-text {
    min-width: 0;
  }

  .sp-ws-title-main {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.01em;
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-ws-title-sub {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-tight);
  }

  .sp-ws-head-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .sp-ws-head-context {
    max-width: 84rem;
    margin: 0 auto;
    padding: 0 var(--sp-8) var(--sp-3);
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .sp-ws-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-ws-tags {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  /* ---- tabs ---- */

  .sp-ws-tabs {
    background: var(--sp-bg-0);
    border-bottom: 1px solid var(--sp-border);
  }

  .sp-ws-tabs :global(.sp-tabs) {
    max-width: 84rem;
    margin: 0 auto;
    width: auto;
    background: transparent;
    border: none;
    padding: var(--sp-2) var(--sp-8);
    border-radius: 0;
  }

  .sp-ws-content {
    flex: 1 1 auto;
    min-height: 0;
  }

  .sp-ws-help {
    max-width: 84rem;
    margin: 0 auto;
    padding: var(--sp-4) var(--sp-8) 0;
  }

  .sp-ws-help:empty {
    display: none;
  }
</style>
