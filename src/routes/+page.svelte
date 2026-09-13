<script lang="ts">
  import { onMount, onDestroy } from "svelte";
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
  import { listProfiles, listActiveRuns } from "$lib/modules/devlauncher/api";
  import type { LaunchProfile, LaunchRun } from "$lib/modules/devlauncher/types";
  import { getHealthReport } from "$lib/modules/toolchain/api";
  import type { HealthReport } from "$lib/modules/toolchain/types";
  import { selectFolder } from "$lib/modules/project_creator/api";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import { HINT_HOME_START } from "$lib/core/help";
  import HelpHint from "$lib/components/ui/HelpHint.svelte";

  let project = $state<ProjectContext | null>(null);
  let loading = $state(true);
  let error = $state("");
  let openingPath = $state<string | null>(null);

  let profiles = $state<LaunchProfile[]>([]);
  let health = $state<HealthReport | null>(null);
  let healthError = $state(false);
  let activeRuns = $state<LaunchRun[]>([]);
  let runsPollId: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    await Promise.all([loadProject(), loadProfiles(), loadHealth(), loadActiveRuns()]);
    // 5с + пауза при скрытом окне: активные раннеры и так обновляются
    // событиями runStore, поллинг — только страховка.
    runsPollId = setInterval(() => {
      if (document.hidden) return;
      loadActiveRuns();
    }, 5000);
  });

  onDestroy(() => {
    if (runsPollId) {
      clearInterval(runsPollId);
      runsPollId = null;
    }
  });

  async function loadActiveRuns() {
    try {
      activeRuns = await listActiveRuns();
    } catch {
      // Non-critical — badges fall back to no "running" state.
    }
  }

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

  async function loadProfiles() {
    try {
      profiles = await listProfiles();
    } catch {
      profiles = [];
    }
  }

  async function loadHealth() {
    try {
      health = await getHealthReport();
      healthError = false;
    } catch {
      health = null;
      healthError = true;
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

  async function openFolder() {
    try {
      const path = await selectFolder();
      if (!path) return;
      await openProject(path);
      notifySuccess(i18n.t("home.open") as TranslationKey, path);
      goto("/workspace");
    } catch (e) {
      notifyError(i18n.t("home.open") as TranslationKey, `${e}`);
    }
  }

  async function openInVSCodeSafe(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess("VS Code", i18n.t("home.open_vscode") as TranslationKey);
    } catch (e) {
      notifyError("VS Code", `${e}`);
    }
  }

  /** Match a recent project reference to a saved launch profile. */
  function profileForRef(ref: RecentProjectRef): LaunchProfile | null {
    return (
      profiles.find((p) => p.project_path && p.project_path === ref.path) ??
      profiles.find((p) => p.name === ref.name) ??
      null
    );
  }

  /** Launch the profile from the Workspace dashboard (deep link). */
  function runProfileFromRecent(profile: LaunchProfile) {
    goto(`/workspace?profile=${encodeURIComponent(profile.name)}&run=1`);
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

  /** Live project status — only two states are ever shown:
   *  "running" (active DevLauncher run) and "open" (current workspace project).
   *  Everything else shows no badge at all. */
  function refStatus(ref: RecentProjectRef): "running" | "open" | null {
    const matched = profileForRef(ref);
    if (
      matched &&
      activeRuns.some((r) => r.profile_name === matched.name)
    ) {
      return "running";
    }
    if (
      project &&
      (ref.path === project.project_path ||
        (matched !== null && matched.name === project.profile_name))
    ) {
      return "open";
    }
    return null;
  }

  const healthOk = $derived(health !== null && health.tools.length > 0 && health.tools.every((t) => t.ok));
  const healthFailing = $derived(health !== null && health.tools.some((t) => !t.ok));

  /** Recent projects that have a saved DevLauncher profile — only these are
   *  shown, so projects without a launch profile stay out of Home. */
  const visibleRecents = $derived(
    $recentProjects.filter((ref) => profileForRef(ref) !== null),
  );

  const statusBadge = $derived<Record<string, { tone: "lime" | "cyan"; label: string }>>({
    running: { tone: "lime", label: i18n.t("home.status_running") as TranslationKey },
    open: { tone: "cyan", label: i18n.t("home.status_open") as TranslationKey },
  });
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("home.title") as TranslationKey}
    description={i18n.t("home.description") as TranslationKey}
    icon="home"
  >
    {#snippet actions()}
      {#if project}
        <Button variant="primary" icon="layers" href="/workspace">{i18n.t("home.resume_workspace") as TranslationKey}</Button>
      {/if}
      <Button variant="secondary" icon="sparkles" href="/create">{i18n.t("nav.project_creator") as TranslationKey}</Button>
    {/snippet}
  </PageHeader>

  <HelpHint
    id={HINT_HOME_START.id}
    resolvedBy={HINT_HOME_START.resolvedBy}
    icon="rocket"
    title={i18n.t("help.home.title") as TranslationKey}
    text={i18n.t("help.home.body") as TranslationKey}
  />

  {#if loading}
    <LoadingState label={i18n.t("home.loading") as TranslationKey} />
  {:else if error}
    <ErrorState title={i18n.t("home.load_error") as TranslationKey} message={error} retry={reload} />
  {:else}

    {#if project}
      {@const projectPath = project.project_path}
      <Card
        variant="elevated"
        padding="lg"
        title={i18n.t("home.current_project") as TranslationKey}
        description={i18n.t("home.hero_desc") as TranslationKey}
      >
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
            <Button variant="primary" icon="layers" href="/workspace">
              {i18n.t("home.resume_workspace") as TranslationKey}
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
      </Card>
    {/if}

    <div class="sp-grid">
      <Card
        title={i18n.t("home.recent_projects") as TranslationKey}
        description={i18n.t("home.recent_desc") as TranslationKey}
      >
        {#if visibleRecents.length === 0}
          <p class="sp-none">{i18n.t("home.no_recent") as TranslationKey}</p>
        {:else}
          <div class="sp-recent-list" id="recent">
            {#each visibleRecents.slice(0, 5) as ref}
              {@const matched = profileForRef(ref)}
              <div class="sp-recent-row">
                <div class="sp-recent-main">
                  <div class="sp-recent-name-row">
                    <strong class="sp-recent-name">{ref.name}</strong>
                    {#if refStatus(ref)}
                      {@const badge = statusBadge[refStatus(ref)!]}
                      <Badge tone={badge.tone}>{badge.label}</Badge>
                    {/if}
                  </div>
                  <span class="sp-recent-path">{ref.path}</span>
                  <span class="sp-recent-when">{formatWhen(ref.at)}</span>
                </div>
                <div class="sp-actions">
                  {#if matched}
                    <Button
                      size="sm"
                      variant="primary"
                      icon="play"
                      onclick={() => runProfileFromRecent(matched)}
                    >
                      {i18n.t("home.run") as TranslationKey}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      icon="bookmark"
                      href={`/devlauncher/profiles/${encodeURIComponent(matched.name)}`}
                    >
                      {i18n.t("home.open_profile") as TranslationKey}
                    </Button>
                  {:else}
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
                  {/if}
                  <Button
                    size="sm"
                    variant="ghost"
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

      <Card
        title={i18n.t("home.diagnostics_title") as TranslationKey}
        description={i18n.t("home.diagnostics_desc") as TranslationKey}
      >
        <div class="sp-diagnostics">
          <div class="sp-diag-row">
            <Badge tone={healthOk ? "lime" : healthFailing ? "amber" : "neutral"}>
              {healthOk
                ? (i18n.t("home.toolchain_ready") as TranslationKey)
                : healthFailing
                  ? (i18n.t("home.toolchain_error") as TranslationKey)
                  : (i18n.t("home.toolchain_unknown") as TranslationKey)}
            </Badge>
            {#if health && health.tools.length > 0}
              <span class="sp-diag-detail">
                {health.tools.filter((t) => t.ok).length}/{health.tools.length}
              </span>
            {:else if healthError}
              <span class="sp-diag-detail">—</span>
            {/if}
          </div>
          <p class="sp-diag-count">
            {i18n.t("home.profiles_count", { n: profiles.length }) as TranslationKey}
          </p>
          {#if profiles.length === 0}
            <p class="sp-none">{i18n.t("home.no_profiles_yet") as TranslationKey}</p>
          {/if}
          <div class="sp-actions">
            <Button size="sm" variant="secondary" icon="wrench" href="/toolchain">
              {i18n.t("home.view_toolchain") as TranslationKey}
            </Button>
            <Button size="sm" variant="ghost" icon="bookmark" href="/workspace">
              {i18n.t("home.manage_profiles") as TranslationKey}
            </Button>
          </div>
        </div>
      </Card>
    </div>

    {#if !project && visibleRecents.length === 0}
      {#snippet emptyAction()}
        <Button variant="primary" icon="folder" onclick={openFolder}>{i18n.t("home.open_folder") as TranslationKey}</Button>
        <Button variant="secondary" icon="sparkles" href="/create">{i18n.t("home.new_project") as TranslationKey}</Button>
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
  {/if}
</PageContainer>

<style>
  .sp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: var(--sp-4);
    margin-top: var(--sp-4);
  }

  .sp-project {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-project-head {
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

  .sp-diagnostics {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-diag-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-diag-detail {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-diag-count {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
  }
</style>