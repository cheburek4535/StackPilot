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
  } from "$lib/modules/devlauncher/api";
  import type {
    LaunchProfile,
    LaunchAction,
    ActionStatus,
  } from "$lib/modules/devlauncher/types";
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
  let profiles = $state<LaunchProfile[]>([]);
  let selectedName = $state<string | null>(null);
  let loading = $state(true);
  let error = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let running = $state<Set<string>>(new Set());
  let deleting = $state(false);
  let launching = $state(false);
  let launchCurrent = $state<string | null>(null);

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

  onMount(async () => {
    await Promise.all([loadProject(), loadProfiles()]);
  });

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

  /** Запуск всего профиля: действия выполняются по очереди, результат
   *  каждого показывается сразу по завершении (бэкенд больше не блокирует
   *  UI, но долгие WaitForPort/Delay всё равно идут последовательно). */
  async function launchProfile(profile: LaunchProfile) {
    if (launching) return;
    launching = true;
    actionResults = new Map();
    try {
      // Привязываем запуск к проекту профиля: процессы попадут в Workspace,
      // а таймер сессии увидит их завершение.
      if (profile.project_path && profile.name !== project?.profile_name) {
        await setCurrentProject(profile.name, profile.project_path, profile.description, []);
        project = await getCurrentProject();
      }
    } catch {
      // не критично — запуск продолжится без привязки
    }
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
    launching = false;
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
      await loadProfiles();
    } catch (e) {
      notifyError(i18n.t("devl.toast_deleted"), i18n.t("devl.toast_delete_failed", { err: String(e) }));
    }
    deleting = false;
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
              onclick={() => {
                selectedName = profile.name;
                actionResults = new Map();
              }}
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
                disabled={launching || selectedProfile.actions.every((a) => !a.enabled)}
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
</style>
