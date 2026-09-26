<script lang="ts">
  import { createProjectStore, setProjectStore } from "$lib/modules/project_creator/createStore.svelte";
  import { onMount, onDestroy } from "svelte";
  
          import { availableLocales } from "$lib/core/i18n.svelte";

  
    
  import type { FrameworkDef } from "$lib/modules/project_creator/types";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import WizardConstructor from "$lib/components/project-creator/WizardConstructor.svelte";
  
  const store = createProjectStore();
  setProjectStore(store);

  
  
  
</script>










<div class="wizard">
  <h1>{i18n.t("create.title") as TranslationKey}</h1>

  {#if store.status === "loading"}
    <div class="skeleton-wrap" aria-busy="true" aria-label={i18n.t("create.init") as TranslationKey}>
      <div class="skeleton-header">
        <div class="skeleton-bar skeleton-bar--title"></div>
      </div>
      <div class="skeleton-mode-switch">
        <div class="skeleton-pill"></div>
        <div class="skeleton-pill"></div>
        <div class="skeleton-pill"></div>
      </div>
      <div class="skeleton-builder">
        <div class="skeleton-phases">
          <div class="skeleton-phase"></div>
          <div class="skeleton-phase"></div>
          <div class="skeleton-phase"></div>
        </div>
        <div class="skeleton-content">
          <div class="skeleton-bar skeleton-bar--subtitle"></div>
          <div class="skeleton-grid">
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
            <div class="skeleton-card"></div>
          </div>
        </div>
        <div class="skeleton-sidebar">
          <div class="skeleton-bar skeleton-bar--sidebar"></div>
          <div class="skeleton-bar skeleton-bar--sidebar-short"></div>
        </div>
      </div>
    </div>
  {:else if store.status === "error"}
    <p class="error">{i18n.t("create.init_failed") as TranslationKey}</p>
  {:else if store.status === "empty"}
    <p class="muted">{i18n.t("create.no_types") as TranslationKey}</p>
  {:else}

    <div class="mode-switch">
      <button class="mode-btn" class:active={store.mode === "constructor"} onclick={() => { store.mode = "constructor"; }}>{i18n.t("create.mode.constructor") as TranslationKey}</button>
      <button class="mode-btn" class:active={store.mode === "presets"} onclick={() => { store.mode = "presets"; }}>{i18n.t("create.mode.templates") as TranslationKey}</button>
      <!-- Анализ временно скрыт: чтобы вернуть — раскомментируйте кнопку и
           строку AnalyzeMode в Promise.all в onMount. -->
      <!-- <button class="mode-btn" class:active={store.mode === "analyze"} onclick={() => { store.mode = "analyze"; }}>{i18n.t("create.mode.analyze") as TranslationKey}</button> -->
    </div>

    {#if store.mode === "analyze"}
      {#if store.AnalyzeMode}
        <store.AnalyzeMode
          analyzing={store.analyzing}
          analyzedPath={store.analyzedPath}
          analysisError={store.analysisError}
          analysisResult={store.analysisResult}
          onrun={store.runAnalysis}
          onapply={store.applyAnalysis}
        />
      {/if}

    {:else if store.mode === "presets"}
      {#if store.PresetsMode}
        <store.PresetsMode tree={store.tree} onapply={store.applyPreset} />
      {/if}

    {:else}

      {#if store.phase === 5 || store.phase === 6}
        <!-- ================================================================
             Environment check & install
             ================================================================ -->
        {#if store.phase === 5}
          {#if store.EnvPanel}
            <store.EnvPanel
              tree={store.tree}
              envCheck={store.envCheck}
              envChecking={store.envChecking}
              envCheckProgress={store.envCheckProgress}
              envSelectedIds={store.envSelectedIds}
              envLocalInfra={store.envLocalInfra}
              envPlan={store.envPlan}
              envInstalling={store.envInstalling}
              envInstallDone={store.envInstallDone}
              envErrors={store.envErrors}
              envLogs={store.envLogs}
              envTaskStates={store.envTaskStates}
              envRestartHint={store.envRestartHint}
              envDownload={store.envDownload}
              envSpeed={store.envSpeed}
              envPhaseStart={store.envPhaseStart}
              envNow={store.envNow}
              envRechecking={store.envRechecking}
              envError={store.envError}
              newSecrets={store.newSecrets}
              secretCopied={store.secretCopied}
              installedTools={store.installedTools}
              ontoggleEnvTool={store.toggleEnvTool}
              onselectAll={store.selectAllEnvTools}
              onoptInLocalInfra={store.optInLocalInfra}
              onrevertLocalInfra={store.revertLocalInfra}
              onstartInstall={store.startInstall}
              oncancelInstall={store.cancelInstall}
              onrecheck={store.recheckEnvironment}
              oncheck={() => store.runEnvironmentCheck()}
              oncontinue={store.doCreateProject}
              onback={store.back}
              onbackReview={() => (store.phase = 2)}
              oncopySecret={store.copySecret}
              ondismissSecrets={() => (store.newSecrets = null)}
            />
          {/if}
        {/if}

        <!-- ================================================================
             Execution
             ================================================================ -->
        {#if store.phase === 6}
          {#if store.ExecPanel}
            <store.ExecPanel
              execPlan={store.execPlan}
              execProjectPath={store.execProjectPath}
              execStatuses={store.execStatuses}
              execOverallStatus={store.execOverallStatus}
              execResult={store.execResult}
              execError={store.execError}
              execLogs={store.execLogs}
              devlProfileExists={store.devlProfileExists}
              oncancel={store.cancelExecution}
              onreset={store.resetAll}
              onopenvscode={store.openInVSCode}
              onreopendevl={store.reopenDevlDialog}
            />
          {/if}
        {/if}
      {:else}

      <!-- ================================================================
           Конструктор: слева фазы + содержимое, справа панель контекста
           ================================================================ -->
      <WizardConstructor />
      {/if}
    {/if}
  {/if}
</div>

<!-- DevLauncher integration: dialogs (profile created / cancel profile) -->
{#if store.DevlDialogs}
  <store.DevlDialogs
    created={store.devlProfileCreated}
    showReminder={store.devlShowReminder}
    confirmCancel={store.devlConfirmCancel}
    onopen={store.openDevLauncher}
    onclose={store.dismissProfileOk}
    oncancel={store.cancelProfile}
    onconfirmcancel={store.confirmCancelProfile}
    ondismisscancel={store.dismissCancelConfirm}
  />
{/if}

<style>
.wizard { max-width: 1280px; margin: 0 auto; padding: 2rem; }
.muted { color: var(--sp-text-3); }
.error { color: var(--sp-danger); }
.mode-switch { 
  display: flex; gap: 0.25rem; margin-bottom: 1.5rem; margin-left: auto; margin-right: auto;
  border-radius: var(--sp-radius-xl); padding: 0.25rem; 
  border: 1px solid rgba(255, 255, 255, 0.08); 
  background: rgba(0, 0, 0, 0.2); 
  backdrop-filter: blur(24px) saturate(150%);
  box-shadow: inset 0 1px 1px rgba(255, 255, 255, 0.05), inset 0 2px 4px rgba(0, 0, 0, 0.2);
  width: fit-content; 
}
.mode-btn { 
  padding: 0.5rem 1.25rem; cursor: pointer; border: none; 
  border-radius: var(--sp-radius-lg); background: transparent; 
  color: var(--sp-text-2); font-size: 0.9rem; 
  transition: transform 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.1), background 0.4s, color 0.4s, box-shadow 0.4s; 
}
.mode-btn:hover { color: var(--sp-text-1); background: rgba(255, 255, 255, 0.05); }
.mode-btn.active { 
  background: rgba(255, 255, 255, 0.1); 
  color: #fff; font-weight: 600; 
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2), inset 0 1px 1px rgba(255, 255, 255, 0.15), inset 0 0 0 1px rgba(255, 255, 255, 0.1); 
}
.mode-btn:active { transform: scale(0.96); }
.prompt { font-size: 1.4rem; font-weight: 700; margin-bottom: 0.4rem; letter-spacing: -0.02em; }
.hint { color: var(--sp-text-3); margin-bottom: 1.5rem; font-size: 0.95rem; }
.creator-footer { margin: 0.9rem 0 0; font-size: 0.8rem; color: var(--sp-text-3); opacity: 0.7; }
.backendless-note { border-left: 3px solid var(--sp-accent-strong); padding: 0.35rem 0.75rem; background: var(--sp-surface-grad), var(--sp-bg-1); box-shadow: var(--sp-gloss-top); margin: 0.75rem 0; }

/* ---- Конструктор: Две колонки (Строгий стиль) ---- */
.builder { display: grid; grid-template-columns: 1fr 340px; gap: 2rem; align-items: start; max-width: 1100px; margin: 0 auto; }
/* Шапка фазы */
.builder-head { grid-column: 1 / -1; min-width: 0; text-align: center; margin-bottom: 1rem; }
.builder-left { display: flex; flex-direction: column; gap: 2rem; min-width: 0; }
.builder-side {
  position: sticky;
  top: 2rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.builder-context {
  display: flex;
  flex-direction: column;
  padding: 1.25rem;
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-xl);
  background: var(--sp-bg-1);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.05);
}

/* Подсказка «Попробуйте тип Кастомный стэк» под панелью контекста */
.custom-stack-hint {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  align-items: flex-start;
  width: 100%;
  text-align: left;
  padding: 0.85rem;
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-lg);
  background: transparent;
  color: var(--sp-text-3);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}
.custom-stack-hint:hover {
  border-color: var(--sp-accent-border);
  background: var(--sp-bg-2);
  color: var(--sp-text-2);
}
.custom-stack-hint .csh-title {
  font-size: 0.74rem;
  font-weight: 600;
  color: var(--sp-text-2);
}
.custom-stack-hint:hover .csh-title { color: var(--sp-accent); }
.custom-stack-hint .csh-body {
  font-size: 0.7rem;
  color: var(--sp-text-3);
}
.ctx-title {
  font-weight: 700;
  font-size: 0.75rem;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  margin: 0 0 0.5rem;
  padding-bottom: 0.5rem;
  color: var(--sp-text-3);
  border-bottom: 1px solid var(--sp-border-faint);
}
.ctx-group { display: flex; flex-direction: column; }
.ctx-row { display: flex; align-items: flex-start; gap: 0.5rem; padding: 0.5rem 0; border-bottom: 1px solid var(--sp-border-faint); }
.ctx-row:last-child { border-bottom: none; }
.ctx-label {
  flex: 0 0 88px;
  font-weight: 600;
  color: var(--sp-text-3);
  font-size: 0.72rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding-top: 0.15rem;
}
.ctx-value { flex: 1; font-size: 0.82rem; color: var(--sp-text-1); overflow-wrap: anywhere; }
.btn-change {
  background: none;
  border: none;
  color: var(--sp-accent);
  padding: 0.1rem 0.15rem;
  border-radius: var(--sp-radius-sm);
  cursor: pointer;
  font-size: 0.72rem;
  flex: 0 0 auto;
  opacity: 0.75;
}
.btn-change:hover { color: var(--sp-accent-strong); text-decoration: underline; opacity: 1; }

/* ---- Фазы (slim stepper) ---- */
.phase-nav { display: flex; align-items: center; justify-content: center; gap: 0; margin-bottom: 1.75rem; flex-wrap: wrap; }
.phase-item {
  display: flex;
  align-items: center;
  gap: 0.45rem;
  background: none;
  border: none;
  cursor: pointer;
  color: var(--sp-text-3);
  font-size: 0.8rem;
  padding: 0.3rem 0.75rem;
}
.phase-item + .phase-item::before {
  content: "";
  width: 26px;
  height: 1px;
  background: var(--sp-border-strong);
  margin-right: 0.75rem;
  flex-shrink: 0;
}
.phase-item:hover { color: var(--sp-text-2); }
.phase-item.active { color: #fff; }
.phase-item.active .phase-circle { background: var(--sp-accent-strong); color: #fff; box-shadow: 0 0 0 3px var(--sp-accent-soft); }
.phase-item.done .phase-circle { background: var(--sp-success); color: #0c0d11; }
.phase-circle {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-border);
  font-weight: 700;
  font-size: 0.7rem;
  transition: background 0.15s, border-color 0.15s;
}
.phase-label { font-weight: 600; }

/* ---- Стороны (Stack) ---- */
.side-section { min-width: 0; }

/* ---- Баннер архитектуры (Integrated / Decoupled) ---- */
.arch-banner {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-lg);
  padding: 0.7rem 0.9rem;
  margin: 0 0 1rem;
  background: var(--sp-surface-grad), var(--sp-bg-1);
  box-shadow: var(--sp-gloss-top);
}
.arch-integrated { border-color: var(--sp-success-border); background: var(--sp-success-soft); }
.arch-decoupled { border-color: var(--sp-accent-border); background: var(--sp-accent-soft); }
.arch-body { flex: 1; min-width: 0; }
.arch-title { margin: 0 0 0.15rem; font-size: 0.85rem; font-weight: 700; color: var(--sp-text-1); }
.arch-integrated .arch-title { color: var(--sp-success); }
.arch-decoupled .arch-title { color: var(--sp-accent); }
.arch-text { margin: 0; font-size: 0.82rem; color: var(--sp-text-2); line-height: 1.35; }
.arch-examples { margin: 0.2rem 0 0; font-size: 0.72rem; color: var(--sp-text-3); }

/* ---- Уровни фреймворков (сворачиваемые колонки) ---- */
.fw-level {
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-lg);
  background: var(--sp-surface-grad), var(--sp-bg-2);
  box-shadow: var(--sp-gloss-top);
  margin-bottom: 0.75rem;
}
.fw-level > summary {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.55rem 0.9rem;
  cursor: pointer;
  list-style: none;
  user-select: none;
  border-bottom: 1px solid transparent;
  border-radius: var(--sp-radius-lg) var(--sp-radius-lg) 0 0;
}
.fw-level > summary::-webkit-details-marker { display: none; }
.fw-level > summary::before {
  content: "▸";
  color: var(--sp-accent-strong);
  font-size: 0.8rem;
  transition: transform 0.15s;
}
.fw-level[open] > summary::before { transform: rotate(90deg); }
.fw-level[open] > summary { border-bottom-color: var(--sp-border); }
.fw-level > summary:hover { background: rgba(255, 255, 255, 0.03); }
.fw-level-title { font-weight: 700; font-size: 0.9rem; color: var(--sp-text-1); }
.fw-level-toolbar { display: flex; justify-content: flex-end; padding: 0.45rem 0.9rem 0; }
.fw-level-action { margin-left: 0.5rem; border: 1px solid var(--sp-accent-border); border-radius: var(--sp-radius-md); padding: 0.2rem 0.45rem; background: transparent; color: var(--sp-text-2); cursor: pointer; font-size: 0.68rem; white-space: nowrap; }
.fw-level-action:hover { border-color: var(--sp-accent-strong); color: #fff; }
.fw-level-count {
  margin-left: auto;
  font-size: 0.72rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.1rem 0.55rem;
}
.fw-level-note { margin: 0; padding: 0.4rem 0.9rem 0.6rem; font-size: 0.75rem; color: var(--sp-text-3); }
.fw-level .fw-grid { margin-bottom: 0; padding: 0.9rem; padding-top: 0.2rem; }

/* ---- Карточки (unified selectable card) ---- */
.card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem; }
.card {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  padding: 1.25rem;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: var(--sp-radius-xl);
  background: var(--sp-glass-bg);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1), inset 0 1px 1px rgba(255, 255, 255, 0.1);
  cursor: pointer;
  transition: transform 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.1), border-color 0.4s, background 0.4s, box-shadow 0.4s;
  text-align: center;
  color: var(--sp-text-1);
  backdrop-filter: blur(12px) saturate(120%);
}
.card:hover { 
  border-color: rgba(255, 255, 255, 0.2); 
  background: rgba(255, 255, 255, 0.05); 
  transform: scale(1.02);
}
.card.selected {
  border-color: var(--sp-accent);
  background: var(--sp-accent-soft);
  box-shadow: inset 0 0 0 1px var(--sp-accent), 0 8px 24px var(--sp-accent-soft);
  transform: scale(1.02);
}
.card:active {
  transform: scale(0.96);
}
.card.blocked { opacity: 0.4; cursor: not-allowed; border-color: rgba(255, 255, 255, 0.05); background: transparent; transform: scale(1); }
.card.blocked:hover { border-color: rgba(255, 255, 255, 0.05); background: transparent; transform: scale(1); }
.card-check {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--sp-accent-strong);
  color: #fff;
  font-size: 0.7rem;
  font-weight: 700;
  box-shadow: var(--sp-shadow-1);
}

/* Утилиты блокировки (клиентская оболочка): карточка видна, но явно
   недоступна — «не молча некликабельна», с подписью и тултипом. */
.opacity-50 { opacity: 0.5; }
.opacity-40 { opacity: 0.4; }
.grayscale { filter: grayscale(1); }
.cursor-not-allowed { cursor: not-allowed; }
.opacity-50:hover,
.opacity-40:hover,
.grayscale:hover { border-color: var(--sp-border-strong); background: var(--sp-bg-1); }
.card h3 { margin: 0; font-size: 0.95rem; }
.card p { margin: 0; font-size: 0.78rem; color: var(--sp-text-3); }
.type-grid { grid-template-columns: repeat(4, 1fr); }
.fw-grid { grid-template-columns: repeat(4, 1fr); grid-auto-rows: 1fr; align-items: stretch; }
.fw-grid .card { min-height: 200px; height: 100%; box-sizing: border-box; }
.fw-grid .card p { flex: 1; }
.unavailable-summary { min-height: 200px; display: flex; flex-direction: column; justify-content: center; gap: 0.5rem; padding: 1rem; border: 1px dashed var(--sp-border-strong); border-radius: var(--sp-radius-lg); background: var(--sp-surface-grad), var(--sp-bg-1); color: var(--sp-text-2); cursor: pointer; text-align: center; }
.unavailable-summary:hover { border-color: var(--sp-accent-strong); color: var(--sp-text-1); background: var(--sp-bg-2); }
.unavailable-summary strong { color: var(--sp-text-1); font-size: 0.9rem; }
.unavailable-summary span { color: var(--sp-text-3); font-size: 0.75rem; }
.fw-grid .card .fw-lang-chip,
.fw-grid .card .fw-lang-multi,
.fw-grid .card .conflict-badge { margin-top: 0.3rem; }
.conflict-badge { display: block; font-size: 0.7rem; color: var(--sp-danger); margin-top: 0.25rem; }
.conflict-detail { display: block; font-size: 0.68rem; color: var(--sp-danger); margin-top: 0.15rem; line-height: 1.25; }
.warn-badge { display: block; font-size: 0.68rem; color: var(--sp-warning); background: var(--sp-warning-soft); border: 1px solid var(--sp-warning-border); border-radius: var(--sp-radius-md); padding: 0.1rem 0.45rem; margin-top: 0.25rem; }
.conflict-alts { display: flex; flex-wrap: wrap; gap: 0.3rem; align-items: center; margin-top: 0.35rem; }
.conflict-alts-label { font-size: 0.68rem; color: var(--sp-text-3); }
.alt-chip {
  display: inline-block;
  font-size: 0.68rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  border-radius: 999px;
  padding: 0.1rem 0.5rem;
  cursor: pointer;
  transition: all 0.15s;
}
.alt-chip:hover { background: var(--sp-accent-border); color: #fff; }
.alt-chip:focus-visible { outline: 2px solid var(--sp-accent-strong); }
.notice-bar { display: block; font-size: 0.78rem; color: var(--sp-warning); background: var(--sp-warning-soft); border: 1px solid var(--sp-warning-border); border-radius: var(--sp-radius-lg); padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.fw-lang-chip { display: inline-block; font-size: 0.72rem; color: var(--sp-accent); background: var(--sp-accent-soft); border: 1px solid var(--sp-accent-border); padding: 0.15rem 0.5rem; border-radius: 999px; margin-top: 0.3rem; }
.fw-lang-chip.selected { color: var(--sp-success); background: var(--sp-success-soft); border-color: var(--sp-success-border); }
.fw-lang-multi { display: block; font-size: 0.68rem; color: var(--sp-accent); margin-top: 0.15rem; }

/* ---- Попап настройки фреймворка ---- */
.fw-card-wrap { position: relative; }
.fw-popup {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 50;
  width: 260px;
  max-width: 90vw;
  background: var(--sp-glass-strong);
  border: 1px solid var(--sp-border-strong);
  border-radius: var(--sp-radius-lg);
  padding: 0.8rem;
  box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-3);
}
.popup-title { margin: 0 0 0.5rem; font-size: 0.85rem; font-weight: 700; color: #fff; }
.popup-label { margin: 0.5rem 0 0.3rem; font-size: 0.72rem; color: var(--sp-text-2); text-transform: uppercase; letter-spacing: 0.04em; }
.popup-list { display: flex; flex-direction: column; gap: 0.3rem; max-height: 160px; overflow-y: auto; }
.popup-opt {
  display: flex; justify-content: space-between; align-items: center; gap: 0.5rem;
  background: var(--sp-bg-2); border: 1px solid var(--sp-accent-border); color: var(--sp-text-1);
  padding: 0.45rem 0.6rem; border-radius: var(--sp-radius-md); cursor: pointer; font-size: 0.82rem; text-align: left;
}
.popup-opt:hover { border-color: var(--sp-accent-strong); }
.popup-opt.selected { border-color: var(--sp-accent); background: var(--sp-surface-grad), var(--sp-bg-2); color: #fff; box-shadow: inset 0 0 0 1px var(--sp-accent-border); }
.popup-opt.stack { flex-direction: column; align-items: stretch; gap: 0.2rem; }
.popup-opt-label { display: flex; align-items: center; gap: 0.45rem; font-weight: 600; }
.popup-opt-desc { font-size: 0.7rem; color: var(--sp-text-3); line-height: 1.35; font-weight: 400; }
.popup-opt.selected .popup-opt-desc { color: var(--sp-text-2); }
.popup-opt-tag {
  font-size: 0.62rem;
  color: var(--sp-info);
  background: var(--sp-info-soft);
  border: 1px solid var(--sp-info-border);
  border-radius: 999px;
  padding: 0.05rem 0.45rem;
  font-weight: 600;
}
.star { color: var(--sp-warning); font-size: 0.72rem; white-space: nowrap; }
.popup-actions { display: flex; gap: 0.4rem; margin-top: 0.7rem; align-items: center; flex-wrap: wrap; }
.btn-xs { padding: 0.3rem 0.7rem; font-size: 0.78rem; }
.btn-remove { background: none; border: 1px solid var(--sp-danger-border); color: var(--sp-danger); padding: 0.3rem 0.7rem; border-radius: var(--sp-radius-md); cursor: pointer; font-size: 0.78rem; }
.btn-remove:hover { background: var(--sp-danger-soft); }

/* ---- Липкий футер страницы стека ---- */
.mega-footer {
  position: sticky;
  bottom: 0;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 1rem;
  margin-top: 1.25rem;
  padding: 0.75rem 1rem;
  background: var(--sp-panel-sheen), var(--sp-solid-chrome);
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-lg);
  box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-2);
  z-index: 40;
}
.mega-summary { font-size: 0.85rem; color: var(--sp-text-2); display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap; }
.mega-langs {
  font-size: 0.78rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  padding: 0.15rem 0.55rem;
  border-radius: 999px;
}
.mega-error { color: var(--sp-danger); font-size: 0.78rem; }
.hint-sm { font-size: 0.75rem; color: var(--sp-text-3); margin: 0 0 0.5rem; }
.tauri-note { display: block; margin-top: 0.5rem; font-size: 0.85rem; color: var(--sp-accent-strong); }

/* ---- Территории стека (Изолированные карточки Focus Mode) ---- */
.territory {
  position: relative;
  background: var(--sp-glass-bg);
  backdrop-filter: blur(24px) saturate(150%);
  border: 1px solid rgba(255, 255, 255, 0.08);
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.2), inset 0 1px 1px rgba(255, 255, 255, 0.1);
  border-radius: var(--sp-radius-xl);
  margin-bottom: 2.5rem;
  transition: transform 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.1), border-color 0.4s;
  overflow: hidden;
}
.territory:hover {
  border-color: rgba(255, 255, 255, 0.15);
}
.territory-head {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1.5rem 1.5rem 1rem 1.5rem;
  background: transparent;
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}
.territory-title-wrap { flex: 1; min-width: 0; }
.territory-title { margin: 0; font-size: 1.25rem; font-weight: 700; color: var(--sp-text-1); letter-spacing: -0.015em; }
.territory-desc { margin: 0.25rem 0 0; font-size: 0.85rem; color: var(--sp-text-3); line-height: 1.4; }
.territory-meta { display: flex; align-items: center; gap: 0.35rem; flex-wrap: wrap; }
.territory-lang-chip {
  font-size: 0.72rem;
  color: var(--sp-accent);
  background: var(--sp-accent-soft);
  border: 1px solid var(--sp-accent-border);
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
}
.territory-count {
  font-size: 0.72rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.15rem 0.55rem;
}
.territory-body { padding: 1.5rem; }
.territory-body .fw-level { margin-bottom: 0.5rem; }
.territory-body .fw-level:last-child { margin-bottom: 0; }
.territory-body .card-grid { margin-bottom: 0.25rem; }

/* Секция «чистых языков» сворачивается как fw-level (native <details>). */
details.territory > summary.territory-head {
  list-style: none;
  cursor: pointer;
  user-select: none;
}
details.territory > summary.territory-head::-webkit-details-marker { display: none; }
details.territory > summary.territory-head::before {
  content: "▸";
  color: var(--sp-accent-strong);
  font-size: 0.8rem;
  flex-shrink: 0;
  transition: transform 0.15s;
}
details.territory[open] > summary.territory-head::before { transform: rotate(90deg); }
details.territory:not([open]) > summary.territory-head {
  border-radius: var(--sp-radius-xl);
  border-bottom-color: transparent;
}
details.territory > summary.territory-head:hover { filter: brightness(1.08); }

/* ---- Чистые языки (plain language picker) ---- */
.lang-sides { display: grid; grid-template-columns: 1fr 1fr; gap: 1.25rem; }
@media (max-width: 900px) { .lang-sides { grid-template-columns: 1fr; } }
.lang-side-title { margin: 0 0 0.15rem; font-size: 0.85rem; font-weight: 700; color: var(--sp-text-1); }
.lang-side-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 0.75rem; }
.lang-grid { grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); margin-bottom: 0; }
.lang-side .unavailable-summary { min-height: 110px; }

