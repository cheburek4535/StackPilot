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
  import {
    getDemoProfile,
    executeAction,
    listProfiles,
    runProfile,
  } from "$lib/modules/devlauncher/api";
  import type { LaunchProfile } from "$lib/modules/devlauncher/types";
  import type { BadgeTone } from "$lib/components/ui/Badge.svelte";
  import {
    actionIcon,
    actionTypeLabel,
    actionSummary,
    formatResult,
    resultClass,
  } from "$lib/modules/devlauncher/actionMeta";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let project = $state<ProjectContext | null>(null);
  let loading = $state(true);
  let error = $state("");
  let openingPath = $state<string | null>(null);

  let demo = $state<LaunchProfile | null>(null);
  let demoResults = $state<Map<string, string>>(new Map());
  let demoRunning = $state<Set<string>>(new Set());

  let profiles = $state<LaunchProfile[]>([]);
  let launchingProfile = $state<string | null>(null);

  const sourceIcons: Record<string, string> = {
    created: "sparkles",
    open: "folder",
    profile: "layers",
    confirmed: "check",
  };

  const sourceColors: Record<string, BadgeTone> = {
    created: "violet",
    open: "neutral",
    profile: "cyan",
    confirmed: "lime",
  };

  onMount(async () => {
    await Promise.all([loadProject(), loadDemo(), loadProfiles()]);
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
      demo = null;
    }
  }

  async function loadProfiles() {
    try {
      profiles = await listProfiles();
    } catch {
      profiles = [];
    }
  }

  async function openRecent(ref: RecentProjectRef) {
    openingPath = ref.path;
    try {
      await openProject(ref.path);
      notifySuccess(i18n.t("home.open") as TranslationKey, ref.name);
      goto("/workspace");
    } catch (e) {
      notifyError(i18n.t("home.open") as TranslationKey, `${e}`);
    }
    openingPath = null;
  }

  async function openInVSCodeSafe(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess("VS Code", i18n.t("home.open_vscode") as TranslationKey);
    } catch (e) {
      notifyError("VS Code", `${e}`);
    }
  }

  async function quickLaunchProfile(profile: LaunchProfile) {
    if (launchingProfile) return;
    launchingProfile = profile.name;
    try {
      await runProfile(profile);
      notifySuccess(i18n.t("home.launch_profile") as TranslationKey, profile.name);
    } catch (e) {
      notifyError(i18n.t("home.launch_profile") as TranslationKey, `${e}`);
    }
    launchingProfile = null;
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
    if (mins < 1) return i18n.t("home.just_now");
    if (mins < 60) return i18n.t("home.minutes_ago", { n: mins });
    const hours = Math.floor(mins / 60);
    if (hours < 24) return i18n.t("home.hours_ago", { n: hours });
    const days = Math.floor(hours / 24);
    return i18n.t("home.days_ago", { n: days });
  }

  function sourceLabel(source: string): string {
    const key = `home.source_${source}` as TranslationKey;
    return i18n.t(key) || source;
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("home.title") as TranslationKey}
    description={i18n.t("home.description") as TranslationKey}
    icon="home"
  >
    {#snippet actions()}
      <Button variant="secondary" icon="layers" href="/devlauncher">{i18n.t("nav.devlauncher") as TranslationKey}</Button>
      <Button variant="primary" icon="sparkles" href="/create">{i18n.t("nav.project_creator") as TranslationKey}</Button>
    {/snippet}
  </PageHeader>

  {#if loading}
    <LoadingState label={i18n.t("home.loading") as TranslationKey} />
  {:else if error}
    <ErrorState title={i18n.t("home.load_error") as TranslationKey} message={error} retry={reload} />
  {:else}

    <div class="sp-grid">
      <Card
        title={i18n.t("home.current_project") as TranslationKey}
        description={i18n.t("home.current_project_desc") as TranslationKey}
      >
        {#if project}
          {@const projectPath = project.project_path}
          <div class="sp-project">
            <div class="sp-project-head">
              <h4 class="sp-project-name">{project.profile_name}</h4>
              <Badge tone="violet">{i18n.t("home.current") as TranslationKey}</Badge>
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
                {i18n.t("home.open_workspace") as TranslationKey}
              </Button>
              {#if projectPath}
                <Button
                  variant="secondary"
                  icon="external"
                  onclick={() => openInVSCodeSafe(projectPath!)}
                >
                  {i18n.t("home.open_vscode") as TranslationKey}
                </Button>
              {/if}
            </div>
          </div>
        {:else}
          <p class="sp-none">{i18n.t("home.no_project") as TranslationKey}</p>
        {/if}
      </Card>

      <Card
        title={i18n.t("home.recent_projects") as TranslationKey}
        description={i18n.t("home.recent_desc") as TranslationKey}
      >
        {#if $recentProjects.length === 0}
          <p class="sp-none">{i18n.t("home.no_recent") as TranslationKey}</p>
        {:else}
          <div class="sp-recent-list">
            {#each $recentProjects as ref}
              <div class="sp-recent-row">
                <div class="sp-recent-main">
                  <div class="sp-recent-name-row">
                    <strong class="sp-recent-name">{ref.name}</strong>
                    <Badge tone={sourceColors[ref.source] ?? "neutral"}>
                      {sourceLabel(ref.source)}
                    </Badge>
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
                    {i18n.t("home.open") as TranslationKey}
                  </Button>
                  <Button
                    size="sm"
                    variant="secondary"
                    icon="external"
                    onclick={() => openInVSCodeSafe(ref.path)}
                  >
                    {i18n.t("home.vscode") as TranslationKey}
                  </Button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </Card>
    </div>

    {#if profiles.length > 0}
      <Card
        title={i18n.t("devl.saved_profiles") as TranslationKey}
        description={i18n.t("devl.no_profiles_desc") as TranslationKey}
      >
        <div class="sp-profile-grid">
          {#each profiles as profile}
            <div class="sp-profile-card">
              <div class="sp-profile-card-head">
                <h4 class="sp-profile-card-name">{profile.name}</h4>
                <Badge tone="cyan">{profile.actions.length} actions</Badge>
              </div>
              <p class="sp-profile-card-desc">{profile.description}</p>
              {#if profile.project_path}
                <p class="sp-profile-card-path">{profile.project_path}</p>
              {/if}
              <div class="sp-actions">
                <Button
                  size="sm"
                  variant="primary"
                  icon="play"
                  loading={launchingProfile === profile.name}
                  disabled={launchingProfile !== null}
                  onclick={() => quickLaunchProfile(profile)}
                >
                  {i18n.t("home.launch_profile") as TranslationKey}
                </Button>
                {#if profile.project_path}
                  <Button
                    size="sm"
                    variant="secondary"
                    icon="external"
                    onclick={() => openInVSCodeSafe(profile.project_path!)}
                  >
                    {i18n.t("home.vscode") as TranslationKey}
                  </Button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </Card>
    {/if}

    {#if !project && $recentProjects.length === 0}
      {#snippet emptyAction()}
        <Button variant="primary" icon="layers" href="/devlauncher">{i18n.t("home.open_devlauncher") as TranslationKey}</Button>
        <Button variant="secondary" icon="sparkles" href="/create">{i18n.t("home.project_creator") as TranslationKey}</Button>
      {/snippet}

      <div class="sp-empty-wrap">
        <EmptyState
          icon="folder"
          title={i18n.t("home.empty_title") as TranslationKey}
          description={i18n.t("home.empty_desc") as TranslationKey}
          action={emptyAction}
        />
      </div>
    {/if}

    {#if demo}
      <div class="sp-demo">
        <Card
          title={i18n.t("home.demo_title") as TranslationKey}
          description={i18n.t("home.demo_desc") as TranslationKey}
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
                  {i18n.t("home.execute") as TranslationKey}
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

  .sp-profile-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr));
    gap: var(--sp-3);
  }

  .sp-profile-card {
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-profile-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .sp-profile-card-name {
    margin: 0;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-profile-card-desc {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-profile-card-path {
    margin: 0;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
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
