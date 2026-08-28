<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { listen } from "@tauri-apps/api/event";
  import PageContainer from "$lib/components/ui/PageContainer.svelte";
  import PageHeader from "$lib/components/ui/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import EmptyState from "$lib/components/ui/EmptyState.svelte";
  import LoadingState from "$lib/components/ui/LoadingState.svelte";
  import ErrorState from "$lib/components/ui/ErrorState.svelte";
  import {
    getCurrentProject,
    setCurrentProject,
    openInVSCode,
  } from "$lib/modules/workspace/api";
  import type { ProjectContext } from "$lib/modules/workspace/types";
  import {
    listProfiles,
    deleteProfile,
    executeAction,
    startFileWatcher,
    stopFileWatcher,
  } from "$lib/modules/devlauncher/api";
  import type {
    LaunchProfile,
    LaunchAction,
    ActionStatus,
    FileChangeEvent,
    LaunchRun,
    StepStatus,
  } from "$lib/modules/devlauncher/types";
  import { isV2Profile, isRunTerminal, stepKindIcon, stepKindSummary, stepStatusClass } from "$lib/modules/devlauncher/types";
  import {
    actionIcon,
    actionTypeLabel,
    actionSummary,
    formatResult,
    resultClass,
  } from "$lib/modules/devlauncher/actionMeta";
  import * as runStore from "$lib/modules/devlauncher/runStore";
  import { notifySuccess, notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let project = $state<ProjectContext | null>(null);
  let profiles = $state<LaunchProfile[]>([]);
  let selectedName = $state<string | null>(null);
  let loading = $state(true);
  let error = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let running = $state<Set<string>>(new Set());
  let deleting = $state(false);
  let launching = $state(false);
  let launchCurrent = $state<string | null>(null);
  let watching = $state(false);
  let lastChangedFile = $state<string | null>(null);
  let unlistenFileWatch: (() => void) | null = null;

  /** Active V2 run displayed inline below the launch button. */
  let activeRun = $state<LaunchRun | null>(null);

  const selectedProfile = $derived(
    profiles.find((p) => p.name === selectedName) ?? null,
  );

  const launchSummary = $derived.by(() => {
    if (actionResults.size === 0 || !selectedProfile) return null;
    let ok = 0, err = 0, skip = 0;
    for (const v of actionResults.values()) {
      if (v.startsWith("✓")) ok++;
      else if (v.startsWith("✗")) err++;
      else if (v.startsWith("—")) skip++;
    }
    return { ok, err, skip };
  });

  /** Check if the selected profile is a V2 profile. */
  const isV2 = $derived(selectedProfile ? isV2Profile(selectedProfile) : false);

  /** Runs belonging to the selected profile (history, refreshed explicitly). */
  let historyRuns = $state<LaunchRun[]>([]);

  function refreshHistory() {
    if (!selectedProfile) {
      historyRuns = [];
      return;
    }
    historyRuns = runStore
      .getAllRuns()
      .filter((r) => r.profile_name === selectedProfile!.name)
      .sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  /** Map a step id to its display label using the profile's step graph. */
  function stepLabel(stepId: string): string {
    if (!selectedProfile || !Array.isArray((selectedProfile as any).steps)) return stepId;
    const steps = (selectedProfile as any).steps as Array<{ id: string; label?: string }>;
    const found = steps.find((s) => s.id === stepId);
    return found?.label || stepId;
  }

  onMount(async () => {
    await runStore.init();
    await Promise.all([loadProject(), loadProfiles()]);
    // Recover any active V2 runs.
    await recoverActiveRun();
    // Listen for file change events from the backend watcher
    unlistenFileWatch = await listen<FileChangeEvent>(
      "devlauncher:file_changed",
      (e) => {
        lastChangedFile = e.payload.path;
        // Auto-clear the notification after 4s
        setTimeout(() => {
          if (lastChangedFile === e.payload.path) {
            lastChangedFile = null;
          }
        }, 4000);
      },
    );
  });

  onDestroy(() => {
    unlistenFileWatch?.();
    runStore.destroy();
  });

  /** Recover the latest run for the selected profile (including finished runs). */
  async function recoverActiveRun() {
    if (!selectedProfile) return;
    const latest = runStore.getLatestRunForProfile(selectedProfile.name);
    if (latest) {
      activeRun = latest;
    } else {
      const runs = runStore.getActiveRuns();
      if (runs.length > 0) {
        activeRun = runs[runs.length - 1];
      }
    }
  }

  async function reload() {
    error = "";
    loading = true;
    await Promise.all([loadProject(), loadProfiles()]);
  }

  async function loadProject() {
    try {
      project = await getCurrentProject();
    } catch (e) {
      error = i18n.t("devl.load_project_failed", { err: String(e) });
    }
    loading = false;
  }

  async function loadProfiles() {
    try {
      profiles = await listProfiles();
      if (profiles.length > 0 && !selectedName) {
        selectedName = profiles[0].name;
      } else if (profiles.length === 0) {
        selectedName = null;
      }
      refreshHistory();
    } catch (e) {
      error = i18n.t("devl.load_profiles_failed", { err: String(e) });
    }
  }

  function openWorkspace() {
    goto("/workspace");
  }

  function formatResultSafe(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }

  /** Запуск всего профиля: V2 использует event-driven orchestrator,
   *  legacy — пошаговый executeAction. */
  async function launchProfile(profile: LaunchProfile) {
    if (launching) return;
    launching = true;
    actionResults = new Map();
    activeRun = null;

    try {
      // Bind profile to workspace project.
      if (profile.project_path && profile.name !== project?.profile_name) {
        await setCurrentProject(profile.name, profile.project_path, profile.description, []);
        project = await getCurrentProject();
      }
    } catch {
      // Non-critical — launch continues without binding.
    }

    if (isV2Profile(profile) && profile.id && profile.schema_version) {
      // V2 path: use event-driven orchestrator.
      try {
        const v2Profile = profile as unknown as import("$lib/modules/devlauncher/types").LaunchProfileV2;
        const run = await runStore.launchRun(v2Profile);
        activeRun = run;
        refreshHistory();
        // Subscribe to run updates via polling (event-driven via runStore).
        pollRun(run.run_id);
      } catch (e) {
        notifyError(i18n.t("devl.launch_failed") as TranslationKey, String(e));
      }
    } else {
      // Legacy path: sequential executeAction per action.
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
    // Start file watcher for live-reload.
    if (profile.project_path) {
      try {
        await startFileWatcher(profile.project_path);
        watching = true;
      } catch {
        // Non-critical.
      }
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

  async function openProjectInVSCode(path: string) {
    try {
      await openInVSCode(path);
      notifySuccess(i18n.t("devl.toast_vscode"), i18n.t("devl.toast_vscode_opening"));
    } catch (e) {
      notifyError(i18n.t("devl.toast_vscode"), i18n.t("devl.toast_vscode_failed", { err: String(e) }));
    }
  }

  async function openProfileInWorkspace(profile: LaunchProfile) {
    if (!profile.project_path) return;
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path,
        profile.description,
        [],
      );
      goto("/workspace");
    } catch (e) {
      notifyError(i18n.t("devl.toast_workspace"), i18n.t("devl.toast_workspace_failed", { err: String(e) }));
    }
  }

  async function runAction(profile: LaunchProfile, actionId: string) {
    const action = profile.actions.find((a) => a.id === actionId);
    if (!action) return;
    running = new Set(running).add(actionId);
    try {
      const result = await executeAction(action);
      actionResults = new Map(actionResults).set(actionId, formatResult(result));
    } catch (e) {
      actionResults = new Map(actionResults).set(actionId, `✗ ${e}`);
    }
    const next = new Set(running);
    next.delete(actionId);
    running = next;
  }

  async function handleDelete(profile: LaunchProfile) {
    if (!confirm(i18n.t("devl.confirm_delete", { name: profile.name }))) return;
    deleting = true;
    try {
      await deleteProfile(profile.name);
      notifySuccess(i18n.t("devl.toast_deleted"), profile.name);
      if (selectedName === profile.name) selectedName = null;
      actionResults = new Map();
      activeRun = null;
      // Stop file watcher if this was the active project
      if (watching && profile.project_path) {
        try {
          await stopFileWatcher();
          watching = false;
        } catch { /* non-critical */ }
      }
      await loadProfiles();
    } catch (e) {
      notifyError(i18n.t("devl.toast_deleted"), i18n.t("devl.toast_delete_failed", { err: String(e) }));
    }
    deleting = false;
  }

  /** Cancel the active V2 run. Idempotent — safe after terminal state. */
  async function cancelRun() {
    await runStore.cancelCurrentRun();
    if (activeRun) {
      // Fetch latest state.
      const updated = await runStore.fetchRun(activeRun.run_id);
      if (updated) activeRun = updated;
    }
    refreshHistory();
  }

  /** Stop all processes of a run without cancelling the run itself. */
  async function stopRun(runId: string) {
    try {
      await runStore.stopProcesses(runId);
      if (activeRun?.run_id === runId) {
        const updated = await runStore.fetchRun(runId);
        if (updated) activeRun = updated;
      }
      refreshHistory();
    } catch { /* non-critical */ }
  }

  function selectProfile(name: string) {
    selectedName = name;
    actionResults = new Map();
    activeRun = null;
    const latest = runStore.getLatestRunForProfile(name);
    if (latest) activeRun = latest;
    refreshHistory();
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("devl.title") as TranslationKey}
    description={i18n.t("devl.subtitle") as TranslationKey}
    icon="layers"
  />

  {#if loading}
    <LoadingState label={i18n.t("devl.loading") as TranslationKey} />
  {:else if error}
    <ErrorState title={i18n.t("devl.load_failed") as TranslationKey} message={error} retry={reload} />
  {:else}

    <Card
      title={i18n.t("devl.current_project") as TranslationKey}
      description={i18n.t("devl.current_project_desc") as TranslationKey}
    >
      {#if project}
        {@const projectPath = project.project_path}
        <div class="sp-project">
          <div class="sp-project-head">
            <h4 class="sp-project-name">{project.profile_name}</h4>
            <Badge tone="violet">{i18n.t("devl.current") as TranslationKey}</Badge>
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
            <Button variant="primary" icon="layers" onclick={openWorkspace}>
              {i18n.t("devl.open_workspace") as TranslationKey}
            </Button>
            {#if projectPath}
              <Button
                variant="secondary"
                icon="external"
                onclick={() => openProjectInVSCode(projectPath!)}
              >
                {i18n.t("devl.open_vscode") as TranslationKey}
              </Button>
            {/if}
          </div>
        </div>
      {:else}
        {#snippet emptyAction()}
          <Button icon="bookmark" href="/devlauncher/profiles">{i18n.t("devl.open_profile") as TranslationKey}</Button>
          <Button variant="secondary" icon="search" href="/devlauncher/analyze">
            {i18n.t("devl.analyze_project") as TranslationKey}
          </Button>
        {/snippet}
        <EmptyState
          compact
          icon="folder"
          title={i18n.t("devl.no_project") as TranslationKey}
          description={i18n.t("devl.no_project_desc") as TranslationKey}
          action={emptyAction}
        />
      {/if}
    </Card>

    <div class="sp-section">
      <h3 class="sp-section-title">{i18n.t("devl.saved_profiles") as TranslationKey}</h3>

      {#if profiles.length === 0}
        {#snippet emptyAction()}
          <Button icon="search" href="/devlauncher/analyze">{i18n.t("devl.analyze_project") as TranslationKey}</Button>
        {/snippet}
        <EmptyState
          icon="bookmark"
          title={i18n.t("devl.no_profiles") as TranslationKey}
          description={i18n.t("devl.no_profiles_desc") as TranslationKey}
          action={emptyAction}
        />
      {:else}
        <div class="sp-profile-list">
          {#each profiles as profile}
            <button
              class="sp-profile-row"
              class:sp-profile-row-active={profile.name === selectedName}
              onclick={() => selectProfile(profile.name)}
            >
              <span class="sp-profile-row-main">
                <strong>{profile.name}</strong>
                <span class="sp-profile-row-desc">{profile.description}</span>
              </span>
              <span class="sp-profile-row-meta">
                <Badge tone="neutral">{i18n.t("devl.actions_count", { n: profile.actions.length }) as TranslationKey}</Badge>
                {#if profile.project_path}
                  <Badge tone="cyan">{i18n.t("devl.has_path") as TranslationKey}</Badge>
                {/if}
              </span>
            </button>
          {/each}
        </div>

        {#if selectedProfile}
          <Card padding="md" variant="elevated">
            <div class="sp-launch-head">
              <div class="sp-profile-head">
                <div>
                  <h4 class="sp-profile-name">{selectedProfile.name}</h4>
                  <p class="sp-project-desc">{selectedProfile.description}</p>
                  <p class="sp-project-path">
                    {selectedProfile.project_path ?? (i18n.t("devl.no_path") as TranslationKey)}
                  </p>
                </div>
              </div>
              <Button
                variant="primary"
                size="lg"
                icon="play"
                block
                loading={launching}
                disabled={launching || (isV2 ? false : selectedProfile.actions.every((a) => !a.enabled))}
                onclick={() => launchProfile(selectedProfile!)}
              >
                {launching ? (i18n.t("devl.launching") as TranslationKey) : (i18n.t("devl.launch", { name: selectedProfile.name }) as TranslationKey)}
              </Button>
            </div>
            {#if launchCurrent}
              <p class="sp-launch-current">▶ {launchCurrent}…</p>
            {/if}
            {#if launchSummary}
              <p class="sp-launch-summary">
                ✓ {launchSummary.ok} · ✗ {launchSummary.err} · — {launchSummary.skip}
              </p>
            {/if}
            {#if watching}
              <div class="sp-watcher-status">
                <span class="sp-watcher-dot"></span>
                {i18n.t("devl.watching") as TranslationKey}
              </div>
            {/if}
            {#if lastChangedFile}
              <div class="sp-file-changed">
                📄 {i18n.t("devl.file_changed", { path: lastChangedFile }) as TranslationKey}
              </div>
            {/if}

            <!-- V2 Run State Display -->
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
                    <Badge tone={activeRun.status === "succeeded" ? "lime" : activeRun.status === "failed" ? "red" : activeRun.status === "cancelled" ? "amber" : "violet"}>
                      {activeRun.status}
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
                      <span class="sp-step-id">{stepLabel(step.step_id)}</span>
                      <span class="sp-step-time">
                        {#if step.started_at && step.finished_at}
                          ({step.finished_at})
                        {/if}
                      </span>
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

            <!-- Run history for this profile -->
            {#if historyRuns.length > 0}
              <div class="sp-run-history">
                <h5 class="sp-sub-title" style="margin:0">
                  {i18n.t("devl.run_history") as TranslationKey}
                </h5>
                {#each historyRuns as r}
                  <div class="sp-history-row">
                    <button
                      class="sp-history-main"
                      onclick={() => (activeRun = runStore.getRunById(r.run_id) ?? r)}
                    >
                      <span class="sp-history-id">#{r.run_id.slice(0, 8)}</span>
                      <Badge tone={r.status === "succeeded" ? "lime" : r.status === "failed" ? "red" : r.status === "cancelled" ? "amber" : r.status === "running" ? "violet" : "neutral"}>
                        {r.status}
                      </Badge>
                      <span class="sp-history-time">{new Date(r.created_at).toLocaleString()}</span>
                    </button>
                    <div class="sp-history-actions">
                      {#if !isRunTerminal(r.status)}
                        <Button variant="danger" size="sm" icon="x" onclick={() => stopRun(r.run_id)}>
                          {i18n.t("devl.stop_processes") as TranslationKey}
                        </Button>
                      {/if}
                      <Button
                        variant="ghost"
                        size="sm"
                        icon="terminal"
                        onclick={() => goto(`/devlauncher/processes?log=${r.steps.find((s) => s.process_id)?.process_id ?? ""}`)}
                      >
                        {i18n.t("devl.logs") as TranslationKey}
                      </Button>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}

            <div class="sp-actions sp-launch-actions">
              {#if selectedProfile.project_path}
                <Button
                  variant="secondary"
                  icon="layers"
                  onclick={() => openProfileInWorkspace(selectedProfile!)}
                >
                  {i18n.t("devl.open_in_workspace") as TranslationKey}
                </Button>
              {/if}
              <Button
                variant="danger"
                icon="trash"
                disabled={deleting || launching}
                onclick={() => handleDelete(selectedProfile!)}
              >
                {i18n.t("devl.delete") as TranslationKey}
              </Button>
            </div>

            <h5 class="sp-sub-title">
              {i18n.t("devl.actions", { n: selectedProfile.actions.length }) as TranslationKey}
            </h5>
            <div class="sp-action-list">
              {#each selectedProfile.actions as action}
                <div class="sp-action-row" class:sp-action-disabled={!action.enabled}>
                  <span class="sp-action-icon">{actionIcon(action.action_type)}</span>
                  <div class="sp-action-info">
                    <span class="sp-action-label">{action.label}</span>
                    <span class="sp-action-detail">
                      {actionTypeLabel(action.action_type)} · {actionSummary(action.action_type)}
                    </span>
                  </div>
                  <Badge tone={action.enabled ? "lime" : "neutral"}>
                    {action.enabled ? (i18n.t("devl.on") as TranslationKey) : (i18n.t("devl.off") as TranslationKey)}
                  </Badge>
                  <Button
                    size="sm"
                    variant="primary"
                    icon="play"
                    disabled={!action.enabled || running.has(action.id) || launching}
                    loading={running.has(action.id)}
                    onclick={() => runAction(selectedProfile!, action.id)}
                  >
                    {i18n.t("devl.execute") as TranslationKey}
                  </Button>
                </div>
                {#if actionResults.has(action.id)}
                  <div class="sp-action-result {resultClass(actionResults.get(action.id)!)}">
                    {actionResults.get(action.id)}
                  </div>
                {/if}
              {/each}
            </div>
          </Card>
        {/if}
      {/if}
    </div>
  {/if}
</PageContainer>

<style>
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
    margin-top: var(--sp-2);
  }

  .sp-section {
    margin-top: var(--sp-8);
  }

  .sp-section-title {
    margin: 0 0 var(--sp-4);
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-profile-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin-bottom: var(--sp-4);
  }

  .sp-profile-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    width: 100%;
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    text-align: left;
    cursor: pointer;
    transition:
      background-color 0.15s ease,
      border-color 0.15s ease;
  }

  .sp-profile-row:hover {
    background: var(--sp-bg-2);
  }

  .sp-profile-row-active {
    background: var(--sp-accent-soft);
    border-color: var(--sp-accent-border);
  }

  .sp-profile-row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .sp-profile-row-desc {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-profile-row-meta {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  .sp-profile-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--sp-4);
  }

  .sp-launch-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-6);
    flex-wrap: wrap;
    margin-bottom: var(--sp-3);
  }

  .sp-launch-head .sp-btn-lg {
    min-width: 16rem;
    font-size: var(--sp-fs-lg);
    padding: var(--sp-3) var(--sp-6);
  }

  .sp-launch-actions {
    margin-top: var(--sp-2);
  }

  .sp-launch-current {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-accent);
  }

  .sp-launch-summary {
    margin: var(--sp-1) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-profile-name {
    margin: 0;
    font-size: var(--sp-fs-md);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-sub-title {
    margin: var(--sp-5) 0 var(--sp-3);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-2);
  }

  .sp-action-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
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

  .sp-file-changed {
    font-size: var(--sp-fs-xs);
    color: var(--sp-accent);
    margin-top: var(--sp-1);
    padding: var(--sp-1) var(--sp-2);
    background: var(--sp-accent-soft);
    border-radius: var(--sp-radius-sm);
  }

  /* V2 Run state */
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

  .sp-step-time {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-2xs);
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

  .sp-run-history {
    margin-top: var(--sp-3);
    border-top: 1px solid var(--sp-border);
    padding-top: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-history-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
  }

  .sp-history-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    background: none;
    border: none;
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    cursor: pointer;
    text-align: left;
    padding: var(--sp-1) 0;
  }

  .sp-history-id {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
  }

  .sp-history-time {
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
    margin-left: auto;
  }

  .sp-history-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
  }

  @keyframes sp-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
</style>
