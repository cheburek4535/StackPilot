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
  import {
    listProfiles,
    executeAction,
  } from "$lib/modules/devlauncher/api";
  import { setCurrentProject } from "$lib/modules/workspace/api";
  import { reloadWorkspaceContext } from "$lib/modules/workspace/context";
  import type { LaunchProfile, LaunchProfileV2, ActionStatus } from "$lib/modules/devlauncher/types";
  import { isV2Profile, isRunTerminal, runStatusLabel } from "$lib/modules/devlauncher/types";
  import * as runStore from "$lib/modules/devlauncher/runStore";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import { notifySuccess, notifyError } from "$lib/core/toasts";

  let profiles = $state<LaunchProfile[]>([]);
  let loading = $state(true);
  let errorMsg = $state("");
  let openingName = $state<string | null>(null);
  let launchingName = $state<string | null>(null);
  let activeRun = $state<import("$lib/modules/devlauncher/types").LaunchRun | null>(null);
  let actionResults = $state<Map<string, string>>(new Map());
  let launchCurrent = $state<string | null>(null);

  onMount(async () => {
    try {
      await runStore.init();
    } catch {
      // Non-critical — profile listing works without the event store.
    }
    await loadProfiles();
  });

  onDestroy(() => {
    runStore.destroy();
  });

  function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(i18n.t("devl.load_timeout") as TranslationKey)), ms);
      p.then(
        (v) => { clearTimeout(timer); resolve(v); },
        (e) => { clearTimeout(timer); reject(e); },
      );
    });
  }

  async function loadProfiles() {
    loading = true;
    errorMsg = "";
    try {
      profiles = await withTimeout(listProfiles(), 10000);
    } catch (e) {
      errorMsg = String(e);
    }
    loading = false;
  }

  /** Открыть рабочее пространство профиля: привязываем проект и уходим
   *  на /workspace — оттуда сразу видны процессы, логи и запуск. */
  async function openWorkspace(profile: LaunchProfile) {
    if (openingName) return;
    openingName = profile.name;
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
      await reloadWorkspaceContext();
      notifySuccess(i18n.t("devl.profile_opened") as TranslationKey, profile.name);
      goto("/workspace");
    } catch (e) {
      notifyError(
        i18n.t("devl.open_workspace_failed", { err: String(e) }) as TranslationKey,
        profile.name,
      );
    }
    openingName = null;
  }

  /** Запуск профиля: V2 — event-driven оркестратор, legacy — executeAction. */
  async function launchProfile(profile: LaunchProfile) {
    if (launchingName) return;
    launchingName = profile.name;
    launchCurrent = null;
    actionResults = new Map();
    activeRun = null;
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
      await reloadWorkspaceContext();
    } catch {
      // Не критично — запуск продолжается без привязки.
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

    launchingName = null;
  }

  async function pollRun(runId: string) {
    try {
      const run = await runStore.fetchRun(runId);
      if (run) {
        activeRun = run;
        if (!isRunTerminal(run.status)) {
          setTimeout(() => pollRun(runId), 1000);
        }
      }
    } catch { /* non-critical */ }
  }

  function formatResultSafe(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }
</script>

