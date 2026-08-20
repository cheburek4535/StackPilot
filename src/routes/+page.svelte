<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import { getCurrentProject, openInVSCode } from "$lib/modules/workspace/api";
  import type { ProjectContext } from "$lib/modules/workspace/types";
  import { openProject } from "$lib/core/integration";
  import { recentProjects } from "$lib/core/recent";
  import type { RecentProjectRef } from "$lib/core/recent";
  import { getDemoProfile, executeAction } from "$lib/modules/devlauncher/api";
  import type { LaunchProfile } from "$lib/modules/devlauncher/types";
  import {
    actionIcon,
    actionTypeLabel,
    actionSummary,
    formatResult,
    resultClass,
  } from "$lib/modules/devlauncher/actionMeta";
  import { notifySuccess, notifyError } from "$lib/core/toasts";

  let project = $state<ProjectContext | null>(null);
  let loading = $state(true);
  let error = $state("");
  let openingPath = $state<string | null>(null);

  let demo = $state<LaunchProfile | null>(null);
  let demoResults = $state<Map<string, string>>(new Map());
  let demoRunning = $state<Set<string>>(new Set());

  onMount(async () => {
    await Promise.all([loadProject(), loadDemo()]);
  });

  async function loadProject() {
    try {
      project = await getCurrentProject();
    } catch (e) {
      error = `Failed to load current project: ${e}`;
    }
    loading = false;
  }

  async function reload() {
    error = "";
    loading = true;
    await loadProject();
  }

  async function loadDemo() {
    try {
      demo = await getDemoProfile();
    } catch {
      // diagnostics section is optional — stay silent on failure
      demo = null;
    }
  }

  async function openRecent(ref: RecentProjectRef) {
    openingPath = ref.path;
    try {
      await openProject(ref.path);
      notifySuccess("Project opened", ref.name);
      goto("/workspace");
    } catch (e) {
      notifyError("Open project", `Failed to open ${ref.path}: ${e}`);
    }
    openingPath = null;
  }

  async function openInVSCodeSafe(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess("VS Code", "Opening project in VS Code");
    } catch (e) {
      notifyError("VS Code", `Failed to open: ${e}`);
    }
  }

  async function runDemoAction(actionId: string) {
    if (!demo) return;
    const action = demo.actions.find((a) => a.id === actionId);
    if (!action) return;
    demoRunning = new Set(demoRunning).add(actionId);
    try {
      const result = await executeAction(action);
      demoResults = new Map(demoResults).set(actionId, formatResult(result));
    } catch (e) {
      demoResults = new Map(demoResults).set(actionId, `✗ ${e}`);
    }
    const next = new Set(demoRunning);
    next.delete(actionId);
    demoRunning = next;
  }

  function formatWhen(iso: string): string {
    if (!iso) return "";
    const ms = Date.now() - Date.parse(iso);
    if (!Number.isFinite(ms)) return "";
    const mins = Math.floor(ms / 60000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title="Home"
    description="Your current project, recent history, and quick actions."
    icon="home"
  >
    {#snippet actions()}
      <Button variant="secondary" icon="layers" href="/devlauncher">DevLauncher</Button>
      <Button variant="primary" icon="sparkles" href="/create">Project Creator</Button>
    {/snippet}
  </PageHeader>

  {#if loading}
    <LoadingState label="Loading Home…" />
  {:else if error}
    <ErrorState title="Failed to load current project" message={error} retry={reload} />
  {:else}

    <div class="sp-grid">
      <Card
        title="Current project"
        description="Backend getCurrentProject() — the active workspace context."
      >
        {#if project}
          {@const projectPath = project.project_path}
          <div class="sp-project">
            <div class="sp-project-head">
              <h4 class="sp-project-name">{project.profile_name}</h4>
              <Badge tone="violet">current</Badge>
            </div>
            <p class="sp-project-path">{projectPath ?? "—"}</p>
            {#if project.description}
              <p class="sp-project-desc">{project.description}</p>
            {/if}
            {#if project.stack.length > 0}
              <div class="sp-tags">
                {#each project.stack as tech}
                  <Badge tone="blue">{tech}</Badge>
                {/each}
              </div>
            {/if}
            <div class="sp-actions">
              <Button variant="primary" icon="layers" onclick={() => goto("/workspace")}>
                Open Workspace
              </Button>
              {#if projectPath}
                <Button
                  variant="secondary"
                  icon="external"
                  onclick={() => openInVSCodeSafe(projectPath!)}
                >
                  Open in VS Code
                </Button>
              {/if}
            </div>
          </div>
        {:else}
          <p class="sp-none">No project is open.</p>
        {/if}
      </Card>

      <Card
        title="Recent projects"
        description="UI-local history of projects you opened or created. Not the backend current project."
      >
        {#if $recentProjects.length === 0}
          <p class="sp-none">No recent projects yet.</p>
        {:else}
          <div class="sp-recent-list">
            {#each $recentProjects as ref}
              <div class="sp-recent-row">
                <div class="sp-recent-main">
                  <div class="sp-recent-name-row">
                    <strong class="sp-recent-name">{ref.name}</strong>
                    <Badge tone="neutral">{ref.source}</Badge>
                  </div>
                  <span class="sp-recent-path">{ref.path}</span>
                  <span class="sp-recent-when">{formatWhen(ref.at)}</span>
                </div>
                <div class="sp-actions">
                  <Button
                    size="sm"
                    variant="primary"
                    icon="layers"
                    loading={openingPath === ref.path}
                    disabled={openingPath !== null}
                    onclick={() => openRecent(ref)}
                  >
                    Open
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    icon="external"
                    onclick={() => openInVSCodeSafe(ref.path)}
                  >
                    VS Code
                  </Button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </Card>
    </div>

    {#if !project && $recentProjects.length === 0}
      {#snippet emptyAction()}
        <Button variant="primary" icon="layers" href="/devlauncher">Open DevLauncher</Button>
        <Button variant="secondary" icon="sparkles" href="/create">Project Creator</Button>
      {/snippet}

      <div class="sp-empty-wrap">
        <EmptyState
          icon="folder"
          title="Nothing here yet"
          description="Open a project, analyze one to build a launch profile, or create a new project to get started."
          action={emptyAction}
        />
      </div>
    {/if}

    {#if demo}
      <div class="sp-demo">
        <Card
          title="Demo profile (diagnostics)"
          description="Hardcoded backend demo data — not your project. Individual actions run via executeAction; there is no whole-profile launch."
        >
          <div class="sp-demo-head">
            <h4 class="sp-project-name">{demo.name}</h4>
            <Badge tone="amber">demo</Badge>
          </div>
          <p class="sp-project-desc">{demo.description}</p>
          <div class="sp-action-list">
            {#each demo.actions as action}
              <div class="sp-action-row" class:sp-action-disabled={!action.enabled}>
                <span class="sp-action-icon">{actionIcon(action.action_type)}</span>
                <div class="sp-action-info">
                  <span class="sp-action-label">{action.label}</span>
                  <span class="sp-action-detail">
                    {actionTypeLabel(action.action_type)} · {actionSummary(action.action_type)}
                  </span>
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  icon="play"
                  disabled={!action.enabled || demoRunning.has(action.id)}
                  loading={demoRunning.has(action.id)}
                  onclick={() => runDemoAction(action.id)}
                >
                  Execute
                </Button>
              </div>
              {#if demoResults.has(action.id)}
                <div class="sp-action-result {resultClass(demoResults.get(action.id)!)}">
                  {demoResults.get(action.id)}
                </div>
              {/if}
            {/each}
          </div>
        </Card>
      </div>
    {/if}
  {/if}
</PageContainer>

<style>
  .sp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: var(--sp-4);
    margin-bottom: var(--sp-4);
  }

  .sp-project {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-project-head,
  .sp-demo-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-project-name {
    margin: 0;
    font-size: var(--sp-fs-lg);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-project-path {
    margin: 0;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-project-desc {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
  }

  .sp-tags {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .sp-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
    margin-top: var(--sp-1);
  }

  .sp-none {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  .sp-recent-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-recent-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-recent-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-recent-name-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-recent-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-recent-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-recent-when {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-empty-wrap {
    margin-top: var(--sp-4);
  }

  .sp-demo {
    margin-top: var(--sp-6);
  }

  .sp-action-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-top: var(--sp-3);
  }

  .sp-action-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-action-disabled {
    opacity: 0.45;
  }

  .sp-action-icon {
    font-size: var(--sp-fs-md);
    width: 1.4rem;
    text-align: center;
    flex-shrink: 0;
  }

  .sp-action-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .sp-action-label {
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
  }

  .sp-action-detail {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-action-result {
    font-size: var(--sp-fs-xs);
    padding: var(--sp-1) var(--sp-3);
    word-break: break-all;
  }

  .sp-action-result.ok {
    color: var(--sp-success);
  }

  .sp-action-result.err {
    color: var(--sp-danger);
  }

  .sp-action-result.skip {
    color: var(--sp-warning);
  }
</style>
