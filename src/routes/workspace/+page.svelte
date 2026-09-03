<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import {
    workspaceContext,
    reloadWorkspaceContext,
  } from "$lib/modules/workspace/context";
  import {
    listProcesses,
    getSessionInfo,
    killProcess,
    setCurrentProject,
    openInVSCode,
  } from "$lib/modules/workspace/api";
  import type { TrackedProcess, SessionInfo } from "$lib/modules/workspace/types";
  import {
    statusTone,
    statusLabel,
    statusIcon,
    formatDuration,
    formatDateTime,
    isProcessRunning,
    isProcessFailed,
  } from "$lib/modules/workspace/status";
  import {
    listProfiles,
    executeAction,
    startFileWatcher,
    stopFileWatcher,
  } from "$lib/modules/devlauncher/api";
  import type {
    LaunchProfile,
    LaunchProfileV2,
    LaunchRun,
    ActionStatus,
  } from "$lib/modules/devlauncher/types";
  import {
    isV2Profile,
    isRunTerminal,
    runStatusLabel,
    stepStatusClass,
  } from "$lib/modules/devlauncher/types";
  import * as runStore from "$lib/modules/devlauncher/runStore";
  import { openProject } from "$lib/core/integration";
  import { recentProjects } from "$lib/core/recent";
  import type { RecentProjectRef } from "$lib/core/recent";
  import { selectFolder } from "$lib/modules/project_creator/api";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let processes = $state<TrackedProcess[]>([]);
  let session = $state<SessionInfo | null>(null);
  let dataLoaded = $state(false);
  let openingPath = $state<string | null>(null);
  let pollId: ReturnType<typeof setInterval> | null = null;

  let profiles = $state<LaunchProfile[]>([]);
  let launching = $state(false);
  let launchingName = $state<string | null>(null);
  let launchCurrent = $state<string | null>(null);
  let activeRun = $state<LaunchRun | null>(null);
  let actionResults = $state<Map<string, string>>(new Map());
  let watching = $state(false);
  /** The profile launcher widget starts collapsed — it lives at the bottom of
   *  the dashboard and expands only on demand or when a launch begins. */
  let profilesExpanded = $state(false);

  const project = $derived($workspaceContext.project);
  const wsLoading = $derived($workspaceContext.loading);
  const wsError = $derived($workspaceContext.error);

  const runningProcs = $derived(processes.filter((p) => isProcessRunning(p.status)));
  const runningCount = $derived(runningProcs.length);
  const erroredCount = $derived(processes.filter((p) => isProcessFailed(p.status)).length);
  const restarts = $derived(processes.reduce((sum, p) => sum + p.restarts, 0));

  /** Recent projects that have a saved DevLauncher profile — only these are
   *  shown, so projects without a launch profile stay out of the Workspace. */
  const profiledRecents = $derived(
    $recentProjects.filter(
      (ref) => profiles.find((p) => p.project_path && p.project_path === ref.path) !== undefined,
    ),
  );

  onMount(async () => {
    try {
      await runStore.init();
    } catch {
      // Non-critical — the dashboard works without the event store.
    }
    await loadProfiles();
    // Deep links: /workspace?profile=<name>[&run=1]
    const want = $page.url.searchParams.get("profile");
    if (want) {
      const found = profiles.find((p) => p.name === want);
      if (found && $page.url.searchParams.get("run") === "1") {
        await launchProfile(found);
      }
    }
  });

  onDestroy(() => {
    stopPolling();
    runStore.destroy();
    if (watching) {
      stopFileWatcher().catch(() => {});
    }
  });

  $effect(() => {
    if (project && !dataLoaded) {
      dataLoaded = true;
      loadData();
      startPolling();
    } else if (!project) {
      dataLoaded = false;
      processes = [];
      session = null;
      stopPolling();
    }
  });

  /** Живое обновление: таймеры (процессы, сессия) считаются на бэкенде,
   *  поэтому страница опрашивает их, пока открыта. */
  function startPolling() {
    if (pollId) return;
    pollId = setInterval(() => {
      loadData();
    }, 2000);
  }

  function stopPolling() {
    if (pollId) {
      clearInterval(pollId);
      pollId = null;
    }
  }

  async function loadData() {
    const [procs, sess] = await Promise.all([
      listProcesses().catch(() => [] as TrackedProcess[]),
      getSessionInfo().catch(() => null),
    ]);
    processes = procs;
    session = sess;
  }

  async function loadProfiles() {
    try {
      profiles = await listProfiles();
    } catch {
      profiles = [];
    }
  }

  function formatResultSafe(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }

  /** Запуск профиля: V2 использует event-driven оркестратор,
   *  legacy — пошаговый executeAction. */
  async function launchProfile(profile: LaunchProfile) {
    if (launching) return;
    launching = true;
    launchingName = profile.name;
    launchCurrent = null;
    actionResults = new Map();
    activeRun = null;
    // The run state is rendered inside the launcher widget — open it so the
    // user sees the run progress without extra clicks.
    profilesExpanded = true;

    try {
      // Bind profile to workspace project.
      if (profile.project_path && profile.name !== project?.profile_name) {
        await setCurrentProject(profile.name, profile.project_path, profile.description, []);
        await reloadWorkspaceContext();
      }
    } catch {
      // Non-critical — launch continues without binding.
    }

    if (isV2Profile(profile) && profile.id && profile.schema_version) {
      try {
        const v2Profile = profile as unknown as LaunchProfileV2;
        const run = await runStore.launchRun(v2Profile);
        activeRun = run;
        pollRun(run.run_id);
      } catch (e) {
        notifyError(i18n.t("devl.launch_failed") as TranslationKey, String(e));
      }
    } else {
      for (const action of profile.actions) {
        if (!action.enabled) continue;
        launchCurrent = action.label;
        try {
          const result = await executeAction(action);
          actionResults = new Map(actionResults).set(action.id, formatResultSafe(result));
        } catch (e) {
          actionResults = new Map(actionResults).set(action.id, `✗ ${e}`);
        }
      }
      launchCurrent = null;
    }

    launching = false;
    launchingName = null;
    // File watcher for live-reload.
    if (profile.project_path) {
      try {
        await startFileWatcher(profile.project_path);
        watching = true;
      } catch {
        // Non-critical.
      }
    }
  }

  /** Сделать профиль текущим проектом рабочего пространства. */
  async function openInWorkspace(profile: LaunchProfile) {
    if (profile.name === project?.profile_name) {
      notifySuccess(i18n.t("ws.toast_project_opened") as TranslationKey, profile.name);
      return;
    }
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
      await reloadWorkspaceContext();
      notifySuccess(i18n.t("ws.toast_project_opened") as TranslationKey, profile.name);
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_project_opened") as TranslationKey,
        i18n.t("ws.toast_open_failed", {
          path: profile.project_path ?? profile.name,
          err: String(e),
        }) as TranslationKey,
      );
    }
  }

  /** Poll a V2 run for updates (supplements event-driven updates). */
  async function pollRun(runId: string) {
    try {
      const run = await runStore.fetchRun(runId);
      if (run) {
        activeRun = run;
        if (!isRunTerminal(run.status) && launching) {
          setTimeout(() => pollRun(runId), 1000);
        }
      }
    } catch {
      // Non-critical.
    }
  }

  /** Cancel the active V2 run. Idempotent — safe after terminal state. */
  async function cancelRun() {
    await runStore.cancelCurrentRun();
    if (activeRun) {
      const updated = await runStore.fetchRun(activeRun.run_id);
      if (updated) activeRun = updated;
    }
  }

  /** Stop all processes of a run without cancelling the run itself. */
  async function stopRun(runId: string) {
    try {
      await runStore.stopProcesses(runId);
      if (activeRun?.run_id === runId) {
        const updated = await runStore.fetchRun(runId);
        if (updated) activeRun = updated;
      }
    } catch { /* non-critical */ }
  }

  async function handleKill(proc: TrackedProcess) {
    try {
      await killProcess(proc.id);
      notifySuccess(i18n.t("ws.toast_killed") as TranslationKey, proc.label);
      loadData();
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_killed") as TranslationKey,
        i18n.t("ws.toast_kill_failed", { err: String(e) }) as TranslationKey,
      );
    }
  }

  async function openRecent(ref: RecentProjectRef) {
    openingPath = ref.path;
    try {
      await openProject(ref.path);
      await reloadWorkspaceContext();
      notifySuccess(i18n.t("ws.toast_project_opened") as TranslationKey, ref.name);
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_project_opened") as TranslationKey,
        i18n.t("ws.toast_open_failed", { path: ref.path, err: String(e) }) as TranslationKey,
      );
    }
    openingPath = null;
  }

  async function openFolder() {
    try {
      const path = await selectFolder();
      if (!path) return;
      await openProject(path);
      await reloadWorkspaceContext();
      notifySuccess(i18n.t("ws.toast_project_opened") as TranslationKey, path);
    } catch (e) {
      notifyError(
        i18n.t("ws.toast_project_opened") as TranslationKey,
        i18n.t("ws.toast_open_failed", { path: "—", err: String(e) }) as TranslationKey,
      );
    }
  }

  async function openInVsCodeSafe(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess("VS Code", i18n.t("ws.toast_opening") as TranslationKey);
    } catch (e) {
      notifyError("VS Code", i18n.t("ws.toast_failed", { err: String(e) }) as TranslationKey);
    }
  }

  function formatWhen(iso: string): string {
    if (!iso) return "";
    const ms = Date.now() - Date.parse(iso);
    if (!Number.isFinite(ms)) return "";
    const mins = Math.floor(ms / 60000);
    if (mins < 1) return i18n.t("time.just_now");
    if (mins < 60) return i18n.t("time.mins_ago", { n: mins });
    const hours = Math.floor(mins / 60);
    if (hours < 24) return i18n.t("time.hours_ago", { n: hours });
    return i18n.t("time.days_ago", { n: Math.floor(hours / 24) });
  }

  function reloadAll() {
    dataLoaded = false;
    processes = [];
    session = null;
    reloadWorkspaceContext();
    loadData();
  }
