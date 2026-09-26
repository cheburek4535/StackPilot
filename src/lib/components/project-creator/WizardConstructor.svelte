<script lang="ts">
  import { getProjectStore } from "$lib/modules/project_creator/createStore.svelte";
  import type { FrameworkDef } from "$lib/modules/project_creator/types";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Icon from "$lib/components/ui/Icon.svelte";
  import TechIcon from "$lib/components/TechIcon.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { availableLocales } from "$lib/core/i18n.svelte";

  const store = getProjectStore();

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      }
    };
  }

  $effect(() => {
    const dismiss = () => store.hideTooltip();
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("pointerdown", dismiss, true);
    return () => {
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("pointerdown", dismiss, true);
    };
  });
</script>

      <div class="builder">
        <div class="builder-head">
          <div class="phase-nav">
            {#each store.PHASES as name, i}
              <button
                class="phase-item"
                class:active={i === store.phase}
                class:done={i < store.phase}
                onclick={() => store.goPhase(i)}
              >
                <span class="phase-circle">{i < store.phase ? "✓" : i + 1}</span>
                <span class="phase-label">{name}</span>
              </button>
            {/each}
          </div>

          {#snippet archBanner()}
            {#if store.archMode}
              <div class="arch-banner arch-{store.archMode}" role="status">
                <div class="arch-body">
                  <p class="arch-title">
                    {store.archMode === "integrated"
                      ? (i18n.t("create.mode_integrated") as TranslationKey)
                      : (i18n.t("create.mode_decoupled") as TranslationKey)}
                  </p>
                  <p class="arch-text">
                    {store.archMode === "integrated"
                      ? (i18n.t("create.mode_integrated_desc") as TranslationKey)
                      : (i18n.t("create.mode_decoupled_desc") as TranslationKey)}
                  </p>
                  <p class="arch-examples">
                    {store.archMode === "integrated"
                      ? (i18n.t("create.mode_integrated_ex") as TranslationKey)
                      : (i18n.t("create.mode_decoupled_ex") as TranslationKey)}
                  </p>
                </div>
              </div>
            {/if}
          {/snippet}

          <div class="builder-head-text">
            {#if store.phase === 0}
              <h2 class="prompt">{i18n.t("create.what_building") as TranslationKey}</h2>
              <p class="prompt-sub">Выберите базовый тип архитектуры для формирования правильного стека технологий</p>
            {/if}
            {#if store.phase === 1}
              <div class="phase-title-row">
                <h2 class="prompt">{i18n.t("create.stack_tools") as TranslationKey}</h2>
                {#if store.selectedFrameworks.length > 0 || store.selectedTools.length > 0}
                  <button
                    class="btn-clear-stack"
                    title={i18n.t("create.clear_stack_title") as TranslationKey}
                    onclick={() => (store.confirmClearStack = true)}
                  >
                    <Icon name="x" size={13} />
                    <span>{i18n.t("create.clear_stack") as TranslationKey}</span>
                  </button>
                {/if}
              </div>
              <p class="prompt-sub">{i18n.t("create.fw_hint") as TranslationKey}</p>
              {@render archBanner()}
              {#if store.dropNotice}
                <p class="notice-bar" role="status">{store.dropNotice}</p>
              {/if}

              <store.HelpHint
                id={store.HINT_CREATE_STACK.id}
                resolvedBy={store.HINT_CREATE_STACK.resolvedBy}
                icon="layers"
                title={i18n.t("help.create_stack.title") as TranslationKey}
                text={i18n.t("help.create_stack.body") as TranslationKey}
              />

              {#if store.confirmClearStack}
                <div class="clear-overlay" onclick={() => (store.confirmClearStack = false)}>
                  <div class="clear-dialog" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
                    <h3>{i18n.t("create.clear_confirm_title") as TranslationKey}</h3>
                    <p>
                      {i18n.t("create.clear_confirm_body") as TranslationKey}
                    </p>
                    <div class="clear-actions">
                      <button class="btn-primary" onclick={() => store.clearStack()}>{i18n.t("create.clear_yes") as TranslationKey}</button>
                      <button class="btn-back" onclick={() => (store.confirmClearStack = false)}>{i18n.t("create.cancel") as TranslationKey}</button>
                    </div>
                  </div>
                </div>
              {/if}
            {/if}
            {#if store.phase === 2}
              <h2 class="prompt">{i18n.t("create.review_create_short") as TranslationKey}</h2>
              <p class="prompt-sub">Проверьте выбранный стек, имя директории и перейдите к генерации проекта</p>
              {@render archBanner()}
            {/if}
          </div>
        </div>

        <div class="builder-left">
          <!-- Phase 0: Project Type -->
          {#if store.phase === 0}
            <store.HelpHint
              id={store.HINT_CREATE_TYPE.id}
              resolvedBy={store.HINT_CREATE_TYPE.resolvedBy}
              icon="layers"
              title={i18n.t("help.create_type.title") as TranslationKey}
              text={i18n.t("help.create_type.body") as TranslationKey}
            />
            <div class="card-grid type-grid">
              {#each store.tree!.project_types as pt}
                <button class="card" onclick={() => store.selectType(pt)}>
                  <TechIcon icon={pt.icon} alt={i18n.t(pt.label as TranslationKey)} size="xl" />
                  <h3>{i18n.t(pt.label as TranslationKey)}</h3>
                  <p>{i18n.t(pt.description as TranslationKey)}</p>
                </button>
              {/each}
            </div>
            <p class="creator-footer">{i18n.t("create.custom_stack_footer") as TranslationKey}</p>
          {/if}

          <!-- Phase 1: Stack & Tools — одна скролл-страница -->
          {#if store.phase === 1}
            {@const fws = store.availableFrameworks()}
            {@const backendFws = fws.filter((f) => f.side === "backend")}
            {@const frontendFws = fws.filter((f) => f.side === "frontend")}
            {@const eitherFws = fws.filter((f) => f.side === "either")}

            {#snippet fwCard(fw: FrameworkDef)}
              {@const reason = store.frameworkBlockReason(fw.id)}
              {@const altInfo = reason !== null ? store.frameworkBlockInfo(fw.id) : null}
              {@const warnReason = store.frameworkWarnReason(fw.id)}
              {@const summary = store.fwLangSummary(fw)}
              <div class="fw-card-wrap">
                <button
                  class="card"
                  class:selected={store.selectedFrameworks.includes(fw.id)}
                  class:blocked={reason !== null}
                  onclick={() => {
                    store.hideTooltip();
                    store.clickFramework(fw.id);
                  }}
                  onmouseenter={(e) => store.showTooltip({ type: "framework", fw, reason, altInfo, warnReason }, e)}
                  onmouseleave={store.hideTooltip}
                  onfocus={(e) => store.showTooltip({ type: "framework", fw, reason, altInfo, warnReason }, e)}
                  onblur={store.hideTooltip}
                >
                  {#if store.selectedFrameworks.includes(fw.id)}
                    <span class="card-check" aria-hidden="true">✓</span>
                  {/if}
                  <TechIcon icon={fw.icon} alt={i18n.t(fw.label as TranslationKey)} size="lg" />
                  <h3>{i18n.t(fw.label as TranslationKey)}</h3>
                  <p>{i18n.t(fw.description as TranslationKey)}</p>
                  {#if store.selectedFrameworks.includes(fw.id) && summary}
                    <span class="fw-lang-chip selected">{summary}</span>
                  {:else}
                    <span class="fw-lang-chip">{store.fwLangsLabel(fw)}</span>
                  {/if}
                  {#if fw.languages.length > 1}
                    <span class="fw-lang-multi">{i18n.t("create.choose_language") as TranslationKey}</span>
                  {/if}
                  {#if warnReason !== null}
                    <span class="warn-badge" title={warnReason}>⚠ {warnReason}</span>
                  {/if}
                  {#if reason}
                    <span class="conflict-badge">{reason}</span>
                    {#if altInfo?.detail}
                      <span class="conflict-detail">{altInfo.detail}</span>
                    {/if}
                    {#if altInfo?.alternatives?.length}
                      <span class="conflict-alts">
                        <span class="conflict-alts-label">{i18n.t("create.instead_of") as TranslationKey}</span>
                        {#each altInfo.alternatives as altId}
                          {@const altFw = store.tree?.frameworks.find((f) => f.id === altId)}
                          {#if altFw}
                            <span
                              role="button"
                              tabindex="0"
                              class="alt-chip"
                              onclick={(e) => {
                                e.stopPropagation();
                                store.clickFramework(altId);
                              }}
                              onkeydown={(e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  e.stopPropagation();
                                  store.clickFramework(altId);
                                }
                              }}
                            >
                              {i18n.t(altFw.label as TranslationKey)}
                            </span>
                          {/if}
                        {/each}
                      </span>
                    {/if}
                  {/if}
                </button>
                {#if store.fwPopup === fw.id}
                  <div class="fw-popup">
                    <p class="popup-title">{i18n.t(fw.label as TranslationKey)}</p>
                    {#if fw.qt_ui_options?.length}
                      {@const selectedMode = fw.qt_ui_options.find((m) => m.id === store.popupQtUi) ?? fw.qt_ui_options[0]}
                      {@const webDefs = (selectedMode.web_framework_options ?? [])
                        .map((wid) => store.tree?.frameworks.find((f) => f.id === wid))
                        .filter((d): d is FrameworkDef => !!d)}
                      <p class="popup-label">{i18n.t("create.ui_technology") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each fw.qt_ui_options as innerMode}
                          <button
                            class="popup-opt stack"
                            class:selected={store.popupQtUi === innerMode.id}
                            onclick={() => (store.popupQtUi = innerMode.id)}
                          >
                            <span class="popup-opt-label">
                              {i18n.t(innerMode.label as TranslationKey)}
                              {#if (innerMode.web_framework_options ?? []).length > 0}
                                <span class="popup-opt-tag">{i18n.t("create.web_ui") as TranslationKey}</span>
                              {/if}
                            </span>
                            <span class="popup-opt-desc">{i18n.t(innerMode.description as TranslationKey)}</span>
                          </button>
                        {/each}
                      </div>
                      {#if webDefs.length > 0}
                        <p class="popup-label">{i18n.t("create.web_frontend") as TranslationKey}</p>
                        <div class="popup-list">
                          {#each webDefs as wf}
                            <button
                              class="popup-opt stack"
                              class:selected={store.popupWebFw === wf.id}
                              onclick={() => (store.popupWebFw = wf.id)}
                            >
                              <span class="popup-opt-label">{i18n.t(wf.label as TranslationKey)}</span>
                              <span class="popup-opt-desc">{i18n.t(wf.description as TranslationKey)}</span>
                            </button>
                          {/each}
                        </div>
                      {/if}
                    {:else if store.companionOptions(fw).length > 0}
                      <p class="popup-label">{i18n.t("create.frontend_framework") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each store.companionOptions(fw) as c, ci}
                          <button
                            class="popup-opt"
                            class:selected={store.popupCompanion === c.id}
                            onclick={() => {
                              store.popupCompanion = c.id;
                              store.popupCompanionLang = c.recommended_language;
                            }}
                          >
                            <span>{i18n.t(c.label as TranslationKey)}</span>
                            {#if ci === 0}
                              <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                      {#if store.popupCompanion}
                        {@const cfw = store.tree?.frameworks.find((f) => f.id === store.popupCompanion)}
                        {#if cfw}
                          <p class="popup-label">{i18n.t("create.frontend_language") as TranslationKey}</p>
                          <div class="popup-list">
                            {#each cfw.languages as l}
                              <button
                                class="popup-opt"
                                class:selected={store.popupCompanionLang === l}
                                onclick={() => (store.popupCompanionLang = l)}
                              >
                                <span>{store.langLabel(l)}</span>
                                {#if l === cfw.recommended_language}
                                  <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                                {/if}
                              </button>
                            {/each}
                          </div>
                        {/if}
                      {/if}
                    {:else}
                      <p class="popup-label">{i18n.t("create.language") as TranslationKey}</p>
                      <div class="popup-list">
                        {#each fw.languages as l}
                          <button
                            class="popup-opt"
                            class:selected={store.popupLang === l}
                            onclick={() => (store.popupLang = l)}
                          >
                            <span>{store.langLabel(l)}</span>
                            {#if l === fw.recommended_language}
                              <span class="star">{i18n.t("create.recommended") as TranslationKey}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                    <div class="popup-actions">
                      <button class="btn-primary btn-xs" onclick={store.applyFwPopup}>{i18n.t("create.done") as TranslationKey}</button>
                      <button class="btn-secondary btn-xs" onclick={store.cancelFwPopup}>{i18n.t("create.cancel") as TranslationKey}</button>
                      {#if store.selectedFrameworks.includes(fw.id)}
                        <button class="btn-remove" onclick={() => store.removeFramework(fw.id)}>{i18n.t("create.remove") as TranslationKey}</button>
                      {/if}
                    </div>
                  </div>
                {/if}
              </div>
            {/snippet}

            {#snippet fwLevel(title: string, items: FrameworkDef[], note: string, levelKey: string)}
              {@const unavailable = store.unavailableFrameworks(items)}
              {@const visibleItems = store.availableFrameworksForDisplay(items, levelKey)}
              <details class="fw-level" open>
                <summary>
                  <TechIcon alt="" size="sm" />
                  <span class="fw-level-title">{title}</span>
                  <span class="fw-level-count">{items.length}</span>
                </summary>
                {#if unavailable.length > 0}
                  <div class="fw-level-toolbar">
                    <button
                      type="button"
                      class="fw-level-action"
                      onclick={() => store.toggleUnavailable(levelKey)}
                    >
                      {store.showUnavailable[levelKey] ? (i18n.t("create.hide_unavailable") as TranslationKey) : (i18n.t("create.show_unavailable") as TranslationKey)}
                    </button>
                  </div>
                {/if}
                <p class="fw-level-note">{note}</p>
                <div class="card-grid fw-grid">
                  {#each visibleItems as fw}
                    {@render fwCard(fw)}
                  {/each}
                  {#if unavailable.length > 0 && !store.showUnavailable[levelKey]}
                    <button
                      type="button"
                      class="unavailable-summary"
                      onclick={() => store.toggleUnavailable(levelKey)}
                    >
                      <strong>{i18n.t("create.unavailable_count", { n: unavailable.length }) as TranslationKey}</strong>
                      <span>{i18n.t("create.unavailable_desc") as TranslationKey}</span>
                    </button>
                  {/if}
                </div>
              </details>
            {/snippet}

            {#snippet territory(side: string, title: string, desc: string, items: FrameworkDef[], langs: string[])}
              <section class="territory territory-{side}">
                <header class="territory-head">
                  <TechIcon alt="" size="md" />
                  <div class="territory-title-wrap">
                    <h3 class="territory-title">{title}</h3>
                    <p class="territory-desc">{desc}</p>
                  </div>
                  <div class="territory-meta">
                    {#each langs as l}
                      <span class="territory-lang-chip">{store.langLabel(l)}</span>
                    {/each}
                    <span class="territory-count">{items.length}</span>
                  </div>
                </header>
                <div class="territory-body">
                  {#each store.FW_LEVELS as lvl}
                    {@const lvlItems = items.filter((f) => store.fwLevelOf(f) === lvl.id)}
                    {#if lvlItems.length > 0}
                      {@render fwLevel(lvl.title, lvlItems, lvl.note, `${side}-${lvl.id}`)}
                    {/if}
                  {/each}
                </div>
              </section>
            {/snippet}

            {#if store.hasBackend && backendFws.length > 0}
              {@render territory(
                "backend",
                i18n.t("create.terr_backend") as TranslationKey,
                i18n.t("create.terr_backend_desc") as TranslationKey,
                backendFws,
                store.backendLangs,
              )}
            {/if}
            {#if frontendFws.length > 0}
              {@render territory(
                "frontend",
                i18n.t("create.terr_frontend") as TranslationKey,
                i18n.t("create.terr_frontend_desc") as TranslationKey,
                frontendFws,
                store.frontendLangs,
              )}
            {/if}
            {#if eitherFws.length > 0}
              {@render territory(
                "either",
                i18n.t("create.terr_standalone") as TranslationKey,
                i18n.t("create.terr_standalone_desc") as TranslationKey,
                eitherFws,
                [],
              )}
            {/if}
            {#if !store.hasBackend}
              <p class="hint backendless-note">
                {i18n.t("create.no_backend_note") as TranslationKey}
              </p>
            {/if}
            {#if fws.length === 0}
              <p class="muted">{i18n.t("create.no_frameworks") as TranslationKey}</p>
            {/if}

            <!-- Языки без фреймворков (необязательно): чистый стек или поддержка -->
            <details class="territory territory-langs" open>
              <summary class="territory-head">
                <TechIcon alt="" size="md" />
                <div class="territory-title-wrap">
                  <h3 class="territory-title">{i18n.t("create.plain_languages") as TranslationKey}</h3>
                  <p class="territory-desc">
                    {i18n.t("create.plain_languages_desc") as TranslationKey}
                  </p>
                </div>
              </summary>
              <div class="territory-body">
                <div class="lang-sides">
                  {#if store.hasBackend && store.backendCandidates().length > 0}
                    {@const beBlocked = store.blockedLanguages("backend")}
                    <div class="lang-side">
                      <div class="lang-side-head">
                        <div>
                          <p class="lang-side-title">{i18n.t("create.backend_language") as TranslationKey}</p>
                          <p class="hint-sm">{i18n.t("create.one_per_side") as TranslationKey}</p>
                        </div>
                        {#if beBlocked.length > 0}
                          <button
                            type="button"
                            class="fw-level-action"
                            onclick={() => store.toggleUnavailableLangs("backend")}
                          >
                            {store.showUnavailableLangs["backend"]
                              ? (i18n.t("create.hide_unavailable") as TranslationKey)
                              : (i18n.t("create.show_unavailable") as TranslationKey)}
                          </button>
                        {/if}
                      </div>
                      <div class="card-grid lang-grid">
                        {#each store.visibleLanguages("backend") as lang}
                          {@const blockedReason = store.languageBlockReason(lang, "backend")}
                          {@const blockedDetail = store.languageBlockDetail(lang)}
                          <button
                            class="card"
                            class:selected={store.backendLangs.includes(lang.id)}
                            class:blocked={blockedReason !== null}
                            disabled={blockedReason !== null}
                            onclick={() => {
                              store.hideTooltip();
                              store.toggleLang("backend", lang.id);
                            }}
                            onmouseenter={(e) => store.showTooltip({ type: "language", lang, side: "backend", blockedReason, blockedDetail }, e)}
                            onmouseleave={store.hideTooltip}
                            onfocus={(e) => store.showTooltip({ type: "language", lang, side: "backend", blockedReason, blockedDetail }, e)}
                            onblur={store.hideTooltip}
                          >
                            {#if store.backendLangs.includes(lang.id)}
                              <span class="card-check" aria-hidden="true">✓</span>
                            {/if}
                            <TechIcon icon={lang.icon} alt={i18n.t(lang.label as TranslationKey)} size="lg" />
                            <h3>{i18n.t(lang.label as TranslationKey)}</h3>
                            {#if store.backendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">{i18n.t("create.active") as TranslationKey}</span>
                            {/if}
                            {#if blockedReason}
                              <span class="conflict-badge">{blockedReason}</span>
                            {/if}
                            {#if blockedDetail}
                              <span class="conflict-detail">{blockedDetail}</span>
                            {/if}
                          </button>
                        {/each}
                        {#if beBlocked.length > 0 && !store.showUnavailableLangs["backend"]}
                          <button
                            type="button"
                            class="unavailable-summary"
                            onclick={() => store.toggleUnavailableLangs("backend")}
                          >
                            <strong>{i18n.t("create.unavailable_count", { n: beBlocked.length }) as TranslationKey}</strong>
                            <span>{i18n.t("create.unavailable_desc") as TranslationKey}</span>
                          </button>
                        {/if}
                      </div>
                    </div>
                  {/if}
                  {#if store.frontendCandidates().length > 0}
                    {@const feBlocked = store.blockedLanguages("frontend")}
                    <div class="lang-side">
                      <div class="lang-side-head">
                        <div>
                          <p class="lang-side-title">{i18n.t("create.frontend_language2") as TranslationKey}</p>
                          <p class="hint-sm">{i18n.t("create.one_per_side") as TranslationKey}</p>
                        </div>
                        {#if feBlocked.length > 0}
                          <button
                            type="button"
                            class="fw-level-action"
                            onclick={() => store.toggleUnavailableLangs("frontend")}
                          >
                            {store.showUnavailableLangs["frontend"]
                              ? (i18n.t("create.hide_unavailable") as TranslationKey)
                              : (i18n.t("create.show_unavailable") as TranslationKey)}
                          </button>
                        {/if}
                      </div>
                      <div class="card-grid lang-grid">
                        {#each store.visibleLanguages("frontend") as lang}
                          {@const blockedReason = store.languageBlockReason(lang, "frontend")}
                          <button
                            class="card"
                            class:selected={store.frontendLangs.includes(lang.id)}
                            class:blocked={blockedReason !== null}
                            disabled={blockedReason !== null}
                            onclick={() => {
                              store.hideTooltip();
                              store.toggleLang("frontend", lang.id);
                            }}
                            onmouseenter={(e) => store.showTooltip({ type: "language", lang, side: "frontend", blockedReason }, e)}
                            onmouseleave={store.hideTooltip}
                            onfocus={(e) => store.showTooltip({ type: "language", lang, side: "frontend", blockedReason }, e)}
                            onblur={store.hideTooltip}
                          >
                            {#if store.frontendLangs.includes(lang.id)}
                              <span class="card-check" aria-hidden="true">✓</span>
                            {/if}
                            <TechIcon icon={lang.icon} alt={i18n.t(lang.label as TranslationKey)} size="lg" />
                            <h3>{i18n.t(lang.label as TranslationKey)}</h3>
                            {#if lang.category === "static"}
                              <p>{i18n.t("create.plain_html") as TranslationKey}</p>
                            {/if}
                            {#if store.frontendLangs.includes(lang.id)}
                              <span class="fw-lang-chip selected">{i18n.t("create.active") as TranslationKey}</span>
                            {/if}
                            {#if blockedReason}
                              <span class="conflict-badge">{blockedReason}</span>
                            {/if}
                          </button>
                        {/each}
                        {#if feBlocked.length > 0 && !store.showUnavailableLangs["frontend"]}
                          <button
                            type="button"
                            class="unavailable-summary"
                            onclick={() => store.toggleUnavailableLangs("frontend")}
                          >
                            <strong>{i18n.t("create.unavailable_count", { n: feBlocked.length }) as TranslationKey}</strong>
                            <span>{i18n.t("create.unavailable_desc") as TranslationKey}</span>
                          </button>
                        {/if}
                      </div>
                    </div>
                  {/if}
                  {#if !store.hasBackend}
                    <p class="hint backendless-note">
                      {i18n.t("create.backend_lang_skipped") as TranslationKey}
                    </p>
                  {/if}
                </div>
              </div>
            </details>

            <!-- Инструменты и фичи -->
            <store.HelpHint
              id={store.HINT_CREATE_TOOLS.id}
              resolvedBy={store.HINT_CREATE_TOOLS.resolvedBy}
              icon="wrench"
              title={i18n.t("help.create_tools.title") as TranslationKey}
              text={i18n.t("help.create_tools.body") as TranslationKey}
            />
            <section class="territory territory-tools" class:sp-help-anchor={store.toolsHintVisible}>
              <header class="territory-head">
                <TechIcon alt="" size="md" />
                <div class="territory-title-wrap">
                  <h3 class="territory-title">{i18n.t("create.tools_features") as TranslationKey}</h3>
                  <p class="territory-desc">{i18n.t("create.tools_features_desc") as TranslationKey}</p>
                </div>
                <span class="territory-count">{i18n.t("create.selected_count", { n: store.selectedTools.length }) as TranslationKey}</span>
              </header>
              <div class="territory-body">
                {#each store.TOOL_CATEGORIES as cat}
                  {@const catTools = store.availableTools().filter((t) => t.category === cat.id)}
                  {#if catTools.length > 0}
                    <div class="tool-group">
                      <p class="tool-cat-title">
                        <TechIcon alt="" size="xs" />
                        {cat.label}
                        <span class="tool-cat-count">{catTools.length}</span>
                      </p>
                      <div class="tool-menu">
                        {#each catTools as tool}
                          <button
                            class="tool-item"
                            class:selected={store.selectedTools.includes(tool.id)}
                            onclick={() => {
                              store.hideTooltip();
                              store.toggleTool(tool.id);
                            }}
                            onmouseenter={(e) => store.showTooltip({ type: "tool", tool }, e)}
                            onmouseleave={store.hideTooltip}
                            onfocus={(e) => store.showTooltip({ type: "tool", tool }, e)}
                            onblur={store.hideTooltip}
                          >
                            <TechIcon icon={tool.icon} alt={i18n.t(tool.label as TranslationKey)} size="md" />
                            <span class="tool-item-text">
                              <span class="tool-item-name">{i18n.t(tool.label as TranslationKey)}</span>
                              <span class="tool-item-desc">{i18n.t(tool.description as TranslationKey)}</span>
                            </span>
                            <span class="tool-item-badges">
                              {#if store.recommendedBadgeIds().includes(tool.id)}
                                <span class="tool-item-badge rec">{i18n.t("create.recommended") as TranslationKey}</span>
                              {/if}
                              {#if tool.requires_docker}
                                <span class="tool-item-badge docker"><TechIcon icon="docker.svg" alt="" size="xs" /> {i18n.t("create.docker_badge") as TranslationKey}</span>
                              {/if}
                              {#if tool.conflicts.length > 0}
                                <span class="tool-item-badge conflict">
                                  {i18n.t("create.conflicts_count", { n: tool.conflicts.length }) as TranslationKey}
                                </span>
                              {/if}
                              {#if store.selectedTools.includes(tool.id)}
                                <span class="tool-item-check">✓</span>
                              {/if}
                            </span>
                          </button>
                        {/each}
                      </div>
                    </div>
                  {/if}
                {/each}



                <div class="features-panel">
                  <p class="group-label">{i18n.t("create.features") as TranslationKey}</p>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={store.testing} />
                    <span>{i18n.t("create.feature.testing") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={store.git} />
                    <span>{i18n.t("create.feature.git") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" bind:checked={store.vscode} />
                    <span>{i18n.t("create.feature.vscode") as TranslationKey}</span>
                  </label>
                  <label class="feature-toggle">
                    <input type="checkbox" checked={store.dockerEnabled()} disabled />
                    <span>{i18n.t("create.feature.docker") as TranslationKey}{store.isDockerForced() ? ` ${i18n.t("create.feature.docker_forced") as TranslationKey}` : ""}</span>
                  </label>
                </div>
              </div>
            </section>

            <!-- Липкий футер: сводка + переход к финальной сверке -->
            <div class="mega-footer">
              <span class="mega-summary">
                {#if store.allSelectedLangs().length > 0}
                  <span class="mega-langs">{store.allSelectedLangs().map((l) => store.langLabel(l)).join(" · ")}</span>
                {/if}
                {i18n.t("create.summary_count", { f: store.selectedFrameworks.length, t: store.selectedTools.length }) as TranslationKey}
                {#if store.stackError}
                  <span class="mega-error">⚠ {store.stackError}</span>
                {/if}
              </span>
              <button
                class="btn-primary"
                class:sp-help-anchor={store.stackHintVisible && !store.stackError && !store.reviewLocked}
                onclick={() => (store.phase = 2)}
                disabled={!!store.stackError || store.reviewLocked}
                title={store.reviewLocked ? (i18n.t("create.scroll_lock_hint") as TranslationKey) : undefined}
              >
                {i18n.t("create.review_create") as TranslationKey}
              </button>
            </div>

            {#if store.reviewLocked}
              <button
                type="button"
                class="scroll-hint"
                onclick={store.scrollToStackBottom}
              >
                <span class="scroll-hint-arrow" aria-hidden="true">↓</span>
                <span>{i18n.t("create.scroll_lock_hint") as TranslationKey}</span>
              </button>
            {/if}

            <p class="grow-note">
              {i18n.t("create.catalog_grows_note") as TranslationKey}
            </p>
          {/if}

          <!-- Phase 2: Review -->
          {#if store.phase === 2}
            <div class="dest-card" class:dest-card-attention={!store.selectedFolder}>
              <div class="dest-head">
                <span class="dest-icon" aria-hidden="true">📁</span>
                <div class="dest-head-text">
                  <p class="dest-title">{i18n.t("create.dest_title") as TranslationKey}</p>
                  <p class="dest-subtitle">{i18n.t("create.dest_desc") as TranslationKey}</p>
                </div>
              </div>

              <div class="dest-step">
                <span class="dest-step-num">1</span>
                <span class="dest-step-label">{i18n.t("create.dest_step_folder") as TranslationKey}</span>
              </div>
              <button
                type="button"
                class="folder-pick"
                class:folder-pick-set={!!store.selectedFolder}
                onclick={store.pickProjectFolder}
              >
                <span class="folder-pick-icon" aria-hidden="true">📂</span>
                <span class="folder-pick-text">
                  <span class="folder-pick-path" class:muted={!store.selectedFolder} title={store.selectedFolder ?? undefined}>
                    {store.selectedFolder ?? (i18n.t("create.dest_no_folder") as TranslationKey)}
                  </span>
                  <span class="folder-pick-action">
                    {store.selectedFolder
                      ? (i18n.t("create.change_folder") as TranslationKey)
                      : (i18n.t("create.select_folder_first") as TranslationKey)}
                  </span>
                </span>
              </button>

              <div class="dest-step">
                <span class="dest-step-num">2</span>
                <span class="dest-step-label">{i18n.t("create.dest_step_name") as TranslationKey}</span>
              </div>
              <input
                id="project-name"
                class="pn-input"
                class:pn-input-invalid={store.projectNameError !== null}
                type="text"
                placeholder={i18n.t("create.project_name_ph") as TranslationKey}
                bind:value={store.projectName}
                oninput={store.onProjectNameInput}
                aria-invalid={store.projectNameError !== null}
              />
              {#if store.projectNameError}
                <p class="name-error" role="alert">⛔ {store.projectNameError}</p>
              {:else}
                <p class="name-hint">{i18n.t("create.name_hint") as TranslationKey}</p>
              {/if}

              {#if store.selectedFolder && store.projectName}
                <div class="path-preview">
                  <span class="pp-label">{i18n.t("create.full_path") as TranslationKey}</span>
                  <code class="pp-path">{store.effectiveProjectPath()}</code>
                  {#if store.folderCheckPending}
                    <span class="pp-checking">{i18n.t("create.checking") as TranslationKey}</span>
                  {:else if store.folderExists}
                    <span class="pp-exists">{i18n.t("create.folder_exists") as TranslationKey}</span>
                  {/if}
                </div>
              {/if}
            </div>

            <!-- README language: компактный переключатель RU/EN + пояснение -->
            <div class="readme-row">
              <span class="readme-label">{i18n.t("create.readme.toggle_label") as TranslationKey}</span>
              <button
                type="button"
                class="readme-switch"
                role="switch"
                aria-checked={store.readmeLocale === availableLocales[1].id}
                aria-label={i18n.t("create.readme.toggle_aria") as TranslationKey}
                onclick={store.toggleReadmeLocale}
              >
                <span class="readme-switch-knob"></span>
              </button>
              <span class="readme-lang">{store.readmeLocaleLabel}</span>
              <span class="readme-help-wrap">
                <button
                  type="button"
                  class="readme-help"
                  aria-label={i18n.t("create.readme.toggle_hint") as TranslationKey}
                  onmouseenter={() => (store.readmeHelpOpen = true)}
                  onmouseleave={() => (store.readmeHelpOpen = false)}
                  onfocus={() => (store.readmeHelpOpen = true)}
                  onblur={() => (store.readmeHelpOpen = false)}
                >
                  <Icon name="help" size={14} />
                </button>
                {#if store.readmeHelpOpen}
                  <span class="readme-tip" role="tooltip">
                    {i18n.t("create.readme.toggle_hint") as TranslationKey}
                  </span>
                {/if}
              </span>
            </div>

            <store.HelpHint
              id={store.HINT_CREATE_PREVIEW.id}
              resolvedBy={store.HINT_CREATE_PREVIEW.resolvedBy}
              variant="info"
              icon="search"
              title={i18n.t("help.create_preview.title") as TranslationKey}
            >
              <p>{i18n.t("help.create_preview.body") as TranslationKey}</p>
              <p>{i18n.t("help.create_preview.next") as TranslationKey}</p>
            </store.HelpHint>

            <div class="preview-section">
              {#if store.PreviewPanel}
                <store.PreviewPanel
                  selectedType={store.selectedType}
                  backendLangs={store.backendLangs}
                  frontendLangs={store.frontendLangs}
                  selectedFrameworks={store.selectedFrameworks}
                  selectedTools={store.selectedTools}
                  envLocalInfra={store.envLocalInfra}
                  testing={store.testing}
                  git={store.git}
                  vscode={store.vscode}
                  store.readmeLocale={store.readmeLocale}
                  projectName={store.projectName || ""}
                  projectFolder={store.selectedFolder || ""}
                  bind:removedStepIds={store.removedStepIds}
                />
              {/if}
            </div>

            {#if store.showConflictDialog}
              <div class="conflict-overlay" onclick={() => { store.showConflictDialog = false; }}>
                <div class="conflict-dialog" onclick={(e) => e.stopPropagation()}>
                  <h3>{i18n.t("create.folder_exists") as TranslationKey}</h3>
                  <p>
                    {i18n.t("create.folder_conflict_body") as TranslationKey}
                  </p>
                  <div class="conflict-actions">
                    <button class="btn-primary" onclick={() => store.resolveFolderConflict('overwrite')}>
                      {i18n.t("create.overwrite") as TranslationKey}
                    </button>
                    <button class="btn-secondary" onclick={() => store.resolveFolderConflict('auto-rename')}>
                      {i18n.t("create.auto_rename", { name: store.projectName }) as TranslationKey}
                    </button>
                    <button class="btn-back" onclick={() => store.resolveFolderConflict('cancel')}>
                      {i18n.t("create.use_other_name") as TranslationKey}
                    </button>
                  </div>
                </div>
              </div>
            {/if}

            {#if store.reviewError}
              <p class="review-error" role="alert">⛔ {i18n.t(store.reviewError as TranslationKey)}</p>
            {/if}
            {#if store.stackError && (!store.projectName || !store.selectedFolder)}
              <p class="review-hint">⚠ {store.stackError}</p>
            {/if}

            <div class="btn-row">
              <button class="btn-back" onclick={store.back}>{i18n.t("create.back") as TranslationKey}</button>
              <button
                class="btn-primary create-btn"
                class:sp-help-anchor={store.previewHintVisible && !!store.projectName && !!store.selectedFolder && !store.stackError && !store.projectNameError}
                disabled={!store.projectName || !store.selectedFolder || store.stackError !== null || store.projectNameError !== null}
                title={store.stackError ?? store.projectNameError ?? undefined}
                onclick={store.confirmAll}
              >
                {i18n.t("create.create_project") as TranslationKey}
              </button>
            </div>
            {#if store.projectNameError}
              <p class="review-hint">⚠ {store.projectNameError}</p>
            {:else if !store.projectName || !store.selectedFolder}
              <p class="review-hint">
                {!store.projectName ? (i18n.t("create.enter_name") as TranslationKey) : (i18n.t("create.select_folder_first") as TranslationKey)}
                {store.stackError ? ` · ${store.stackError}` : ""}
              </p>
            {/if}
          {/if}
        </div>

        <!-- ============ Панель контекста (правая колонка) ============ -->
        <div class="builder-side">
        <aside class="builder-context">
          <p class="ctx-title">{i18n.t("create.your_stack") as TranslationKey}</p>

          {#if store.stackIssues.length > 0}
            <div class="stack-issues">
              {#each store.stackIssues as issue}
                {@const isError = issue.severity !== "Warning"}
                {@const text = issue.message_key ? i18n.t(issue.message_key as TranslationKey, issue.args) : issue.message}
                <div class="stack-issue" class:error={isError} class:warning={!isError}>
                  <span class="icon">{isError ? "⛔" : "⚠️"}</span>
                  <span class="message">{text}</span>
                  {#if issue.args?.recommendation}
                    <div class="recommendation">💡 {issue.args.recommendation}</div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          <div class="ctx-group">
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.project_type") as TranslationKey}</span>
              <span class="ctx-value">{store.selectedType ? (i18n.t(store.selectedType.label as TranslationKey)) : "—"}</span>
              <button class="btn-change" onclick={() => store.goPhase(0)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.backend") as TranslationKey}</span>
              <span class="ctx-value">
                {store.hasBackend
                  ? (store.backendLangs.length > 0 ? store.backendLangs.map((l) => store.langLabel(l)).join(", ") : (i18n.t("create.none") as TranslationKey))
                  : (i18n.t("create.na_no_backend") as TranslationKey)}
              </span>
              {#if store.hasBackend}
                <button class="btn-change" onclick={() => store.goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
              {/if}
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.frontend") as TranslationKey}</span>
              <span class="ctx-value">{store.frontendLangs.length > 0 ? store.frontendLangs.map((l) => store.langLabel(l)).join(", ") : (i18n.t("create.none") as TranslationKey)}</span>
              <button class="btn-change" onclick={() => store.goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.frameworks") as TranslationKey}</span>
              <span class="ctx-value">
                {store.selectedFrameworks.length > 0
                  ? store.selectedFrameworks
                      .map((id) => {
                        const f = store.tree?.frameworks.find((x) => x.id === id);
                        return f ? i18n.t(f.label as TranslationKey) : id;
                      })
                      .join(", ")
                  : (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => store.goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.tools") as TranslationKey}</span>
              <span class="ctx-value">
                {store.selectedTools.length > 0
                  ? store.selectedTools
                      .map((id) => {
                        const t = store.tree?.tools.find((x) => x.id === id);
                        return t ? i18n.t(t.label as TranslationKey) : id;
                      })
                      .join(", ")
                  : (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => store.goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
            <div class="ctx-row">
              <span class="ctx-label">{i18n.t("create.features") as TranslationKey}</span>
              <span class="ctx-value">
                {[
                  store.git && i18n.t("create.feature.git"),
                  store.testing && i18n.t("create.feature.testing"),
                  store.vscode && i18n.t("create.feature.vscode"),
                  store.dockerEnabled() && i18n.t("create.feature.docker"),
                ].filter(Boolean).join(", ") || (i18n.t("create.none") as TranslationKey)}
              </span>
              <button class="btn-change" onclick={() => store.goPhase(1)}>{i18n.t("create.change") as TranslationKey}</button>
            </div>
          </div>

          </aside>

          {#if store.phase !== 0 && store.selectedType && store.selectedType.id !== "custom"}
            <button
              type="button"
              class="custom-stack-hint"
              title={i18n.t("create.custom_hint_body") as TranslationKey}
              onclick={() => store.goPhase(0)}
            >
              <span class="csh-title">{i18n.t("create.custom_hint_title") as TranslationKey}</span>
              <span class="csh-body">{i18n.t("create.custom_hint_body") as TranslationKey}</span>
            </button>
          {/if}
        </div>
      </div>

{#if store.tooltipData}
  <div
    use:portal
    class="sp-floating-tooltip sp-tt-{store.tooltipData.placement}"
    style="left: {store.tooltipData.x}px; top: {store.tooltipData.y}px;"
    role="tooltip"
  >
    {#if store.tooltipData.item.type === "tool"}
      {@const tool = store.tooltipData.item.tool}
      <div class="tt-header">
        <TechIcon icon={tool.icon} alt="" size="sm" />
        <span class="tt-title">{i18n.t(tool.label as TranslationKey)}</span>
      </div>
      <p class="tt-desc">{i18n.t(tool.description as TranslationKey)}</p>

      {#if store.recommendedBadgeIds().includes(tool.id)}
        <div class="tt-badge-row">
          <span class="tt-chip tt-chip-rec">⭐ {i18n.t("create.recommended") as TranslationKey}</span>
        </div>
      {/if}

      {#if tool.requires_docker}
        <div class="tt-row tt-docker">
          <TechIcon icon="docker.svg" alt="" size="xs" />
          <span>{i18n.t("create.requires_docker") as TranslationKey}</span>
        </div>
      {/if}

      {#if tool.requires && tool.requires.length > 0}
        <div class="tt-row">
          <span class="tt-label">{i18n.t("create.requires", { list: "" }) as TranslationKey}</span>
          <span class="tt-val">{tool.requires.join(", ")}</span>
        </div>
      {/if}

      {#if tool.conflicts && tool.conflicts.length > 0}
        <div class="tt-row tt-conf">
          <span class="tt-label">{i18n.t("create.conflicts_with", { list: "" }) as TranslationKey}</span>
          <span class="tt-val">{tool.conflicts.join(", ")}</span>
        </div>
      {/if}

    {:else if store.tooltipData.item.type === "framework"}
      {@const fw = store.tooltipData.item.fw}
      {@const item = store.tooltipData.item}
      <div class="tt-header">
        <TechIcon icon={fw.icon} alt="" size="sm" />
        <div class="tt-title-wrap">
          <span class="tt-title">{i18n.t(fw.label as TranslationKey)}</span>
          <div class="tt-tags">
            <span class="tt-tag">
              {fw.side === "backend" ? i18n.t("create.backend") : fw.side === "frontend" ? i18n.t("create.frontend") : "Fullstack"}
            </span>
            <span class="tt-tag">{fw.class === "standalone" ? "Standalone" : "In-place"}</span>
          </div>
        </div>
      </div>
      <p class="tt-desc">{i18n.t(fw.description as TranslationKey)}</p>

      <div class="tt-meta-block">
        <div class="tt-row">
          <span class="tt-label">Языки:</span>
          <span class="tt-val">{fw.languages.map((l) => store.langLabel(l)).join(", ")}</span>
        </div>
        {#if fw.languages.length > 1}
          <div class="tt-row">
            <span class="tt-label">Основной язык:</span>
            <span class="tt-val highlight">{store.langLabel(fw.recommended_language)}</span>
          </div>
        {/if}
      </div>

      {#if fw.recommends && fw.recommends.length > 0}
        {@const recNames = fw.recommends.map((r) => {
          const rf = store.tree?.frameworks.find((x) => x.id === r.framework);
          return rf ? i18n.t(rf.label as TranslationKey) : r.framework;
        }).join(", ")}
        <div class="tt-row tt-rec">
          <span class="tt-label">Рекомендуется с:</span>
          <span class="tt-val">{recNames}</span>
        </div>
      {/if}

      {#if item.reason}
        <div class="tt-alert tt-alert-error">
          <span class="tt-alert-icon">⛔</span>
          <div>
            <div class="tt-alert-msg">{item.reason}</div>
            {#if item.altInfo?.detail}
              <div class="tt-alert-sub">{item.altInfo.detail}</div>
            {/if}
          </div>
        </div>
      {/if}

      {#if item.warnReason}
        <div class="tt-alert tt-alert-warn">
          <span class="tt-alert-icon">⚠️</span>
          <div class="tt-alert-msg">{item.warnReason}</div>
        </div>
      {/if}

    {:else if store.tooltipData.item.type === "language"}
      {@const lang = store.tooltipData.item.lang}
      {@const item = store.tooltipData.item}
      {@const compatFws = (store.tree?.frameworks ?? []).filter((f) => f.languages.includes(lang.id))}
      <div class="tt-header">
        <TechIcon icon={lang.icon} alt="" size="sm" />
        <div class="tt-title-wrap">
          <span class="tt-title">{store.langLabel(lang.id)}</span>
          <span class="tt-tag">
            {item.side === "backend" ? i18n.t("create.backend_language") : i18n.t("create.frontend_language2")}
          </span>
        </div>
      </div>

      {#if lang.category === "static"}
        <p class="tt-desc">{i18n.t("create.plain_html") as TranslationKey}</p>
      {:else}
        <p class="tt-desc">
          {item.side === "backend"
            ? "Язык программирования для серверной части проекта."
            : "Язык программирования для клиентской части проекта."}
        </p>
      {/if}

      {#if compatFws.length > 0}
        <div class="tt-meta-block">
          <span class="tt-label">Фреймворки в каталоге ({compatFws.length}):</span>
          <div class="tt-fw-pills">
            {#each compatFws.slice(0, 6) as cfw}
              <span class="tt-fw-pill">{i18n.t(cfw.label as TranslationKey)}</span>
            {/each}
            {#if compatFws.length > 6}
              <span class="tt-fw-pill-more">+{compatFws.length - 6}</span>
            {/if}
          </div>
        </div>
      {/if}

      {#if item.blockedReason}
        <div class="tt-alert tt-alert-error">
          <span class="tt-alert-icon">⛔</span>
          <div>
            <div class="tt-alert-msg">{item.blockedReason}</div>
            {#if item.blockedDetail}
              <div class="tt-alert-sub">{item.blockedDetail}</div>
            {/if}
          </div>
        </div>
      {/if}
    {/if}
  </div>
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
.builder {
  display: grid;
  grid-template-columns: 1fr 340px;
  gap: 1.5rem;
  align-items: start;
  width: 100%;
}

/* Шапка фазы */
.builder-head {
  grid-column: 1 / -1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  padding: 1.25rem 2rem 1.5rem;
  background: var(--sp-surface-grad), var(--sp-bg-1);
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-xl);
  box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
  margin-bottom: 0.5rem;
  text-align: center;
}

.builder-head-text {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
}

.prompt {
  margin: 0;
  font-size: var(--sp-fs-xl);
  font-weight: var(--sp-fw-bold);
  color: var(--sp-text-1);
  letter-spacing: -0.02em;
}

.prompt-sub {
  margin: var(--sp-1) 0 0;
  font-size: var(--sp-fs-sm);
  color: var(--sp-text-3);
  max-width: 42rem;
  line-height: var(--sp-lh-normal);
}

.phase-title-row {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.btn-clear-stack {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-1);
  padding: 0.25rem 0.6rem;
  background: var(--sp-danger-soft);
  color: var(--sp-danger);
  border: 1px solid var(--sp-danger-border);
  border-radius: var(--sp-radius-md);
  font-size: var(--sp-fs-xs);
  font-weight: var(--sp-fw-medium);
  cursor: pointer;
  transition: all 0.15s ease;
}

.btn-clear-stack:hover {
  background: rgba(239, 68, 68, 0.18);
  color: #fff;
}

.builder-left { display: flex; flex-direction: column; gap: 1.5rem; min-width: 0; }
.builder-side {
  position: sticky;
  top: 1.5rem;
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
  background: var(--sp-surface-grad), var(--sp-bg-1);
  box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
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

/* ---- Фазы (stepper) ---- */
.phase-nav {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0;
  margin-bottom: 0.35rem;
  flex-wrap: wrap;
  padding: 0.25rem 0.5rem;
  background: var(--sp-bg-2);
  border: 1px solid var(--sp-border);
  border-radius: var(--sp-radius-full);
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, 0.25);
}
.phase-item {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  background: none;
  border: none;
  cursor: pointer;
  color: var(--sp-text-3);
  font-size: var(--sp-fs-sm);
  font-weight: var(--sp-fw-medium);
  font-family: inherit;
  padding: 0.35rem 0.85rem;
  border-radius: var(--sp-radius-full);
  transition: all 0.2s cubic-bezier(0.16, 1, 0.3, 1);
}
.phase-item + .phase-item::before {
  content: "";
  width: 24px;
  height: 1px;
  background: var(--sp-border-strong);
  margin-right: 0.85rem;
  flex-shrink: 0;
}
.phase-item:hover {
  color: var(--sp-text-1);
  background: rgba(255, 255, 255, 0.04);
}
.phase-item.active {
  color: #fff;
  background: var(--sp-surface-grad), var(--sp-bg-3);
  box-shadow: var(--sp-gloss-top), var(--sp-shadow-1);
}
.phase-circle {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--sp-bg-1);
  border: 1px solid var(--sp-border);
  font-weight: var(--sp-fw-bold);
  font-size: 0.72rem;
  color: var(--sp-text-3);
  transition: all 0.2s ease;
}
.phase-item.active .phase-circle {
  background: var(--sp-accent-strong);
  border-color: var(--sp-accent);
  color: #fff;
  box-shadow: 0 0 14px var(--sp-accent-glow);
}
.phase-item.done .phase-circle {
  background: var(--sp-success-soft);
  border-color: var(--sp-success);
  color: var(--sp-success);
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
.type-grid .card { height: 100%; box-sizing: border-box; }
.fw-grid { grid-template-columns: repeat(auto-fill, 230px); grid-auto-rows: 1fr; align-items: stretch; justify-content: center; }
.fw-grid .card { min-height: 220px; height: 100%; width: 100%; box-sizing: border-box; flex: 1; }
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
.fw-card-wrap { position: relative; display: flex; flex-direction: column; height: 100%; }
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
  background: rgba(255, 255, 255, 0.02);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: var(--sp-radius-lg);
  padding: 0.6rem 0.8rem;
  cursor: pointer;
  text-align: left;
  color: var(--sp-text-1);
  transition: border-color 0.15s, background 0.15s, transform 0.15s;
}
.tool-item:hover { border-color: rgba(255, 255, 255, 0.2); background: rgba(255, 255, 255, 0.05); transform: translateY(-1px); }
.tool-item.selected { border-color: var(--sp-accent); background: var(--sp-accent-soft); }
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
:global(.sp-floating-tooltip) {
  position: fixed;
  z-index: 99999;
  pointer-events: none;
  width: min(300px, calc(100vw - 24px));
  box-sizing: border-box;
  padding: 0.75rem 0.9rem;
  border-radius: var(--sp-radius-xl, 12px);
  background: rgba(14, 18, 25, 0.96);
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.55), 0 2px 8px rgba(0, 0, 0, 0.3), inset 0 1px 1px rgba(255, 255, 255, 0.12);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  color: var(--sp-text-1, #f8fafc);
  font-size: 0.8rem;
  line-height: 1.4;
}

:global(.sp-floating-tooltip.sp-tt-bottom) {
  transform: translate(-50%, 0);
  animation: sp-tt-fade-down 0.14s cubic-bezier(0.16, 1, 0.3, 1);
}

:global(.sp-floating-tooltip.sp-tt-top) {
  transform: translate(-50%, -100%);
  animation: sp-tt-fade-up 0.14s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes sp-tt-fade-down {
  from { opacity: 0; transform: translate(-50%, -6px); }
  to { opacity: 1; transform: translate(-50%, 0); }
}

@keyframes sp-tt-fade-up {
  from { opacity: 0; transform: translate(-50%, calc(-100% + 6px)); }
  to { opacity: 1; transform: translate(-50%, -100%); }
}

:global(.sp-floating-tooltip .tt-header) {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  margin-bottom: 0.45rem;
}

:global(.sp-floating-tooltip .tt-title-wrap) {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

:global(.sp-floating-tooltip .tt-title) {
  font-weight: 700;
  font-size: 0.9rem;
  color: #fff;
  letter-spacing: -0.01em;
}

:global(.sp-floating-tooltip .tt-tags) {
  display: flex;
  gap: 0.35rem;
  flex-wrap: wrap;
}

:global(.sp-floating-tooltip .tt-tag) {
  font-size: 0.65rem;
  font-weight: 600;
  padding: 0.05rem 0.45rem;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.1);
  color: var(--sp-text-2, #94a3b8);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

:global(.sp-floating-tooltip .tt-desc) {
  margin: 0 0 0.5rem;
  font-size: 0.77rem;
  color: var(--sp-text-2, #cbd5e1);
  line-height: 1.38;
}

:global(.sp-floating-tooltip .tt-badge-row) {
  margin-bottom: 0.45rem;
}

:global(.sp-floating-tooltip .tt-chip) {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  font-size: 0.7rem;
  font-weight: 600;
  padding: 0.15rem 0.55rem;
  border-radius: 999px;
}

:global(.sp-floating-tooltip .tt-chip-rec) {
  background: var(--sp-warning-soft, rgba(245, 158, 11, 0.15));
  border: 1px solid var(--sp-warning-border, rgba(245, 158, 11, 0.3));
  color: var(--sp-warning, #f59e0b);
}

:global(.sp-floating-tooltip .tt-meta-block) {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  padding: 0.45rem 0.6rem;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: var(--sp-radius-md, 8px);
  margin-bottom: 0.5rem;
}

:global(.sp-floating-tooltip .tt-row) {
  display: flex;
  align-items: baseline;
  gap: 0.4rem;
  font-size: 0.74rem;
  line-height: 1.35;
}

:global(.sp-floating-tooltip .tt-label) {
  color: var(--sp-text-3, #94a3b8);
  font-weight: 500;
  flex-shrink: 0;
}

:global(.sp-floating-tooltip .tt-val) {
  color: var(--sp-text-1, #f1f5f9);
  font-weight: 600;
}

:global(.sp-floating-tooltip .tt-val.highlight) {
  color: var(--sp-accent-strong, #fb923c);
}

:global(.sp-floating-tooltip .tt-docker) {
  color: var(--sp-info, #38bdf8);
  margin-bottom: 0.35rem;
  font-weight: 600;
}

:global(.sp-floating-tooltip .tt-conf) {
  color: var(--sp-danger, #f87171);
}

:global(.sp-floating-tooltip .tt-fw-pills) {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
  margin-top: 0.25rem;
}

:global(.sp-floating-tooltip .tt-fw-pill) {
  font-size: 0.68rem;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.07);
  border: 1px solid rgba(255, 255, 255, 0.1);
  color: var(--sp-text-1, #e2e8f0);
}

:global(.sp-floating-tooltip .tt-fw-pill-more) {
  font-size: 0.68rem;
  padding: 0.1rem 0.4rem;
  color: var(--sp-text-3, #64748b);
}

:global(.sp-floating-tooltip .tt-alert) {
  display: flex;
  gap: 0.45rem;
  padding: 0.45rem 0.65rem;
  border-radius: var(--sp-radius-md, 8px);
  font-size: 0.73rem;
  line-height: 1.35;
  margin-top: 0.45rem;
}

:global(.sp-floating-tooltip .tt-alert-error) {
  background: rgba(239, 68, 68, 0.14);
  border: 1px solid rgba(239, 68, 68, 0.3);
  color: #fca5a5;
}

:global(.sp-floating-tooltip .tt-alert-warn) {
  background: rgba(245, 158, 11, 0.14);
  border: 1px solid rgba(245, 158, 11, 0.3);
  color: #fcd34d;
}

:global(.sp-floating-tooltip .tt-alert-msg) {
  font-weight: 600;
}

:global(.sp-floating-tooltip .tt-alert-sub) {
  margin-top: 0.2rem;
  opacity: 0.9;
  font-size: 0.7rem;
}
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
  .type-grid { grid-template-columns: repeat(3, 1fr); }
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
  .type-grid { grid-template-columns: repeat(2, 1fr); }
}
@media (max-width: 600px) {
  .type-grid { grid-template-columns: 1fr; }
}
</style>
