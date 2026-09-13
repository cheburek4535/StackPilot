<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getProfile, getDemoProfile, executeAction, deleteProfile, runProfile, saveProfile, saveProfileV2, getWslState, installWsl } from "$lib/modules/devlauncher/api";
  import { setCurrentProject } from "$lib/modules/workspace/api";
  import type { LaunchProfile, LaunchProfileV2, LaunchAction, LaunchStep, ActionType, ActionStatus, LaunchRun, WslState } from "$lib/modules/devlauncher/types";
  import { isV2Profile, isRunTerminal, runStatusLabel, stepKindIcon, stepKindLabel, stepKindSummary, stepStatusClass, trackingQualityLabel, profileHasDockerSteps } from "$lib/modules/devlauncher/types";
  import { subscribeWslInstallProgress } from "$lib/modules/devlauncher/wsl";
  import { buildStep, stepToAction, deleteStepCascade, emptyAddTemplateDraft, type AddTemplate, type AddTemplateDraft } from "$lib/modules/devlauncher/stepBuilder";
  import { markProfileOpened, shouldShowTerminalHint, markHintShown } from "$lib/modules/devlauncher/onboarding";
  import * as runStore from "$lib/modules/devlauncher/runStore";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import { notifyError, notifySuccess } from "$lib/core/toasts";
  import { getSettings, updateSettings, detectApplications } from "$lib/core/api";
  import type { DetectedApplications } from "$lib/core/types";
  import AppPicker from "$lib/components/ui/AppPicker.svelte";

  let profile = $state<LaunchProfile | null>(null);
  let loading = $state(true);
  let errorMsg = $state("");
  let actionResults = $state<Map<string, string>>(new Map());
  let runningAll = $state(false);
  let currentAction = $state<string | null>(null);

  /** V2 run state. */
  let activeRun = $state<LaunchRun | null>(null);
  /** Whether the profile has been launched (drives visibility of the
   *  "Open in Workspace" button — it stays hidden until the launch). */
  let launched = $state(false);
  let logRunId = $state<string | null>(null);
  let logStepId = $state<string | null>(null);
  let logLines = $state<string[]>([]);
  let logOpen = $state(false);

  let profileName = $derived($page.params.name);
  const isV2 = $derived(profile ? isV2Profile(profile) : false);

  /**
   * Name of the profile currently loaded. SvelteKit reuses this component
   * instance when navigating between /profiles/A and /profiles/B, so the
   * profile must be re-fetched when the param changes — otherwise the page
   * keeps showing the previous profile until a manual refresh.
   */
  let loadedName = $state<string | null>($page.params.name ?? null);

  $effect(() => {
    const name = $page.params.name;
    if (name && name !== loadedName) {
      loadedName = name;
      void loadProfile();
    }
  });

  /** V2 step graph of the loaded profile (present on V2 profiles returned
   *  by the backend). */
  let profileSteps = $derived<LaunchStep[]>(
    (profile as LaunchProfile & { steps?: LaunchStep[] }).steps ?? [],
  );
  let showAddPanel = $state(false);
  let addTpl = $state<AddTemplateDraft>(emptyAddTemplateDraft());
  let savingActions = $state(false);

  /** First-run mini-guide: a highlighted example step the user can add
   *  with one click. Only appears on the first profile page visit after a
   *  profile was actually created (see modules/devlauncher/onboarding.ts). */
  let terminalHintVisible = $state(false);

  /** Run id whose processes are being stopped (drives button feedback). */
  let stoppingRunId = $state<string | null>(null);
  /** True while a cancel request is in flight (drives button feedback). */
  let cancellingRun = $state(false);

  /** Polling handle for the active run (supplements store events). */
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let pollingInFlight = false;

  /** Store subscription: re-sync run state + history on every mutation. */
  let unsubscribeStore: (() => void) | null = null;
  /** Status of this profile's latest run seen by the last sync. */
  let lastSyncedStatus: import("$lib/modules/devlauncher/types").RunStatus | null = null;

  // ---- WSL readiness (Docker on Windows requires WSL2) ----
  let wslState = $state<WslState | null>(null);
  let showWslModal = $state(false);
  /** Modal phase: "confirm" → show warning + install; "installing" → spinner;
   *  "failed" → error; "reboot" → "Готово, перезагрузите ПК". */
  let wslPhase = $state<"confirm" | "installing" | "failed" | "reboot">("confirm");
  let wslStage = $state("");
  let wslError = $state("");
  let wslUnlisten: (() => void) | null = null;

  /** V2 profile view (present only when the loaded profile is V2). */
  const v2Profile = $derived<LaunchProfileV2 | null>(
    isV2 && profile ? (profile as unknown as LaunchProfileV2) : null,
  );
  /** The profile includes docker steps. */
  const hasDockerSteps = $derived(profileHasDockerSteps(v2Profile));
  /** WSL is not installed on this machine (regardless of docker steps) —
   *  drives the always-visible "Download WSL" button. */
  const wslAbsent = $derived(!!wslState && !wslState.present);
  /** Docker steps exist but WSL (their Windows backend) is not installed.
   *  When true the launch is blocked behind the WSL install dialog instead. */
  const wslMissing = $derived(!!hasDockerSteps && wslAbsent);

  /** Load the session-cached WSL state (cheap after the first call). */
  async function refreshWsl() {
    try {
      wslState = await getWslState();
    } catch {
      wslState = null;
    }
  }

  /** Run the WSL install with live stage display; on success the modal
   *  morphs into the "ready — reboot your PC" dialog. */
  async function runWslInstall() {
    wslPhase = "installing";
    wslStage = "";
    wslError = "";
    wslUnlisten?.();
    wslUnlisten = null;
    try {
      wslUnlisten = await subscribeWslInstallProgress((stage) => {
        wslStage = stage;
      });
    } catch {
      wslUnlisten = null;
    }
    try {
      await installWsl();
      wslUnlisten?.();
      wslUnlisten = null;
      await refreshWsl();
      wslPhase = "reboot";
    } catch (e) {
      wslUnlisten?.();
      wslUnlisten = null;
      wslError = String(e);
      wslPhase = "failed";
    }
  }

  /** Open the WSL install dialog (used by the pre-flight gate and by the
   *  always-available "Download WSL" button). */
  function openWslInstall() {
    wslPhase = "confirm";
    wslStage = "";
    wslError = "";
    showWslModal = true;
  }

  /** Close the WSL dialog, keeping the state for later installs. */
  function closeWslInstall() {
    showWslModal = false;
    wslUnlisten?.();
    wslUnlisten = null;
  }

  /** Stop polling once the active run reaches a terminal status. */
  $effect(() => {
    const status = activeRun?.status;
    if (status && isRunTerminal(status)) stopPolling();
  });

  // ---- Application selection (browser / database viewer) ----
  let detectedApps = $state<DetectedApplications | null>(null);
  let appBrowser = $state("");
  let appDbViewer = $state("");
  let appSaving = $state(false);

  let summary = $derived.by(() => {
    if (actionResults.size === 0) return null;
    let ok = 0, err = 0, skip = 0;
    for (const v of actionResults.values()) {
      if (v.startsWith("✓")) ok++;
      else if (v.startsWith("✗")) err++;
      else if (v.startsWith("—")) skip++;
    }
    return { ok, err, skip };
  });

  let failedIds = $derived.by(() => {
    if (!profile || actionResults.size === 0) return [];
    return profile.actions
      .filter((a) => actionResults.get(a.id)?.startsWith("✗"))
      .map((a) => a.id);
  });

  onMount(async () => {
    try {
      await runStore.init();
    } catch {
      // Non-critical — the profile itself loads without the event store.
    }
    // Live-sync run state and history from store mutations (event-driven).
    unsubscribeStore = runStore.subscribe(() => syncRunFromStore());
    await loadProfile();
    // If the user clicked "Run" on a recent mini-card on the Home page, the
    // profile page was opened with ?run=1 — launch the profile here.
    if ($page.url.searchParams.get("run") === "1") {
      await runAll();
    }
    // A recovered still-active run needs polling to stay fresh.
    if (activeRun && !isRunTerminal(activeRun.status)) {
      startPolling(activeRun.run_id);
    }
    void refreshDetectedApps();
    void refreshWsl();
  });

  onDestroy(() => {
    stopPolling();
    unsubscribeStore?.();
    unsubscribeStore = null;
    runStore.destroy();
    wslUnlisten?.();
    wslUnlisten = null;
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

  /** Fetch the profile from the backend without touching loading state. */
  async function fetchProfile() {
    const name = profileName;
    if (!name) return;

    try {
      const demo = await withTimeout(getDemoProfile(), 10000);
      if (demo.name === name) {
        profile = demo;
        // Demo profiles never count as "the user created a profile" —
        // the first-run guide stays hidden for them.
        terminalHintVisible = false;
      } else {
        profile = await withTimeout(getProfile(name), 10000);
        // A real profile was opened: this is the first-run guide's trigger.
        markProfileOpened();
        terminalHintVisible = shouldShowTerminalHint();
      }
    } catch (e) {
      // Only surface load errors on the initial load — a failed silent
      // refresh (e.g. transient IPC error after a run finished) must not
      // replace a page that already shows a valid profile.
      if (!profile) {
        errorMsg = i18n.t("devl.profile_load_failed", { name, err: String(e) }) as TranslationKey;
      }
    }

    await recoverRun();
    refreshHistory();
  }

  async function loadProfile() {
    loading = true;
    errorMsg = "";
    const name = profileName;

    if (!name) {
      errorMsg = i18n.t("devl.profile_not_specified") as TranslationKey;
      loading = false;
      return;
    }

    await fetchProfile();
    loading = false;
  }

  /**
   * Store mutation callback: re-derive the page's run state and history from
   * the store. When this profile's latest run transitions to a terminal
   * state, the profile itself is silently re-fetched so on-disk changes
   * (e.g. steps saved elsewhere) appear without a manual page refresh.
   */
  function syncRunFromStore() {
    if (!profile) return;
    const latest = runStore.getLatestRunForProfile(profile.name);
    if (latest) {
      // Respect a run the user picked from history — don't yank it away.
      if (!activeRun || activeRun.run_id === latest.run_id) {
        activeRun = latest;
      }
      const status = latest.status;
      const wasTerminal = lastSyncedStatus !== null && isRunTerminal(lastSyncedStatus);
      if (isRunTerminal(status) && !wasTerminal) {
        lastSyncedStatus = status;
        void fetchProfile();
        return;
      }
      lastSyncedStatus = status;
    }
    refreshHistory();
  }

  /** Poll a V2 run for updates (fallback: основной источник — события
   *  runStore). 5с + пауза при скрытом окне. */
  function startPolling(runId: string) {
    stopPolling();
    pollTimer = setInterval(() => {
      if (document.hidden) return;
      void (async () => {
        if (pollingInFlight) return;
        pollingInFlight = true;
        try {
          const run = await runStore.fetchRun(runId);
          if (run) {
            activeRun = run;
            if (isRunTerminal(run.status)) stopPolling();
          }
        } catch {
          // Non-critical.
        } finally {
          pollingInFlight = false;
        }
      })();
    }, 5000);
  }

  function stopPolling() {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  /** Map a step id to its display label using the profile's step graph. */
  function stepLabel(stepId: string): string {
    if (!profile || !Array.isArray((profile as any).steps)) return stepId;
    const steps = (profile as any).steps as Array<{ id: string; label?: string }>;
    const found = steps.find((s) => s.id === stepId);
    return found?.label || stepId;
  }

  /** Stop all processes of a run. */
  async function stopRun(runId: string) {
    if (stoppingRunId !== null) return;
    stoppingRunId = runId;
    try {
      await runStore.stopProcesses(runId);
      if (activeRun?.run_id === runId) {
        const updated = await runStore.fetchRun(runId);
        if (updated) activeRun = updated;
      }
      refreshHistory();
      notifySuccess(i18n.t("devl.stop_processes_done") as TranslationKey);
    } catch (error) {
      notifyError(i18n.t("devl.stop_processes") as TranslationKey, String(error));
    } finally {
      stoppingRunId = null;
    }
  }

  /** Recover the latest run for this profile (including finished runs).
   *  Only runs that belong to THIS profile are adopted: a freshly created
   *  project (or any profile without its own run) must NEVER pick up the
   *  active run of another project that is still launching — that would tie
   *  the new project to an unrelated launch (see Project Creator race). */
  async function recoverRun() {
    if (!profile) return;
    const latest = runStore.getLatestRunForProfile(profile.name);
    if (latest) {
      activeRun = latest;
    } else {
      activeRun = null;
    }
    // A still-active run means the profile has really been launched.
    if (activeRun && !isRunTerminal(activeRun.status)) {
      launched = true;
    }
  }

  /** Runs for this profile (history). */
  let historyRuns = $state<LaunchRun[]>([]);

  function refreshHistory() {
    if (!profile) {
      historyRuns = [];
      return;
    }
    historyRuns = runStore
      .getAllRuns()
      .filter((r) => r.profile_name === profile!.name)
      .sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  async function runAction(actionId: string) {
    if (!profile) return;
    const action = profile.actions.find((a) => a.id === actionId);
    if (!action) return;

    try {
      const result = await executeAction(action);
      const msg = formatResult(result);
      actionResults = new Map(actionResults.set(actionId, msg));
    } catch (e) {
      actionResults = new Map(actionResults.set(actionId, `✗ ${e}`));
    }
  }

  // ---- Action set editing (delete / add) ----

  function actionLabel(actionId: string): string {
    if (!profile) return actionId;
    const step = profileSteps.find((s) => s.id === actionId);
    if (step) return step.label;
    const action = profile.actions.find((a) => a.id === actionId);
    return action?.label ?? actionId;
  }

  /** Labels of the steps an action depends on (V2 graph), for display. */
  function dependsLabels(actionId: string): string[] {
    const step = profileSteps.find((s) => s.id === actionId);
    return step?.depends_on?.map((dep) => actionLabel(dep)) ?? [];
  }

  async function saveEditedProfile(
    nextActions: LaunchAction[],
    nextSteps: LaunchStep[] | null,
  ) {
    if (!profile || savingActions) return;
    savingActions = true;
    try {
      if (isV2) {
        const v2: LaunchProfileV2 = {
          schema_version: "2",
          id: profile.id ?? "",
          name: profile.name,
          description: profile.description,
          project_root: profile.project_path ?? null,
          steps: nextSteps ?? [],
          environment_binding_id: profile.environment_binding_id ?? null,
          preferred_ide: profile.preferred_ide ?? null,
        };
        await saveProfileV2(v2);
      } else {
        await saveProfile({ ...profile, actions: nextActions });
      }
      notifySuccess(i18n.t("devl.actions_saved") as TranslationKey);
      await loadProfile();
      showAddPanel = false;
      addTpl = emptyAddTemplateDraft();
    } catch (e) {
      notifyError(i18n.t("devl.save_actions_failed") as TranslationKey, String(e));
    } finally {
      savingActions = false;
    }
  }

  async function removeAction(actionId: string) {
    if (!profile || savingActions) return;
    if (isV2) {
      const { steps, removed } = deleteStepCascade(profileSteps, actionId);
      const cascadeNote =
        removed.length > 1
          ? i18n.t("devl.delete_cascade_note", { n: removed.length - 1 })
          : "";
      if (
        !confirm(
          `${i18n.t("devl.delete_action_confirm", { label: actionLabel(actionId) })}${cascadeNote}`,
        )
      ) {
        return;
      }
      await saveEditedProfile([], steps);
    } else {
      if (!confirm(i18n.t("devl.delete_action_confirm", { label: actionLabel(actionId) }))) {
        return;
      }
      await saveEditedProfile(profile.actions.filter((a) => a.id !== actionId), null);
    }
  }

  function addActionStep(tpl: AddTemplate) {
    if (!profile || savingActions) return;
    const step = buildStep(tpl, addTpl, profile.project_path ?? "");
    if (isV2) {
      void saveEditedProfile([], [...profileSteps, step]);
    } else {
      void saveEditedProfile([...profile.actions, stepToAction(step)], null);
    }
  }

  /** Add the guide's example step ("Open a terminal") and retire the guide. */
  function addHintTerminal() {
    addActionStep({ kind: "terminal_plain" });
    markHintShown();
    terminalHintVisible = false;
  }

  /** Dismiss the guide without adding anything (one-time). */
  function dismissTerminalHint() {
    markHintShown();
    terminalHintVisible = false;
  }

  // ---- Application selection (browser / database viewer) ----

  async function refreshDetectedApps() {
    try {
      detectedApps = await detectApplications();
    } catch {
      detectedApps = null;
    }
    try {
      const s = await getSettings();
      appBrowser = s.browser_path ?? "";
      appDbViewer = s.db_viewer_path ?? "";
    } catch {
      // ignore — pickers stay empty
    }
  }

  async function saveAppSelection(field: "browser_path" | "db_viewer_path", path: string) {
    if (appSaving) return;
    appSaving = true;
    try {
      const s = await getSettings();
      const next = { ...s, [field]: path };
      const saved = await updateSettings(next);
      if (field === "browser_path") appBrowser = saved.browser_path ?? "";
      else appDbViewer = saved.db_viewer_path ?? "";
      notifySuccess(i18n.t("devl.apps_saved") as TranslationKey);
    } catch (e) {
      notifyError(i18n.t("devl.apps_save_failed") as TranslationKey, String(e));
    } finally {
      appSaving = false;
    }
  }

  async function runAll(force = false) {
    if (!profile || runningAll) return;

    // WSL pre-flight: Docker on Windows needs WSL2, and when WSL is not
    // installed the launch can't proceed (Docker Desktop would hang on its
    // own interactive install prompt). Block behind the install dialog; only
    // a real install + reboot makes WSL present.
    if (!force && wslMissing) {
      openWslInstall();
      return;
    }

    runningAll = true;
    activeRun = null;
    launched = true;

    // Bind the profile to the workspace project so the Workspace module
    // (Overview/Runtime/Logs) shows the processes after the launch.
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
    } catch {
      // Non-critical — the launch itself continues.
    }

    if (isV2 && profile.id && profile.schema_version) {
      // V2 path: use event-driven orchestrator.
      try {
        const v2Profile = profile as unknown as import("$lib/modules/devlauncher/types").LaunchProfileV2;
        const run = await runStore.launchRun(v2Profile);
        activeRun = run;
        refreshHistory();
        lastSyncedStatus = run.status;
        // Poll for updates.
        startPolling(run.run_id);
      } catch (e) {
        errorMsg = i18n.t("devl.launch_failed", { err: String(e) }) as TranslationKey;
      }
      runningAll = false;
      return;
    }

    // Legacy path.
    const results = new Map<string, string>();
    try {
      const profileResults = await runProfile(profile);
      for (const [actionId, status] of profileResults) {
        results.set(actionId, formatResult(status));
      }
    } catch (e) {
      for (const action of profile.actions) {
        if (!action.enabled) continue;
        currentAction = action.label;
        try {
          const result = await executeAction(action);
          results.set(action.id, formatResult(result));
        } catch (err) {
          results.set(action.id, `✗ ${err}`);
        }
        actionResults = new Map(results);
      }
    }
    actionResults = new Map(results);
    currentAction = null;
    runningAll = false;
    goto("/workspace/logs");
  }

  async function retryFailed() {
    if (!profile || runningAll) return;
    runningAll = true;
    const results = new Map(actionResults);
    for (const id of failedIds) {
      const action = profile.actions.find((a) => a.id === id);
      if (!action) continue;
      currentAction = action.label;
      try {
        const result = await executeAction(action);
        results.set(action.id, formatResult(result));
      } catch (e) {
        results.set(action.id, `✗ ${e}`);
      }
      actionResults = new Map(results);
    }
    currentAction = null;
    runningAll = false;
  }

  function clearResults() {
    actionResults = new Map();
  }

  /** Cancel the active V2 run. Idempotent. Cancels the run tracked by THIS
   *  page — never a "current" run that may belong to another project. */
  async function cancelRun() {
    if (cancellingRun || !activeRun) return;
    cancellingRun = true;
    try {
      await runStore.cancelCurrentRun(activeRun.run_id);
      const updated = await runStore.fetchRun(activeRun.run_id);
      if (updated) activeRun = updated;
      refreshHistory();
    } catch {
      // The store subscription / polling will surface the real state.
    } finally {
      cancellingRun = false;
    }
  }

  /** View logs for a V2 step. */
  async function viewStepLogs(stepId: string) {
    if (!activeRun) return;
    logRunId = activeRun.run_id;
    logStepId = stepId;
    logOpen = true;
    logLines = [];
    try {
      const { getStepLogs } = await import("$lib/modules/devlauncher/api");
      const logs = await getStepLogs(activeRun.run_id, stepId);
      logLines = [...logs.stdout, ...logs.stderr];
    } catch (e) {
      logLines = [`Failed to load logs: ${e}`];
    }
  }

  /** View combined logs for an entire run. */
  async function viewRunLogs(runId: string) {
    logRunId = runId;
    logStepId = null;
    logOpen = true;
    logLines = [];
    try {
      const { getRunLogs } = await import("$lib/modules/devlauncher/api");
      const logs = await getRunLogs(runId);
      logLines = [...logs.stdout, ...logs.stderr];
    } catch (e) {
      logLines = [`Failed to load logs: ${e}`];
    }
  }

  function closeLogs() {
    logOpen = false;
    logRunId = null;
    logStepId = null;
    logLines = [];
  }

  function formatResult(r: ActionStatus): string {
    if ("Success" in r) return `✓ ${r.Success.message}`;
    if ("Failed" in r) return `✗ ${r.Failed.error}`;
    if ("Skipped" in r) return `— ${r.Skipped.reason}`;
    return "?";
  }

  function actionIcon(act: ActionType): string {
    if ("RunCommand" in act) return "▶";
    if ("OpenUrl" in act) return "🌐";
    if ("OpenApplication" in act) return "⬛";
    if ("WaitForUrl" in act) return "⏳";
    if ("WaitForPort" in act) return "⏳";
    if ("Delay" in act) return "⏱";
    if ("ExecuteScript" in act) return "▶";
    return "?";
  }

  function actionDetail(act: ActionType): string {
    if ("RunCommand" in act) return act.RunCommand.command;
    if ("OpenUrl" in act) return act.OpenUrl.url;
    if ("OpenApplication" in act) return act.OpenApplication.path;
    if ("WaitForUrl" in act) return act.WaitForUrl.url;
    if ("WaitForPort" in act) return `${act.WaitForPort.host}:${act.WaitForPort.port}`;
    if ("Delay" in act) return `${act.Delay.seconds}s`;
    if ("ExecuteScript" in act) return act.ExecuteScript.script;
    return "?";
  }

  function actionTypeLabel(act: ActionType): string {
    if ("RunCommand" in act) return i18n.t("act.command");
    if ("OpenUrl" in act) return i18n.t("act.url");
    if ("OpenApplication" in act) return i18n.t("act.app");
    if ("WaitForUrl" in act) return i18n.t("act.wait_url");
    if ("WaitForPort" in act) return i18n.t("act.wait_port");
    if ("Delay" in act) return i18n.t("act.delay");
    if ("ExecuteScript" in act) return i18n.t("act.script");
    return i18n.t("act.unknown");
  }

  function goBack() {
    goto("/workspace");
  }

  async function handleDelete() {
    if (!profile) return;
    if (!confirm(i18n.t("devl.confirm_delete", { name: profile.name }))) return;
    try {
      await deleteProfile(profile.name);
      goto("/workspace");
    } catch (e) {
      errorMsg = i18n.t("devl.toast_delete_failed", { err: String(e) }) as TranslationKey;
    }
  }

  async function openInWorkspace() {
    if (!profile) return;
    try {
      await setCurrentProject(
        profile.name,
        profile.project_path ?? null,
        profile.description,
        [],
      );
      goto("/workspace");
    } catch (e) {
      errorMsg = i18n.t("devl.open_workspace_failed", { err: String(e) }) as TranslationKey;
    }
  }

  function resultClass(msg: string): string {
    if (msg.startsWith("✓")) return "ok";
    if (msg.startsWith("✗")) return "err";
    if (msg.startsWith("—")) return "skip";
    return "";
  }
</script>

<main>
  <button class="back-btn" onclick={goBack}>{i18n.t("devl.all_profiles") as TranslationKey}</button>

  {#if loading}
    <p class="empty">{i18n.t("devl.profile_loading") as TranslationKey}</p>

  {:else if errorMsg}
    <div class="error-card">
      <p>{errorMsg}</p>
      <button class="secondary" onclick={goBack}>{i18n.t("devl.back_to_list") as TranslationKey}</button>
    </div>

  {:else if profile}
    <div class="profile-header">
      <div>
        <h1>{profile.name}</h1>
        <p class="desc">{profile.description}</p>
      </div>
      <div class="header-actions">
        <button class="danger-outline" onclick={handleDelete} disabled={runningAll}>
          {i18n.t("devl.delete_profile") as TranslationKey}
        </button>
        <button class="secondary" onclick={openInWorkspace}>
          {i18n.t("devl.open_in_workspace") as TranslationKey}
        </button>
        {#if wslAbsent}
          <button class="secondary" onclick={openWslInstall}>
            {i18n.t("devl.wsl_download_button") as TranslationKey}
          </button>
        {/if}
        <button class="primary" onclick={() => runAll()} disabled={runningAll}>
          {runningAll ? (i18n.t("devl.running") as TranslationKey) : (i18n.t("devl.run_all") as TranslationKey)}
        </button>
        {#if activeRun && !isRunTerminal(activeRun.status)}
          <button
            class="danger-outline"
            onclick={cancelRun}
            disabled={stoppingRunId !== null || cancellingRun}
          >
            {cancellingRun
              ? (i18n.t("devl.cancelling") as TranslationKey)
              : (i18n.t("devl.cancel_run") as TranslationKey)}
          </button>
        {:else if activeRun}
          <button
            class="danger-outline"
            onclick={() => stopRun(activeRun!.run_id)}
            disabled={stoppingRunId !== null}
          >
            {stoppingRunId === activeRun.run_id
              ? (i18n.t("devl.stop_processes_running") as TranslationKey)
              : (i18n.t("devl.stop_processes") as TranslationKey)}
          </button>
        {/if}
      </div>
    </div>

    {#if currentAction}
      <p class="running-hint">▶ {currentAction}…</p>
    {/if}

    <!-- V2 Run State -->
    {#if activeRun}
      <section class="run-section">
        <div class="run-header">
          <h2>Run: {activeRun.profile_name}</h2>
          <span class="run-status {activeRun.status}">{runStatusLabel(activeRun.status)}</span>
        </div>
        <div class="run-steps">
          {#each activeRun.steps as step}
            <div class="run-step {stepStatusClass(step.status)}">
              <span class="step-icon">
                {#if step.status === "pending"}○
                {:else if step.status === "running"}◉
                {:else if step.status === "succeeded"}✓
                {:else if step.status === "failed"}✗
                {:else if step.status === "skipped"}—
                {:else if step.status === "cancelled"}⊘
                {:else if step.status === "retrying"}↻
                {:else}?{/if}
              </span>
              <div class="step-info">
                <span class="step-id">{stepLabel(step.step_id)}</span>
                {#if step.error}
                  <span class="step-error">{step.error}</span>
                {/if}
              </div>
              <div class="step-controls">
                {#if (step.status === "running" || step.status === "succeeded") && step.process_id}
                  <button class="small-btn" onclick={() => viewStepLogs(step.step_id)}>
                    Logs
                  </button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
        {#if activeRun.diagnostics.length > 0}
          <div class="run-diagnostics">
            <h3>Diagnostics</h3>
            {#each activeRun.diagnostics as diag}
              <p class="diag {diag.severity === "error" ? "diag-err" : diag.severity === "warning" ? "diag-warn" : "diag-info"}">
                [{diag.source}] {diag.message}
                {#if diag.step_id}
                  <span class="diag-step">(step: {diag.step_id})</span>
                {/if}
              </p>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

    <!-- Docker sign-in note: purely informational. Docker Desktop may show a
         sign-in / terms prompt on first launch — it is optional and does not
         block containers. -->
    {#if hasDockerSteps}
      <p class="docker-signin-note">
        {i18n.t("devl.docker_signin_hint") as TranslationKey}
      </p>
    {/if}

    <!-- Run history -->
    {#if historyRuns.length > 0}
      <section class="run-section">
        <div class="run-header">
          <h2>{i18n.t("devl.run_history") as TranslationKey}</h2>
        </div>
        <div class="run-steps">
          {#each historyRuns as r}
            <div class="run-history-row" class:active={activeRun?.run_id === r.run_id}>
              <button class="history-main" onclick={() => (activeRun = runStore.getRunById(r.run_id) ?? r)}>
                <span class="history-id">#{r.run_id.slice(0, 8)}</span>
                <span class="run-status {r.status}">{runStatusLabel(r.status)}</span>
                <span class="history-time">{new Date(r.created_at).toLocaleString()}</span>
              </button>
              <div class="step-controls">
                {#if !isRunTerminal(r.status)}
                  <button
                    class="small-btn"
                    onclick={() => stopRun(r.run_id)}
                    disabled={stoppingRunId !== null}
                  >
                    {stoppingRunId === r.run_id
                      ? (i18n.t("devl.stop_processes_running") as TranslationKey)
                      : (i18n.t("devl.stop_processes") as TranslationKey)}
                  </button>
                {/if}
                <button class="small-btn" onclick={() => viewRunLogs(r.run_id)}>
                  {i18n.t("devl.logs") as TranslationKey}
                </button>
              </div>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- Legacy Actions (always shown for backward compatibility) -->
    <section>
      <div class="actions-head">
        <h2>{i18n.t("devl.actions", { n: profile.actions.length }) as TranslationKey}</h2>
        <button
          class="add-action-btn"
          disabled={savingActions}
          onclick={() => (showAddPanel = !showAddPanel)}
          title={i18n.t("devl.add_action") as TranslationKey}
        >
          {showAddPanel ? "✕" : "+"}
          <span>{showAddPanel ? (i18n.t("devl.close") as TranslationKey) : (i18n.t("devl.add_action") as TranslationKey)}</span>
        </button>
      </div>

      {#if showAddPanel}
        <div class="template-panel">
          <div class="tpl-row">
            <div class="tpl-body">
              <div class="tpl-name">{i18n.t("devl.tpl.terminal_plain") as TranslationKey}</div>
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "terminal_plain" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">{i18n.t("devl.tpl.terminal_cmd") as TranslationKey}</div>
              <input type="text" placeholder={i18n.t("devl.tpl.cmd_ph") as TranslationKey} bind:value={addTpl.command} />
              <input type="text" placeholder={i18n.t("devl.tpl.wd_ph") as TranslationKey} bind:value={addTpl.workdir} />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "terminal_cmd" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">{i18n.t("devl.tpl.run_command") as TranslationKey}</div>
              <input type="text" placeholder={i18n.t("devl.tpl.cmd_ph") as TranslationKey} bind:value={addTpl.command} />
              <input type="text" placeholder={i18n.t("devl.tpl.wd_ph") as TranslationKey} bind:value={addTpl.workdir} />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "run_command" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">{i18n.t("devl.tpl.open_folder") as TranslationKey}</div>
              <input type="text" placeholder={i18n.t("devl.tpl.path_ph") as TranslationKey} bind:value={addTpl.path} />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "open_folder" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields">
              <div class="tpl-name">{i18n.t("devl.tpl.open_url") as TranslationKey}</div>
              <input type="text" placeholder="https://localhost:3000" bind:value={addTpl.url} />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "open_url" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields tpl-inline">
              <div class="tpl-name">{i18n.t("devl.tpl.wait_port") as TranslationKey}</div>
              <input type="text" placeholder="Host" bind:value={addTpl.host} class="tpl-sm" />
              <input type="number" placeholder="Port" bind:value={addTpl.port} class="tpl-sm" />
              <input type="number" placeholder={i18n.t("devl.tpl.timeout_ph") as TranslationKey} bind:value={addTpl.timeout} class="tpl-sm" />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "wait_port" })}>+</button>
          </div>
          <div class="tpl-row">
            <div class="tpl-body tpl-fields tpl-inline">
              <div class="tpl-name">{i18n.t("devl.tpl.delay") as TranslationKey}</div>
              <input type="number" placeholder={i18n.t("devl.tpl.seconds_ph") as TranslationKey} bind:value={addTpl.seconds} class="tpl-sm" />
            </div>
            <button class="small-btn" onclick={() => addActionStep({ kind: "delay" })}>+</button>
          </div>
        </div>
      {/if}

      <div class="action-list">
        {#each profile.actions as action}
          <div class="action-row" class:disabled={!action.enabled}>
            <span class="action-icon">{actionIcon(action.action_type)}</span>
            <div class="action-info">
              <span class="action-label">{action.label}</span>
              <span class="action-type">{actionTypeLabel(action.action_type)}</span>
              <span class="action-detail">{actionDetail(action.action_type)}</span>
              {#if dependsLabels(action.id).length > 0}
                <span class="action-deps">
                  {i18n.t("devl.after") as TranslationKey} {dependsLabels(action.id).join(", ")}
                </span>
              {/if}
            </div>
            <div class="action-controls">
              <button
                class="run-btn"
                onclick={() => runAction(action.id)}
                disabled={!action.enabled}
                title={i18n.t("devl.execute_tooltip") as TranslationKey}
              >
                ▶
              </button>
              <span class="toggle" class:active={action.enabled}>
                {action.enabled ? (i18n.t("devl.on") as TranslationKey) : (i18n.t("devl.off") as TranslationKey)}
              </span>
              <button
                class="delete-btn"
                disabled={savingActions}
                onclick={() => removeAction(action.id)}
                title={i18n.t("devl.delete_action") as TranslationKey}
              >
                🗑
              </button>
            </div>
          </div>
          {#if actionResults.has(action.id)}
            <div class="result-row {resultClass(actionResults.get(action.id)!)}">
              {actionResults.get(action.id)}
            </div>
          {/if}
        {/each}
      </div>
    </section>

    {#if terminalHintVisible}
      <section class="hint-section">
        <div class="hint-header">
          <p class="hint-text">{i18n.t("devl.hint.intro") as TranslationKey}</p>
          <button
            class="hint-dismiss"
            onclick={dismissTerminalHint}
            title={i18n.t("devl.close") as TranslationKey}
            aria-label={i18n.t("devl.close") as TranslationKey}
          >
            ✕
          </button>
        </div>
        <div class="hint-example">
          <span class="action-icon">▣</span>
          <div class="action-info">
            <span class="action-label">{i18n.t("devl.hint.example_label") as TranslationKey}</span>
            <span class="action-type">{i18n.t("devl.tpl.terminal_plain") as TranslationKey}</span>
            <span class="action-detail">{i18n.t("devl.hint.example_detail") as TranslationKey}</span>
          </div>
          <div class="action-controls">
            <button class="small-btn" onclick={addHintTerminal} title={i18n.t("devl.add_action") as TranslationKey}>
              +
            </button>
          </div>
        </div>
      </section>
    {/if}

    <!-- Application selection -->
    <section>
      <div class="actions-head">
        <h2>{i18n.t("devl.apps_title") as TranslationKey}</h2>
      </div>
      <div class="apps-grid">
        <AppPicker
          apps={detectedApps?.browsers ?? []}
          value={appBrowser}
          onChange={(v) => void saveAppSelection("browser_path", v)}
          title={i18n.t("settings.system.browser")}
          description={i18n.t("settings.apps.browser_desc")}
        />
        <AppPicker
          apps={detectedApps?.db_viewers ?? []}
          value={appDbViewer}
          onChange={(v) => void saveAppSelection("db_viewer_path", v)}
          title={i18n.t("settings.system.db_viewer")}
          description={i18n.t("settings.apps.db_viewer_desc")}
        />
      </div>
    </section>

    {#if summary}
      <div class="summary">
        <span class="summary-ok">✓ {summary.ok}</span>
        <span class="summary-err">✗ {summary.err}</span>
        <span class="summary-skip">— {summary.skip}</span>
        <div class="summary-actions">
          {#if failedIds.length > 0 && !runningAll}
            <button class="secondary" onclick={retryFailed}>{i18n.t("devl.retry_failed") as TranslationKey}</button>
          {/if}
          <button class="secondary" onclick={clearResults}>{i18n.t("devl.clear_results") as TranslationKey}</button>
        </div>
      </div>
    {/if}
  {/if}
</main>

<!-- Log Viewer Modal -->
{#if logOpen}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="modal-overlay" onclick={closeLogs} role="presentation">
    <!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
    <div class="modal-content" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Step logs">
      <div class="modal-header">
        <span>Logs: {logStepId}</span>
        <button class="modal-close" onclick={closeLogs}>✕</button>
      </div>
      <div class="modal-body">
        {#if logLines.length === 0}
          <div class="log-empty">Нет логов для этого шага (процесс не запускался или терминал открыт в отдельном окне).</div>
        {:else}
          {#each logLines as line}
            <div class="log-line">{line}</div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- WSL pre-flight / install modal -->
{#if showWslModal}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="modal-overlay" onclick={closeWslInstall} role="presentation">
    <!-- svelte-ignore a11y_interactive_supports_focus a11y_click_events_have_key_events -->
    <div
      class="modal-content docker-auth-modal"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-label="WSL install"
    >
      <div class="modal-header">
        {#if wslPhase === "reboot"}
          <span>{i18n.t("devl.wsl_reboot_title") as TranslationKey}</span>
        {:else if wslPhase === "installing"}
          <span>{i18n.t("devl.wsl_installing_title") as TranslationKey}</span>
        {:else if wslPhase === "failed"}
          <span>{i18n.t("devl.wsl_install_failed") as TranslationKey}</span>
        {:else}
          <span>{i18n.t("devl.wsl_missing_title") as TranslationKey}</span>
        {/if}
        {#if wslPhase !== "installing"}
          <button class="modal-close" onclick={closeWslInstall}>✕</button>
        {/if}
      </div>
      <div class="docker-auth-modal-body">
        {#if wslPhase === "confirm"}
          <p>{i18n.t("devl.wsl_missing_why") as TranslationKey}</p>
          <p>{i18n.t("devl.wsl_not_installed") as TranslationKey}</p>
          <div class="modal-actions">
            <button class="secondary" onclick={closeWslInstall}>
              {i18n.t("devl.wsl_install_skip") as TranslationKey}
            </button>
            <button
              class="primary"
              onclick={() => {
                void runWslInstall();
              }}
            >
              {i18n.t("devl.wsl_install_action") as TranslationKey}
            </button>
          </div>
        {:else if wslPhase === "installing"}
          <div class="wsl-installing">
            <span class="spinner" aria-hidden="true"></span>
            <p>
              {wslStage || (i18n.t("devl.wsl_installing_stage") as TranslationKey)}
            </p>
          </div>
        {:else if wslPhase === "failed"}
          <p class="error-text">{wslError}</p>
          <div class="modal-actions">
            <button class="secondary" onclick={closeWslInstall}>
              {i18n.t("devl.wsl_install_skip") as TranslationKey}
            </button>
            <button
              class="primary"
              onclick={() => {
                void runWslInstall();
              }}
            >
              {i18n.t("devl.wsl_install_retry") as TranslationKey}
            </button>
          </div>
        {:else}
          <p>{i18n.t("devl.wsl_reboot_body") as TranslationKey}</p>
          <div class="modal-actions">
            <button class="primary" onclick={closeWslInstall}>
              {i18n.t("devl.wsl_reboot_ok") as TranslationKey}
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
    color: var(--sp-text-1);
  }

  .back-btn {
    background: none;
    border: none;
    color: var(--sp-text-3);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    padding: 0;
    margin-bottom: 1rem;
  }
  .back-btn:hover { color: var(--sp-accent); }

  .profile-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 1.5rem;
  }

  .header-actions {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    flex-wrap: wrap;
  }

  .danger-outline {
    background: transparent;
    border: 1px solid var(--sp-danger);
    color: var(--sp-danger);
    padding: 0.45rem 1rem;
    border-radius: var(--sp-radius-md);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
  }
  .danger-outline:hover { background: var(--sp-danger-soft); }
  .danger-outline:disabled { opacity: 0.5; cursor: default; }

  h1 { margin: 0; font-size: var(--sp-fs-xl); color: var(--sp-text-1); }
  .desc { color: var(--sp-text-3); font-size: var(--sp-fs-sm); margin: 0.15rem 0 0; }
  .running-hint { margin: 0 0 1rem; font-size: var(--sp-fs-sm); color: var(--sp-accent); }
  .empty { color: var(--sp-text-3); font-style: italic; }

  .error-card {
    background: var(--sp-danger-soft);
    border: 1px solid var(--sp-danger-border);
    border-radius: var(--sp-radius-md);
    padding: 1.5rem;
    text-align: center;
    color: var(--sp-danger);
  }

  section { margin-bottom: 1.5rem; }
  h2 { font-size: var(--sp-fs-md); margin: 0 0 0.75rem; color: var(--sp-text-2); }

  /* V2 Run section */
  .run-section {
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    padding: 1rem;
    margin-bottom: 1.5rem;
  }

  .run-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.75rem;
  }

  .run-header h2 { margin: 0; }

  .run-status {
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-bold);
    text-transform: uppercase;
    padding: 0.15rem 0.5rem;
    border-radius: var(--sp-radius-xs);
  }
  .run-status.pending { background: var(--sp-bg-2); color: var(--sp-text-2); }
  .run-status.running { background: var(--sp-success-soft); color: var(--sp-success); }
  .run-status.succeeded { background: var(--sp-success-soft); color: var(--sp-success); }
  .run-status.failed { background: var(--sp-danger-soft); color: var(--sp-danger); }
  .run-status.cancelled { background: var(--sp-warning-soft); color: var(--sp-warning); }
  .run-status.partial_success { background: var(--sp-warning-soft); color: var(--sp-warning); }

  .run-steps { display: flex; flex-direction: column; gap: 0.3rem; }

  .run-step {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: var(--sp-radius-sm);
    font-size: var(--sp-fs-xs);
  }

  .run-step.step-pending { opacity: 0.5; }
  .run-step.step-running { background: var(--sp-accent-soft); }
  .run-step.step-succeeded { color: var(--sp-success); }
  .run-step.step-failed { color: var(--sp-danger); background: var(--sp-danger-soft); }
  .run-step.step-skipped { color: var(--sp-warning); opacity: 0.7; }
  .run-step.step-cancelled { color: var(--sp-text-3); text-decoration: line-through; }
  .run-step.step-retrying { color: var(--sp-amber); }

  .run-history-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: var(--sp-radius-sm);
    border: 1px solid var(--sp-border);
    font-size: var(--sp-fs-xs);
  }

  .run-history-row.active {
    border-color: var(--sp-accent-border);
    background: var(--sp-accent-soft);
  }

  .history-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: none;
    border: none;
    color: var(--sp-text-1);
    font-family: var(--sp-font-sans);
    cursor: pointer;
    text-align: left;
    padding: 0;
  }

  .history-id {
    font-family: var(--sp-font-mono);
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
  }

  .history-time {
    font-size: var(--sp-fs-2xs);
    color: var(--sp-text-3);
    margin-left: auto;
  }

  .step-icon { width: 1.2em; text-align: center; flex-shrink: 0; }
  .step-info { flex: 1; min-width: 0; }
  .step-id { font-family: var(--sp-font-mono); }
  .step-error { color: var(--sp-danger); font-size: var(--sp-fs-2xs); display: block; }
  .step-controls { display: flex; gap: 0.3rem; flex-shrink: 0; }

  .small-btn {
    padding: 0.15rem 0.4rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-xs);
    background: transparent;
    color: var(--sp-text-2);
    font-size: var(--sp-fs-2xs);
    cursor: pointer;
    font-family: var(--sp-font-sans);
  }
  .small-btn:hover { background: var(--sp-bg-2); color: var(--sp-text-1); }
  .small-btn:disabled { opacity: 0.5; cursor: default; }
  .small-btn:disabled:hover { background: transparent; color: var(--sp-text-2); }

  .run-diagnostics { margin-top: 0.75rem; padding-top: 0.75rem; border-top: 1px solid var(--sp-border); }
  .run-diagnostics h3 { font-size: var(--sp-fs-sm); margin: 0 0 0.5rem; color: var(--sp-text-2); }

  .diag { margin: 0; font-size: var(--sp-fs-xs); font-family: var(--sp-font-mono); padding: 0.25rem 0.5rem; border-radius: var(--sp-radius-sm); }
  .diag-err { color: var(--sp-danger); background: var(--sp-danger-soft); }
  .diag-warn { color: var(--sp-warning); background: var(--sp-warning-soft); }
  .diag-info { color: var(--sp-text-3); }
  .diag-step { color: var(--sp-text-3); font-size: var(--sp-fs-2xs); }

  /* Legacy actions */
  .action-list { display: flex; flex-direction: column; gap: 0.35rem; }

  .action-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 0.8rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    box-shadow: var(--sp-shadow-1);
  }
  .action-row.disabled { opacity: 0.4; }

  .action-icon { font-size: 1rem; width: 1.4rem; text-align: center; flex-shrink: 0; }

  .action-info { flex: 1; min-width: 0; }
  .action-label { display: block; font-weight: var(--sp-fw-semibold); font-size: var(--sp-fs-sm); color: var(--sp-text-1); }
  .action-type { font-size: var(--sp-fs-xs); color: var(--sp-text-3); text-transform: uppercase; letter-spacing: 0.03em; margin-right: 0.5rem; }
  .action-detail { font-size: var(--sp-fs-xs); color: var(--sp-text-3); word-break: break-all; }

  .action-controls { display: flex; align-items: center; gap: 0.5rem; flex-shrink: 0; }

  .run-btn {
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-bg-2);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    color: var(--sp-success);
    transition: background 0.15s;
  }
  .run-btn:hover:not(:disabled) { background: var(--sp-success-soft); border-color: var(--sp-success); }
  .run-btn:disabled { color: var(--sp-text-3); cursor: default; }

  .toggle { font-size: var(--sp-fs-xs); color: var(--sp-text-3); }
  .toggle.active { color: var(--sp-success); }

  .result-row { font-size: var(--sp-fs-xs); padding: 0.35rem 0.8rem 0.35rem 2.6rem; }
  .result-row.ok { color: var(--sp-success); }
  .result-row.err { color: var(--sp-danger); }
  .result-row.skip { color: var(--sp-warning); }

  /* First-run mini-guide (example step the user can add with one click) */
  .hint-section {
    margin-top: 1.25rem;
    padding: 0.75rem;
    border: 1px dashed var(--sp-accent-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-accent-soft);
  }
  .hint-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.6rem;
  }
  .hint-text {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: 1.45;
  }
  .hint-dismiss {
    flex-shrink: 0;
    padding: 0.1rem 0.4rem;
    border: none;
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    line-height: 1;
  }
  .hint-dismiss:hover { color: var(--sp-text-1); }
  .hint-example {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 0.8rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-accent-border);
    border-radius: var(--sp-radius-md);
    animation: hint-glow 2.6s ease-in-out infinite;
  }
  @keyframes hint-glow {
    0%, 100% { box-shadow: 0 0 0 0 rgba(98, 182, 98, 0); }
    50% { box-shadow: 0 0 14px 1px rgba(98, 182, 98, 0.35); }
  }

  /* Action set editing */
  .actions-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }
  .actions-head h2 { margin: 0; }

  .add-action-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.7rem;
    border: 1px solid var(--sp-accent-border);
    border-radius: var(--sp-radius-sm);
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    font-family: var(--sp-font-sans);
  }
  .add-action-btn:hover:not(:disabled) { background: var(--sp-accent-border); color: #fff; }
  .add-action-btn:disabled { opacity: 0.5; cursor: default; }

  .template-panel {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.75rem;
    margin-bottom: 0.75rem;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }
  .tpl-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.5rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-sm);
  }
  .tpl-body { flex: 1; min-width: 0; }
  .tpl-name { font-size: var(--sp-fs-xs); font-weight: var(--sp-fw-semibold); color: var(--sp-text-1); }
  .tpl-fields { display: flex; flex-direction: column; gap: 0.3rem; }
  .tpl-fields input {
    font-size: var(--sp-fs-xs);
    padding: 0.25rem 0.45rem;
    background: var(--sp-bg-2);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-xs);
    color: var(--sp-text-1);
    font-family: var(--sp-font-mono);
  }
  .tpl-fields.tpl-inline { flex-direction: row; align-items: center; flex-wrap: wrap; }
  .tpl-inline .tpl-sm { width: 90px; }

  .delete-btn {
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--sp-danger-border);
    border-radius: var(--sp-radius-sm);
    background: transparent;
    cursor: pointer;
    font-size: var(--sp-fs-sm);
    color: var(--sp-danger);
  }
  .delete-btn:hover:not(:disabled) { background: var(--sp-danger-soft); }
  .delete-btn:disabled { opacity: 0.5; cursor: default; }

  .action-deps {
    display: block;
    font-size: var(--sp-fs-2xs);
    color: var(--sp-warning);
    margin-top: 0.15rem;
  }

  .apps-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }
  @media (max-width: 720px) {
    .apps-grid { grid-template-columns: 1fr; }
  }

  button.primary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid rgba(0, 0, 0, 0.35);
    background: var(--sp-accent-strong);
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--sp-accent-strong) 72%, black) 0%,
      var(--sp-accent-strong) 45%,
      var(--sp-accent) 100%
    );
    color: #fff;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
    white-space: nowrap;
    box-shadow: var(--sp-gloss-top), var(--sp-shadow-1), 0 2px 14px rgba(228, 87, 10, 0.2);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.55), 0 0 1px rgba(0, 0, 0, 0.4);
    transition: background 0.15s, box-shadow 0.15s;
  }
  button.primary:hover:not(:disabled) { filter: brightness(1.06); }
  button.primary:disabled { opacity: 0.5; cursor: not-allowed; }

  button.secondary {
    padding: 0.5rem 1.2rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    cursor: pointer;
  }
  button.secondary:hover:not(:disabled) { background: var(--sp-bg-3); }

  .summary {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.7rem 1rem;
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    margin-top: 0.5rem;
    font-size: var(--sp-fs-sm);
  }
  .summary-ok { color: var(--sp-success); }
  .summary-err { color: var(--sp-danger); }
  .summary-skip { color: var(--sp-warning); }
  .summary-actions { margin-left: auto; display: flex; gap: 0.4rem; }

  /* Modal */
  .modal-overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,0.55);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
    backdrop-filter: blur(2px);
  }
  .modal-content {
    background: var(--sp-bg-0);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-lg);
    width: 90vw; max-width: 800px;
    max-height: 80vh;
    display: flex; flex-direction: column;
    box-shadow: var(--sp-shadow-3);
    overflow: hidden;
  }
  .modal-header {
    display: flex; justify-content: space-between; align-items: center;
    padding: 0.7rem 1rem;
    border-bottom: 1px solid var(--sp-border);
    font-weight: var(--sp-fw-semibold);
    font-size: var(--sp-fs-sm);
  }
  .modal-close {
    background: none; border: none; color: var(--sp-text-3);
    font-size: 1.2rem; cursor: pointer;
    padding: 0.2rem 0.4rem; border-radius: var(--sp-radius-xs);
  }
  .modal-close:hover { background: var(--sp-bg-2); color: var(--sp-text-1); }
  .modal-body {
    flex: 1; overflow-y: auto; padding: 0.75rem 1rem;
    font-family: var(--sp-font-mono); font-size: var(--sp-fs-xs);
    background: var(--sp-code-bg);
  }

  /* Docker auth pre-flight modal (text UI, not log viewer) */
  .docker-auth-modal .modal-header {
    color: var(--sp-warning);
  }
  .docker-auth-modal-body {
    padding: 1.25rem 1.5rem;
    font-family: var(--sp-font-sans);
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    line-height: 1.55;
  }
  .docker-auth-modal-body p { margin: 0 0 1.25rem; }
  .modal-actions {
    display: flex; justify-content: flex-end; gap: 0.5rem;
  }

  /* WSL install modal */
  .docker-auth-modal .wsl-installing {
    display: flex; align-items: center; gap: 0.75rem;
    padding: 0.25rem 0 0.5rem;
  }
  .wsl-installing .spinner {
    width: 1rem; height: 1rem; flex: none;
    border: 2px solid var(--sp-border-1);
    border-top-color: var(--sp-primary);
    border-radius: 50%;
    animation: wsl-spin 0.8s linear infinite;
  }
  @keyframes wsl-spin {
    to { transform: rotate(360deg); }
  }
  .error-text {
    color: var(--sp-danger);
  }

  /* Optional Docker sign-in note (informational, non-blocking) */
  .docker-signin-note {
    margin: 1rem 0;
    padding: 0.6rem 0.9rem;
    border-left: 3px solid var(--sp-border);
    color: var(--sp-text-2);
    font-size: var(--sp-fs-sm);
    line-height: 1.5;
  }
  .log-line { white-space: pre-wrap; word-break: break-all; line-height: 1.5; }
</style>