</script>

<PageContainer width="wide">
  {#if wsLoading}
    <LoadingState label={i18n.t("ws.loading_workspace") as TranslationKey} />
  {:else if wsError}
    <ErrorState
      title={i18n.t("ws.load_failed") as TranslationKey}
      message={wsError}
      retry={reloadAll}
    />
  {:else if !project}
    <div class="sp-empty-wrap">
      {#snippet emptyAction()}
        <Button variant="primary" icon="layers" href="/devlauncher/profiles">
          {i18n.t("devl.profiles") as TranslationKey}
        </Button>
        <Button variant="secondary" icon="folder" onclick={openFolder}>
          {i18n.t("ws.open_folder") as TranslationKey}
        </Button>
        <Button variant="secondary" icon="sparkles" href="/create">
          {i18n.t("ws.open_project_creator") as TranslationKey}
        </Button>
      {/snippet}
      <EmptyState
        icon="folder"
        title={i18n.t("ws.no_project_open") as TranslationKey}
        description={i18n.t("ws.no_project_desc") as TranslationKey}
        action={emptyAction}
      />
    </div>

    {#if profiledRecents.length > 0}
      <div class="sp-recent-section" id="recent-projects">
        <Card
          title={i18n.t("ws.recent_projects") as TranslationKey}
          description={i18n.t("ws.recent_desc") as TranslationKey}
        >
          <div class="sp-recent-list">
            {#each profiledRecents as ref}
              <div class="sp-recent-row">
                <div class="sp-recent-main">
                  <div class="sp-recent-name-row">
                    <strong class="sp-recent-name">{ref.name}</strong>
                    <Badge tone="neutral">{ref.source}</Badge>
                  </div>
                  <span class="sp-recent-path">{ref.path}</span>
                  <span class="sp-recent-when">{formatWhen(ref.at)}</span>
                </div>
                <Button
                  size="sm"
                  variant="primary"
                  icon="layers"
                  loading={openingPath === ref.path}
                  disabled={openingPath !== null}
                  onclick={() => openRecent(ref)}
                >
                  {i18n.t("ws.open") as TranslationKey}
                </Button>
              </div>
            {/each}
          </div>
        </Card>
      </div>
    {/if}
  {:else}
    <PageHeader
      title={i18n.t("ws.dashboard") as TranslationKey}
      description={i18n.t("ws.dashboard_desc") as TranslationKey}
      icon="home"
    />

    <Card variant="elevated" padding="lg">
      <div class="sp-hero">
        <div class="sp-hero-main">
          <div class="sp-hero-head">
            <h2 class="sp-hero-name">{project.profile_name}</h2>
            <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
          </div>
          {#if project.description}
            <p class="sp-hero-desc">{project.description}</p>
          {/if}
          {#if project.project_path}
            <p class="sp-hero-path">{project.project_path}</p>
          {/if}
          {#if project.stack.length > 0}
            <div class="sp-hero-tags">
              {#each project.stack as tech}
                <Badge tone="blue">{tech}</Badge>
              {/each}
            </div>
          {/if}
          <p class="sp-hero-opened">
            {i18n.t("ws.opened", { when: formatDateTime(project.opened_at) }) as TranslationKey}
          </p>
        </div>
        <div class="sp-hero-actions">
          <Button
            variant="primary"
            icon="layers"
            href={`/devlauncher/profiles/${encodeURIComponent(project.profile_name)}`}
          >
            {i18n.t("ws.open_in_devlauncher") as TranslationKey}
          </Button>
          {#if project.project_path}
            <Button
              variant="secondary"
              icon="external"
              onclick={() => openInVsCodeSafe(project!.project_path!)}
            >
              {i18n.t("ws.open_vscode") as TranslationKey}
            </Button>
            <Button
              variant="secondary"
              icon="folder"
              onclick={() => goto("/workspace/files")}
            >
              {i18n.t("ws.browse_files") as TranslationKey}
            </Button>
          {/if}
        </div>
      </div>
    </Card>

    <div class="sp-stats">
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-lime" aria-hidden="true">
          <Icon name="play" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{runningCount}</span>
          <span class="sp-stat-label">{i18n.t("ws.stat_running") as TranslationKey}</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-violet" aria-hidden="true">
          <Icon name="terminal" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{processes.length}</span>
          <span class="sp-stat-label">{i18n.t("ws.stat_total") as TranslationKey}</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-amber" aria-hidden="true">
          <Icon name="refresh" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">{restarts}</span>
          <span class="sp-stat-label">{i18n.t("ws.stat_restarts") as TranslationKey}</span>
        </div>
      </div>
      <div class="sp-stat">
        <span class="sp-stat-icon sp-stat-icon-cyan" aria-hidden="true">
          <Icon name="clock" size={16} />
        </span>
        <div class="sp-stat-text">
          <span class="sp-stat-value">
            {session ? formatDuration(session.duration_secs) : "—"}
          </span>
          <span class="sp-stat-label">{i18n.t("ws.stat_uptime") as TranslationKey}</span>
        </div>
      </div>
    </div>

    <div class="sp-grid">
      <Card
        title={i18n.t("ws.stat_running") as TranslationKey}
        description={i18n.t("ws.overview_desc") as TranslationKey}
      >
        {#if processes.length === 0}
          <div class="sp-inline-empty">
            <p>{i18n.t("ws.no_processes") as TranslationKey}</p>
            <Button
              size="sm"
              variant="secondary"
              icon="terminal"
              href="/workspace/logs"
            >
              {i18n.t("ws.open_runtime") as TranslationKey}
            </Button>
          </div>
        {:else}
          <div class="sp-proc-list">
            {#each processes as p}
              <div class="sp-proc-row">
                <span class="sp-proc-icon" aria-hidden="true">
                  <Icon name={statusIcon(p.status)} size={14} />
                </span>
                <div class="sp-proc-main">
                  <span class="sp-proc-label">{p.label}</span>
                  <span class="sp-proc-meta">
                    {i18n.t("ws.pid", { pid: p.pid }) as TranslationKey} · {formatDuration(p.duration_secs)}
                    {#if p.run_id}
                      · <span class="sp-proc-run">run #{p.run_id.slice(0, 8)}</span>
                    {/if}
                    {#if p.command}
                      · <span class="sp-proc-cmd">{p.command}</span>
                    {/if}
                  </span>
                </div>
                <Badge tone={statusTone(p.status)}>{statusLabel(p.status)}</Badge>
                <Button
                  size="sm"
                  variant="ghost"
                  icon="x"
                  disabled={!isProcessRunning(p.status)}
                  onclick={() => handleKill(p)}
                >
                </Button>
              </div>
            {/each}
          </div>
          {#if erroredCount > 0}
            <p class="sp-proc-warn">
              {i18n.t("ws.errored_count", { n: erroredCount }) as TranslationKey}
            </p>
          {/if}
        {/if}
      </Card>
    </div>

    <div class="sp-grid">
      <Card
        title={i18n.t("ws.session_title") as TranslationKey}
        description={i18n.t("ws.session_desc") as TranslationKey}
      >
        {#if !session}
          <div class="sp-inline-empty">
            <p>{i18n.t("ws.no_session") as TranslationKey}</p>
          </div>
        {:else}
          <div class="sp-session-grid">
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.started") as TranslationKey}</span>
              <span class="sp-session-value">
                {formatDateTime(session.started_at)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.time_total") as TranslationKey}</span>
              <span class="sp-session-value">
                {formatDuration(session.total_duration_secs ?? session.duration_secs)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.time_session") as TranslationKey}</span>
              <span class="sp-session-value">
                {formatDuration(session.duration_secs)}
              </span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.processes") as TranslationKey}</span>
              <span class="sp-session-value">{session.process_count}</span>
            </div>
            <div class="sp-session-item">
              <span class="sp-session-label">{i18n.t("ws.errors") as TranslationKey}</span>
              <span class="sp-session-value" class:sp-session-err={session.error_count > 0}>
                {session.error_count}
              </span>
            </div>
          </div>
        {/if}
      </Card>

      <Card
        title={i18n.t("ws.problems") as TranslationKey}
        description={i18n.t("ws.problems_desc") as TranslationKey}
      >
        <div class="sp-inline-empty">
          <p>
            {erroredCount > 0
              ? i18n.t("ws.errored_count", { n: erroredCount })
              : (i18n.t("ws.no_problems") as TranslationKey)}
          </p>
          <Button size="sm" variant="secondary" icon="alert" href="/workspace/problems">
            {i18n.t("ws.problems") as TranslationKey}
          </Button>
        </div>
      </Card>
    </div>

    <!-- Профили: один сворачиваемый виджет в самом низу, чтобы не
         засорять дашборд. Клик по шапке раскрывает список — каждый профиль
         можно открыть в рабочем пространстве или запустить. -->
    <section class="sp-launch-widget" class:sp-launch-widget-open={profilesExpanded}>
      <button
        class="sp-launch-widget-head"
        onclick={() => (profilesExpanded = !profilesExpanded)}
        aria-expanded={profilesExpanded}
      >
        <span class="sp-launch-widget-heading">
          <span class="sp-launch-widget-title">
            {i18n.t("ws.profile_launcher") as TranslationKey}
          </span>
          <span class="sp-launch-widget-desc">
            {#if profiles.length === 0}
              {i18n.t("ws.profiles_empty") as TranslationKey}
            {:else if !profilesExpanded}
              {i18n.t("ws.profiles_collapsed_hint") as TranslationKey}
            {:else}
              {i18n.t("devl.profiles_count", { n: profiles.length }) as TranslationKey}
            {/if}
          </span>
        </span>
        {#if profiles.length > 0}
          <span class="sp-launch-widget-toggle">
            <span class="sp-launch-widget-count">{profiles.length}</span>
            <Icon name={profilesExpanded ? "chevronUp" : "chevronDown"} size={16} />
          </span>
        {/if}
      </button>

      {#if profiles.length === 0}
        <div class="sp-inline-empty">
          <p>{i18n.t("ws.profiles_empty") as TranslationKey}</p>
          <Button
            size="sm"
            variant="secondary"
            icon="search"
            href="/devlauncher/analyze"
          >
            {i18n.t("ws.analyze_project") as TranslationKey}
          </Button>
        </div>
      {:else if profilesExpanded}
        <div class="sp-launch-widget-body">
          <div class="sp-profiles">
            {#each profiles as profile}
              <div class="sp-profile-row">
                <div class="sp-profile-main">
                  <span class="sp-profile-label">{profile.name}</span>
                  <span class="sp-profile-desc">
                    {profile.description}
                    {#if profile.project_path === project.project_path}
                      · <Badge tone="cyan">{i18n.t("devl.has_path") as TranslationKey}</Badge>
                    {/if}
                  </span>
                  {#if profile.project_path && profile.project_path !== project.project_path}
                    <span class="sp-profile-path">{profile.project_path}</span>
                  {/if}
                </div>
                <div class="sp-profile-actions">
                  <Button
                    size="sm"
                    variant="ghost"
                    icon="folder"
                    disabled={launching}
                    onclick={() => openInWorkspace(profile)}
                  >
                    {i18n.t("devl.open_in_workspace") as TranslationKey}
                  </Button>
                  <Button
                    size="sm"
                    variant="primary"
                    icon="play"
                    loading={launching && launchingName === profile.name}
                    disabled={launching}
                    onclick={() => launchProfile(profile)}
                  >
                    {i18n.t("devl.launch", { name: profile.name }) as TranslationKey}
                  </Button>
                </div>
              </div>
            {/each}
          </div>

          {#if launchCurrent}
            <p class="sp-launch-current">▶ {launchCurrent}…</p>
          {/if}
          {#if actionResults.size > 0}
            <div class="sp-action-results">
              {#each [...actionResults.entries()] as [id, result]}
                <div class="sp-action-result {result.startsWith("✗") ? "err" : result.startsWith("—") ? "skip" : "ok"}">
                  {id}: {result}
                </div>
              {/each}
            </div>
          {/if}
          {#if watching}
            <div class="sp-watcher-status">
              <span class="sp-watcher-dot"></span>
              {i18n.t("devl.watching") as TranslationKey}
            </div>
          {/if}

          {#if activeRun}
            <div class="sp-run-card">
              <div class="sp-run-header">
                <h5 class="sp-sub-title" style="margin:0">
                  {i18n.t("devl.run") as TranslationKey} — {activeRun.profile_name}
                </h5>
                <div class="sp-run-actions">
                  {#if !isRunTerminal(activeRun.status)}
                    <Button variant="danger" size="sm" icon="x" onclick={cancelRun}>
                      {i18n.t("devl.cancel_run") as TranslationKey}
                    </Button>
                  {:else}
                    <Button
                      variant="danger"
                      size="sm"
                      icon="x"
                      onclick={() => stopRun(activeRun!.run_id)}
                    >
                      {i18n.t("devl.stop_processes") as TranslationKey}
                    </Button>
                  {/if}
                  <Badge tone={activeRun.status === "succeeded" ? "lime" : activeRun.status === "failed" ? "red" : activeRun.status === "cancelled" ? "amber" : activeRun.status === "partial_success" ? "amber" : "violet"}>
                    {runStatusLabel(activeRun.status)}
                  </Badge>
                </div>
              </div>
              <div class="sp-run-steps">
                {#each activeRun.steps as step}
                  <div class="sp-step-row {stepStatusClass(step.status)}">
                    <span class="sp-step-status">
                      {#if step.status === "pending"}○
                      {:else if step.status === "running"}◉
                      {:else if step.status === "succeeded"}✓
                      {:else if step.status === "failed"}✗
                      {:else if step.status === "skipped"}—
                      {:else if step.status === "cancelled"}⊘
                      {:else if step.status === "retrying"}↻
                      {:else}?{/if}
                    </span>
                    <span class="sp-step-id">{step.step_id}</span>
                    {#if step.error}
                      <span class="sp-step-error">{step.error}</span>
                    {/if}
                  </div>
                {/each}
              </div>
              {#if activeRun.diagnostics.length > 0}
                <div class="sp-run-diagnostics">
                  {#each activeRun.diagnostics as diag}
                    <p class="sp-diag {diag.severity === "error" ? "sp-diag-err" : diag.severity === "warning" ? "sp-diag-warn" : "sp-diag-info"}">
                      [{diag.source}] {diag.message}
                    </p>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </section>
  {/if}
</PageContainer>

<style>
  .sp-empty-wrap {
    margin: var(--sp-4) 0;
  }

  .sp-recent-section {
    margin-top: var(--sp-4);
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

  /* hero */

  .sp-hero {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .sp-hero-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .sp-hero-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-hero-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-hero-name {
    margin: 0;
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.01em;
    color: var(--sp-text-1);
  }

  .sp-hero-desc {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  .sp-hero-path {
    margin: 0;
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-hero-tags {
    display: flex;
    gap: var(--sp-1);
    flex-wrap: wrap;
  }

  .sp-hero-opened {
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  /* stats */

  .sp-stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
    gap: var(--sp-3);
    margin: var(--sp-4) 0;
  }

  .sp-stat {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-4);
    background: var(--sp-glass-bg);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    box-shadow: var(--sp-shadow-1);
  }

  .sp-stat-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.25rem;
    height: 2.25rem;
    border-radius: var(--sp-radius-md);
    flex-shrink: 0;
  }

  .sp-stat-icon-lime {
    color: var(--sp-success);
    background: rgba(132, 204, 22, 0.12);
    border: 1px solid rgba(132, 204, 22, 0.3);
  }

  .sp-stat-icon-violet {
    color: var(--sp-violet);
    background: rgba(160, 139, 232, 0.12);
    border: 1px solid rgba(160, 139, 232, 0.3);
  }

  .sp-stat-icon-amber {
    color: var(--sp-warning);
    background: rgba(245, 158, 11, 0.12);
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .sp-stat-icon-cyan {
    color: var(--sp-info);
    background: rgba(6, 182, 212, 0.12);
    border: 1px solid rgba(6, 182, 212, 0.3);
  }

  .sp-stat-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .sp-stat-value {
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-stat-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  /* grid */

  .sp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(20rem, 1fr));
    gap: var(--sp-4);
    margin-bottom: var(--sp-4);
  }

  .sp-inline-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-6) var(--sp-4);
    text-align: center;
  }

  .sp-inline-empty p {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
  }

  /* profiles launcher */

  .sp-profiles {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-profile-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-profile-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-profile-label {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-profile-desc {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-profile-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  .sp-launch-current {
    margin: var(--sp-3) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-accent);
  }

  .sp-action-results {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-top: var(--sp-2);
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

  .sp-watcher-status {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--sp-fs-xs);
    color: var(--sp-success);
    margin-top: var(--sp-2);
  }

  .sp-watcher-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--sp-success);
    animation: sp-pulse 2s ease-in-out infinite;
  }

  /* V2 run state */

  .sp-run-card {
    margin-top: var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
  }

  .sp-run-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    margin-bottom: var(--sp-3);
  }

  .sp-run-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-run-steps {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-step-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
    font-family: var(--sp-font-mono);
  }

  .sp-step-row.step-pending { opacity: 0.5; }
  .sp-step-row.step-running { background: var(--sp-accent-soft); }
  .sp-step-row.step-succeeded { color: var(--sp-success); }
  .sp-step-row.step-failed { color: var(--sp-danger); background: rgba(248,113,113,0.08); }
  .sp-step-row.step-skipped { color: var(--sp-warning); opacity: 0.7; }
  .sp-step-row.step-cancelled { color: var(--sp-text-3); text-decoration: line-through; }
  .sp-step-row.step-retrying { color: var(--sp-amber); }

  .sp-step-status {
    width: 1.2em;
    text-align: center;
    flex-shrink: 0;
  }

  .sp-step-id {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-step-error {
    color: var(--sp-danger);
    font-size: var(--sp-fs-2xs);
    max-width: 16rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-run-diagnostics {
    margin-top: var(--sp-2);
    padding-top: var(--sp-2);
    border-top: 1px solid var(--sp-border);
  }

  .sp-diag {
    margin: 0;
    font-size: var(--sp-fs-xs);
    font-family: var(--sp-font-mono);
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--sp-radius-sm);
  }

  .sp-diag-err { color: var(--sp-danger); background: rgba(248,113,113,0.08); }
  .sp-diag-warn { color: var(--sp-warning); background: rgba(251,191,36,0.08); }
  .sp-diag-info { color: var(--sp-text-3); }

  /* processes */

  .sp-proc-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-proc-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-proc-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--sp-success);
    flex-shrink: 0;
  }

  .sp-proc-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .sp-proc-label {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-proc-meta {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-family: var(--sp-font-mono);
  }

  .sp-proc-run {
    color: var(--sp-violet);
  }

  .sp-proc-cmd {
    color: var(--sp-text-2);
    word-break: break-all;
  }

  .sp-proc-warn {
    margin: var(--sp-3) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-warning);
  }

  /* session */

  .sp-session-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--sp-3);
  }

  .sp-session-item {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-session-label {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-session-value {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-session-err {
    color: var(--sp-danger);
  }

  .sp-sub-title {
    margin: var(--sp-5) 0 var(--sp-3);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-2);
  }

  /* profile launcher widget (collapsed by default, bottom of the dashboard) */

  .sp-launch-widget {
    margin: var(--sp-4) 0;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    background: var(--sp-bg-1);
    box-shadow: var(--sp-shadow-2);
    overflow: hidden;
  }

  .sp-launch-widget-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    width: 100%;
    padding: var(--sp-4) var(--sp-5);
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    font-family: var(--sp-font-sans);
    transition: background-color 0.15s ease;
  }

  .sp-launch-widget-head:hover {
    background: var(--sp-bg-2);
  }

  .sp-launch-widget-heading {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-width: 0;
  }

  .sp-launch-widget-title {
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    line-height: var(--sp-lh-tight);
  }

  .sp-launch-widget-desc {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-launch-widget-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    flex: 0 0 auto;
    color: var(--sp-text-3);
  }

  .sp-launch-widget-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.6rem;
    height: 1.6rem;
    padding: 0 var(--sp-1);
    border-radius: var(--sp-radius-full);
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
  }

  .sp-launch-widget-body {
    padding: 0 var(--sp-5) var(--sp-5);
    border-top: 1px solid var(--sp-border-faint);
  }

  .sp-profile-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  @keyframes sp-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
</style>