<PageContainer width="wide">
  <PageHeader
    title={i18n.t("devl.profiles_title") as TranslationKey}
    description={i18n.t("devl.profiles_hint") as TranslationKey}
    icon="layers"
  >
    {#snippet actions()}
      <Button variant="secondary" size="sm" icon="search" href="/devlauncher/analyze">
        {i18n.t("ws.analyze_project") as TranslationKey}
      </Button>
    {/snippet}
  </PageHeader>

  {#if loading}
    <LoadingState label={i18n.t("devl.profile_loading") as TranslationKey} />
  {:else if errorMsg}
    <div class="sp-error">
      <p>{errorMsg}</p>
      <Button variant="secondary" size="sm" icon="refresh" onclick={loadProfiles}>
        {i18n.t("create.retry") as TranslationKey}
      </Button>
    </div>
  {:else if profiles.length === 0}
    <EmptyState
      icon="layers"
      title={i18n.t("ws.profiles_empty") as TranslationKey}
      description={i18n.t("devl.profiles_hint") as TranslationKey}
    >
      {#snippet action()}
        <Button variant="primary" icon="search" href="/devlauncher/analyze">
          {i18n.t("ws.analyze_project") as TranslationKey}
        </Button>
      {/snippet}
    </EmptyState>
  {:else}
    <Card
      title={i18n.t("devl.profiles_title") as TranslationKey}
      description={i18n.t("devl.profiles_count", { n: profiles.length }) as TranslationKey}
    >
      <div class="sp-profile-list">
        {#each profiles as profile (profile.name)}
          <div class="sp-profile-row">
            <div class="sp-profile-main">
              <div class="sp-profile-name-row">
                <strong class="sp-profile-name">{profile.name}</strong>
                {#if isV2Profile(profile)}
                  <Badge tone="violet">V2</Badge>
                {/if}
                {#if profile.project_path}
                  <Badge tone="cyan">{i18n.t("devl.has_path") as TranslationKey}</Badge>
                {/if}
              </div>
              {#if profile.description}
                <span class="sp-profile-desc">{profile.description}</span>
              {/if}
              {#if profile.project_path}
                <span class="sp-profile-path">{profile.project_path}</span>
              {/if}
            </div>
            <div class="sp-profile-actions">
              <Button
                size="sm"
                variant="secondary"
                icon="bookmark"
                href={`/devlauncher/profiles/${encodeURIComponent(profile.name)}`}
              >
                {i18n.t("ws.manage_profiles") as TranslationKey}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon="folder"
                loading={openingName === profile.name}
                disabled={openingName !== null || launchingName !== null}
                onclick={() => openWorkspace(profile)}
              >
                {i18n.t("devl.open_in_workspace") as TranslationKey}
              </Button>
              <Button
                size="sm"
                variant="primary"
                icon="play"
                loading={launchingName === profile.name}
                disabled={launchingName !== null || openingName !== null}
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

      {#if activeRun}
        <div class="sp-run-card">
          <div class="sp-run-header">
            <h5 style="margin:0">
              {i18n.t("devl.run") as TranslationKey} — {activeRun.profile_name}
            </h5>
            <Badge
              tone={
                activeRun.status === "succeeded"
                  ? "lime"
                  : activeRun.status === "failed"
                    ? "red"
                    : activeRun.status === "cancelled"
                      ? "amber"
                      : activeRun.status === "partial_success"
                        ? "amber"
                        : "violet"
              }
            >
              {runStatusLabel(activeRun.status)}
            </Badge>
          </div>
          {#if !isRunTerminal(activeRun.status)}
            <Button
              variant="danger"
              size="sm"
              icon="x"
              onclick={() => runStore.cancelCurrentRun()}
            >
              {i18n.t("devl.cancel_run") as TranslationKey}
            </Button>
          {/if}
        </div>
      {/if}
    </Card>
  {/if}
</PageContainer>

<style>
  .sp-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-6) var(--sp-4);
    text-align: center;
    color: var(--sp-danger);
  }

  .sp-error p {
    margin: 0;
    font-size: var(--sp-fs-sm);
  }

  .sp-profile-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-profile-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-2);
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

  .sp-profile-name-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }

  .sp-profile-name {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
  }

  .sp-profile-desc {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
  }

  .sp-profile-path {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    word-break: break-all;
  }

  .sp-profile-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex-shrink: 0;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  .sp-launch-current {
    margin: var(--sp-3) 0 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-accent);
  }

  .sp-run-card {
    margin-top: var(--sp-3);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: var(--sp-3) var(--sp-4);
    background: var(--sp-bg-1);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-run-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }
</style>