/* ---- Инструменты ---- */
.tool-group {
  margin-bottom: 1.5rem;
  padding-top: 1.2rem;
  border-top: 1px solid var(--sp-border-strong);
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.tool-group:first-child {
  border-top: none;
  padding-top: 0;
}
.tool-cat-title {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0 0 0.5rem;
  font-size: 0.8rem;
  font-weight: 700;
  color: var(--sp-text-2);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.tool-cat-count {
  font-size: 0.68rem;
  color: var(--sp-text-3);
  background: var(--sp-bg-2);
  border-radius: 999px;
  padding: 0.1rem 0.5rem;
  font-weight: 600;
}
.tool-menu { display: grid; grid-template-columns: repeat(2, 1fr); gap: 0.5rem; }
.tool-item {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  min-width: 0;
  background: var(--sp-surface-grad), var(--sp-bg-2);
  border: 1px solid var(--sp-border-strong);
  border-radius: var(--sp-radius-lg);
  padding: 0.6rem 0.8rem;
  cursor: pointer;
  text-align: left;
  color: var(--sp-text-1);
  box-shadow: var(--sp-gloss-top);
  transition: border-color 0.15s, background 0.15s, box-shadow 0.15s, transform 0.15s;
}
.tool-item:hover { border-color: var(--sp-accent-strong); background: var(--sp-surface-grad), var(--sp-bg-2); transform: translateY(-1px); }
.tool-item.selected { border-color: var(--sp-accent); background: var(--sp-surface-grad), var(--sp-bg-2); box-shadow: inset 0 0 0 1px var(--sp-accent), var(--sp-gloss-top-strong); }
.tool-item-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.1rem; }
.tool-item-name { font-size: 0.88rem; font-weight: 600; }
.tool-item-desc { font-size: 0.74rem; color: var(--sp-text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.tool-item-badges { display: flex; align-items: center; justify-content: flex-end; gap: 0.35rem; flex: 0 0 auto; flex-wrap: wrap; }
.tool-item-badge { font-size: 0.68rem; padding: 0.12rem 0.45rem; border-radius: 999px; white-space: nowrap; }
.tool-item-badge.docker { color: var(--sp-info); background: var(--sp-info-soft); border: 1px solid var(--sp-info-border); }
.tool-item-badge.conflict { color: var(--sp-danger); background: var(--sp-danger-soft); border: 1px solid var(--sp-danger-border); }
.tool-item-badge.rec { color: var(--sp-warning); background: var(--sp-warning-soft); border: 1px solid var(--sp-warning-border); }
.tool-item-check { color: var(--sp-accent-strong); font-weight: 700; font-size: 0.95rem; }
.group-label { font-size: 0.85rem; font-weight: 600; color: var(--sp-text-3); text-transform: uppercase; letter-spacing: 0.04em; }
.tooltip {
  position: fixed;
  background: var(--sp-glass-strong);
  border: 1px solid var(--sp-border-strong);
  border-radius: var(--sp-radius-lg);
  padding: 0.6rem 0.9rem;
  font-size: 0.8rem;
  max-width: 240px;
  z-index: 999;
  pointer-events: none;
  color: var(--sp-text-2);
  opacity: 0.96;
  box-shadow: var(--sp-gloss-top), 0 6px 20px rgba(0, 0, 0, 0.45);
  animation: tooltip-in 0.14s ease-out;
}
@keyframes tooltip-in {
  from { opacity: 0; transform: translateY(3px); }
  to { opacity: 0.94; transform: translateY(0); }
}
.tooltip strong { color: #fff; }
.tt-req, .tt-conf, .tt-docker { margin: 0.2rem 0; font-size: 0.75rem; }
.features-panel { margin-bottom: 1.5rem; display: flex; flex-wrap: wrap; gap: 1rem; align-items: center; }
.feature-toggle { display: flex; align-items: center; gap: 0.4rem; cursor: pointer; font-size: 0.9rem; }
.feature-toggle input { accent-color: var(--sp-accent-strong); }

/* ---- Кнопки ---- */
.btn-row { display: flex; gap: 0.75rem; margin-top: 1.5rem; flex-wrap: wrap; }.btn-back { background: none; border: 1px solid var(--sp-border-strong); color: var(--sp-text-3); padding: 0.4rem 0.9rem; border-radius: var(--sp-radius-md); cursor: pointer; font-size: 0.85rem; }
.btn-back:hover { border-color: var(--sp-accent-strong); color: #fff; }
.btn-primary {
  background: var(--sp-accent-strong);
  background: linear-gradient(
    180deg,
    color-mix(in srgb, var(--sp-accent-strong) 72%, black) 0%,
    var(--sp-accent-strong) 45%,
    var(--sp-accent) 100%
  );
  color: #fff;
  padding: 0.6rem 1.5rem;
  border-radius: var(--sp-radius-lg);
  border: 1px solid rgba(0, 0, 0, 0.35);
  cursor: pointer;
  font-weight: 600;
  font-size: 0.95rem;
  box-shadow: var(--sp-gloss-top), var(--sp-shadow-1), 0 2px 14px rgba(228, 87, 10, 0.2);
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.55), 0 0 1px rgba(0, 0, 0, 0.4);
  transition: background 0.15s, box-shadow 0.15s;
}
.btn-primary:hover:not(:disabled) {
  background: var(--sp-accent-strong);
  background: linear-gradient(
    180deg,
    color-mix(in srgb, var(--sp-accent-strong) 62%, black) 0%,
    var(--sp-accent-strong) 40%,
    color-mix(in srgb, var(--sp-accent) 88%, var(--sp-accent-strong)) 100%
  );
  box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-accent);
}
.btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-secondary { background: var(--sp-surface-grad), var(--sp-bg-2); color: var(--sp-text-1); padding: 0.6rem 1.5rem; border-radius: var(--sp-radius-lg); border: 1px solid var(--sp-border); box-shadow: var(--sp-gloss-top); cursor: pointer; font-size: 0.95rem; transition: background 0.15s, border-color 0.15s; }
.btn-secondary:hover { background: var(--sp-bg-3); border-color: var(--sp-border-strong); }
.create-btn { font-size: 1.1rem; padding: 0.75rem 2rem; }
.review-error { color: var(--sp-danger); font-weight: 600; font-size: 0.9rem; margin: 1rem 0 0; }
.review-hint { color: var(--sp-text-3); font-size: 0.8rem; margin: 0.5rem 0 0; }

/* ---- Проблемы стека ---- */
.stack-issues { border: 1px solid var(--sp-border-strong); border-radius: var(--sp-radius-lg); padding: 0.8rem 1rem; margin-bottom: 0.75rem; background: var(--sp-surface-grad), var(--sp-bg-2); box-shadow: var(--sp-gloss-top); }
.stack-issue { display: flex; flex-wrap: wrap; gap: 0.3rem 0.4rem; align-items: flex-start; margin: 0.4rem 0; font-size: 0.8rem; line-height: 1.35; }
.stack-issue.error { color: var(--sp-danger); }
.stack-issue.warning { color: var(--sp-warning); }
.stack-issue .recommendation { flex-basis: 100%; margin-top: 0.15rem; font-size: 0.9em; font-style: italic; color: var(--sp-text-3); }

/* ---- Summary ---- */
.dest-card { 
  position: relative; 
  border: 1px solid rgba(255, 255, 255, 0.15); 
  border-radius: var(--sp-radius-xl); 
  padding: 2rem; 
  margin-bottom: 2rem; 
  background: var(--sp-glass-bg); 
  backdrop-filter: blur(24px) saturate(150%);
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.2), inset 0 1px 1px rgba(255, 255, 255, 0.1), 0 0 0 3px var(--sp-accent-soft); 
  transition: transform 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.1), border-color 0.4s;
}
.dest-card-attention { animation: dest-pulse 1.6s ease-in-out 3; }
@keyframes dest-pulse {
  0%, 100% { box-shadow: var(--sp-gloss-top), var(--sp-shadow-1), 0 0 0 3px var(--sp-accent-soft); }
  50% { box-shadow: var(--sp-gloss-top), var(--sp-shadow-1), 0 0 0 7px var(--sp-accent-soft); }
}
.dest-head { display: flex; align-items: flex-start; gap: 0.7rem; margin-bottom: 0.9rem; }
.dest-icon { font-size: 1.4rem; line-height: 1.2; }
.dest-title { margin: 0; font-weight: 700; font-size: 1.05rem; color: var(--sp-text-1); }
.dest-subtitle { margin: 0.15rem 0 0; font-size: 0.82rem; color: var(--sp-text-3); }
.dest-step { display: flex; align-items: center; gap: 0.45rem; margin: 0.65rem 0 0.4rem; }
.dest-step-num { display: inline-flex; align-items: center; justify-content: center; width: 18px; height: 18px; border-radius: 50%; background: var(--sp-accent-strong); color: #fff; font-size: 0.68rem; font-weight: 700; }
.dest-step-label { font-size: 0.78rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--sp-text-2); }
.folder-pick { display: flex; align-items: center; gap: 0.7rem; width: 100%; padding: 0.7rem 0.9rem; border-radius: var(--sp-radius-lg); border: 1px dashed var(--sp-border-strong); background: var(--sp-bg-1); color: var(--sp-text-2); cursor: pointer; text-align: left; transition: border-color 0.15s, background 0.15s; }
.folder-pick:hover { border-color: var(--sp-accent-strong); background: var(--sp-bg-2); }
.folder-pick-set { border-style: solid; }
.folder-pick-icon { font-size: 1.1rem; }
.folder-pick-text { display: flex; flex-direction: column; min-width: 0; flex: 1; }
.folder-pick-path { font-family: var(--sp-font-mono); font-size: 0.82rem; color: var(--sp-text-1); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.folder-pick-path.muted { font-family: var(--sp-font-sans); color: var(--sp-text-3); }
.folder-pick-action { font-size: 0.72rem; color: var(--sp-accent); margin-top: 0.1rem; }
.preview-section { border: 1px solid var(--sp-border-strong); border-radius: var(--sp-radius-lg); padding: 1rem; margin-bottom: 1rem; background: var(--sp-surface-grad), var(--sp-bg-1); box-shadow: var(--sp-gloss-top), var(--sp-shadow-1); min-height: 360px; }
.pn-input { width: 100%; padding: 0.7rem 0.85rem; border-radius: var(--sp-radius-lg); border: 1px solid var(--sp-border-strong); background: var(--sp-bg-1); color: #fff; font-size: 1rem; box-sizing: border-box; outline: none; }
.pn-input:focus { border-color: var(--sp-accent-strong); box-shadow: 0 0 0 3px var(--sp-accent-soft); }
.pn-input::placeholder { color: var(--sp-text-3); }
.pn-input-invalid { border-color: var(--sp-danger); }
.pn-input-invalid:focus { border-color: var(--sp-danger); box-shadow: 0 0 0 3px var(--sp-danger-soft); }
.name-error { margin: 0.35rem 0 0; font-size: 0.8rem; color: var(--sp-danger); }
.name-hint { margin: 0.35rem 0 0; font-size: 0.75rem; color: var(--sp-text-3); }
.readme-row { display: flex; align-items: center; gap: 0.5rem; margin: -0.35rem 0 1rem; }
.readme-label { font-size: 0.82rem; color: var(--sp-text-2); }
.readme-switch { position: relative; width: 34px; height: 18px; padding: 0; border-radius: 999px; border: 1px solid var(--sp-border-strong); background: var(--sp-bg-2); cursor: pointer; transition: background 0.15s, border-color 0.15s; }
.readme-switch[aria-checked="true"] { background: var(--sp-accent); border-color: var(--sp-accent); }
.readme-switch-knob { position: absolute; top: 2px; left: 2px; width: 12px; height: 12px; border-radius: 50%; background: var(--sp-text-3); transition: transform 0.15s, background 0.15s; }
.readme-switch[aria-checked="true"] .readme-switch-knob { transform: translateX(16px); background: #fff; }
.readme-lang { min-width: 1.6rem; font-size: 0.72rem; font-weight: 700; letter-spacing: 0.04em; color: var(--sp-text-2); }
.readme-help-wrap { position: relative; display: inline-flex; }
.readme-help { display: inline-flex; align-items: center; justify-content: center; width: 18px; height: 18px; padding: 0; border: none; border-radius: 50%; background: none; color: var(--sp-text-3); cursor: help; }
.readme-help:hover, .readme-help:focus-visible { color: var(--sp-accent-strong); }
.readme-tip { position: absolute; bottom: calc(100% + 6px); left: 50%; transform: translateX(-50%); width: 280px; padding: 0.5rem 0.65rem; background: var(--sp-bg-1); border: 1px solid var(--sp-border-strong); border-radius: var(--sp-radius-lg); box-shadow: var(--sp-shadow-1); color: var(--sp-text-1); font-size: 0.75rem; line-height: 1.35; z-index: 20; }
.path-preview { display: flex; align-items: center; gap: 0.5rem; margin-top: 0.6rem; flex-wrap: wrap; }
.pp-label { font-size: 0.8rem; color: var(--sp-text-3); }
.pp-path { font-size: 0.85rem; color: var(--sp-accent-strong); background: var(--sp-bg-1); padding: 0.2rem 0.5rem; border-radius: var(--sp-radius-sm); word-break: break-all; }
.pp-checking { font-size: 0.8rem; color: var(--sp-text-3); font-style: italic; }
.pp-exists { font-size: 0.8rem; color: var(--sp-warning); font-weight: 600; }
.conflict-overlay { position: fixed; inset: 0; background: rgba(2, 3, 6, 0.72); backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.clear-overlay { position: fixed; inset: 0; background: rgba(2, 3, 6, 0.72); backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.clear-dialog { background: var(--sp-glass-strong); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); border: 1px solid var(--sp-border-strong); border-radius: var(--sp-radius-xl); padding: 1.5rem; max-width: 460px; width: 90%; box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-3); }
.clear-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
.clear-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
.clear-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.clear-actions button { width: 100%; text-align: center; }
.btn-clear-stack {
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--sp-danger);
  background: var(--sp-danger-soft);
  border: 1px solid var(--sp-danger-border);
  border-radius: var(--sp-radius-lg);
  padding: 0.35rem 0.8rem;
  cursor: pointer;
  margin-bottom: 0.8rem;
  margin-left: 0.5rem;
  transition: background 0.15s;
}
.btn-clear-stack:hover { background: var(--sp-danger-soft); border-color: var(--sp-danger); }
.notice-bar { display: inline-block; font-size: 0.78rem; color: var(--sp-warning); background: var(--sp-warning-soft); border: 1px solid var(--sp-warning-border); border-radius: var(--sp-radius-lg); padding: 0.45rem 0.7rem; margin-bottom: 0.8rem; }
.conflict-dialog { background: var(--sp-glass-strong); backdrop-filter: blur(14px); -webkit-backdrop-filter: blur(14px); border: 1px solid var(--sp-border-strong); border-radius: var(--sp-radius-xl); padding: 1.5rem; max-width: 480px; width: 90%; box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-3); }
.conflict-dialog h3 { margin: 0 0 0.75rem; color: var(--sp-warning); }
.conflict-dialog p { font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 1.25rem; line-height: 1.4; }
.conflict-actions { display: flex; flex-direction: column; gap: 0.6rem; }
.conflict-actions button { width: 100%; text-align: center; }

/* ---- Окружение ---- */
.env-warn { color: var(--sp-warning); font-weight: 600; }

/* ---- Примечание о расширении каталога ---- */
.grow-note {
  margin: 1.5rem auto 0;
  padding-top: 1.25rem;
  text-align: center;
  font-size: 0.8rem;
  color: var(--sp-text-3);
  opacity: 0.55;
}

/* ---- Подсказка «листайте дальше» для новичков (блокировка CTA) ---- */
.scroll-hint {
  position: fixed;
  bottom: 84px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 0.5rem;
  z-index: 60;
  max-width: min(560px, calc(100vw - 2rem));
  padding: 0.55rem 1rem;
  background: var(--sp-glass-strong);
  border: 1px solid var(--sp-accent-strong);
  border-radius: var(--sp-radius-full);
  color: var(--sp-text-1);
  font-size: 0.82rem;
  font-weight: 600;
  cursor: pointer;
  box-shadow: var(--sp-gloss-top-strong), var(--sp-shadow-2), 0 4px 24px rgba(228, 87, 10, 0.28);
  animation: scroll-hint-in 0.35s ease-out;
}
.scroll-hint:hover { background: var(--sp-bg-2); }
.scroll-hint-arrow { color: var(--sp-accent-strong); font-weight: 700; }
@keyframes scroll-hint-in {
  from { opacity: 0; transform: translate(-50%, 10px); }
  to { opacity: 1; transform: translate(-50%, 0); }
}


@keyframes skeleton-pulse {
  0%, 100% { opacity: 0.15; }
  50% { opacity: 0.35; }
}
@keyframes skeleton-shimmer {
  0% { background-position: -400px 0; }
  100% { background-position: 400px 0; }
}
.skeleton-wrap { animation: skeleton-pulse 1.6s ease-in-out infinite; }
.skeleton-header { margin-bottom: 1.2rem; }
.skeleton-bar {
  height: 14px;
  border-radius: var(--sp-radius-md);
  background: linear-gradient(90deg, var(--sp-bg-3) 25%, var(--sp-bg-2) 50%, var(--sp-bg-3) 75%);
  background-size: 800px 100%;
  animation: skeleton-shimmer 1.8s ease-in-out infinite;
}
.skeleton-bar--title { width: 320px; height: 22px; margin-bottom: 0.6rem; }
.skeleton-bar--subtitle { width: 220px; height: 16px; margin-bottom: 1rem; }
.skeleton-bar--sidebar { width: 100%; margin-bottom: 0.8rem; }
.skeleton-bar--sidebar-short { width: 60%; }
.skeleton-mode-switch { display: flex; gap: 0.5rem; margin-bottom: 1.5rem; }
.skeleton-pill {
  width: 110px;
  height: 34px;
  border-radius: var(--sp-radius-lg);
  background: var(--sp-bg-3);
}
.skeleton-builder { display: grid; grid-template-columns: 180px 1fr 260px; gap: 1.5rem; }
.skeleton-phases { display: flex; flex-direction: column; gap: 0.6rem; }
.skeleton-store.phase {
  height: 36px;
  border-radius: var(--sp-radius-lg);
  background: var(--sp-bg-3);
}
.skeleton-content { min-width: 0; }
.skeleton-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 0.8rem; }
.skeleton-card {
  height: 120px;
  border-radius: var(--sp-radius-xl);
  background: var(--sp-bg-3);
}
.skeleton-sidebar { padding-top: 2rem; }

@media (max-width: 1100px) {
  .type-grid, .fw-grid { grid-template-columns: repeat(3, 1fr); }
}
@media (max-width: 1080px) {
  .tool-menu { grid-template-columns: 1fr; }
}
@media (max-width: 900px) {
  .builder { grid-template-columns: 1fr; }
  .builder-side { position: static; }
  .builder-context { position: static; }
  .skeleton-builder { grid-template-columns: 1fr; }
  .skeleton-sidebar { display: none; }
  .type-grid, .fw-grid { grid-template-columns: repeat(2, 1fr); }
}
@media (max-width: 600px) {
  .type-grid, .fw-grid { grid-template-columns: 1fr; }
}
</style>
