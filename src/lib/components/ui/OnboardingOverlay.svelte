<script lang="ts">
  import { APP_NAME, APP_VERSION } from "$lib/core/app";
  import {
    onboarding,
    completeOnboarding,
    skipOnboarding,
    hideOnboarding,
  } from "$lib/core/onboarding";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import Button from "./Button.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  type Step = {
    icon: IconName;
    titleKey: string;
    bodyKeys: string[];
    /** True for the slide that renders the user-path scheme instead of text. */
    scheme?: boolean;
  };

  /** Scheme node — one step of the recommended first-run path. */
  type SchemeNode = {
    icon: IconName;
    titleKey: string;
    bodyKey: string;
  };

  const SCHEME_NODES: SchemeNode[] = [
    {
      icon: "sparkles",
      titleKey: "onboarding.scheme_node1_title",
      bodyKey: "onboarding.scheme_node1_body",
    },
    {
      icon: "search",
      titleKey: "onboarding.scheme_node2_title",
      bodyKey: "onboarding.scheme_node2_body",
    },
    {
      icon: "wrench",
      titleKey: "onboarding.scheme_node3_title",
      bodyKey: "onboarding.scheme_node3_body",
    },
    {
      icon: "layers",
      titleKey: "onboarding.scheme_node4_title",
      bodyKey: "onboarding.scheme_node4_body",
    },
    {
      icon: "play",
      titleKey: "onboarding.scheme_node5_title",
      bodyKey: "onboarding.scheme_node5_body",
    },
    {
      icon: "folder",
      titleKey: "onboarding.scheme_node6_title",
      bodyKey: "onboarding.scheme_node6_body",
    },
  ];

  const STEPS: Step[] = [
    {
      icon: "rocket",
      titleKey: "onboarding.step1_title",
      bodyKeys: ["onboarding.step1_body1", "onboarding.step1_body2"],
    },
    {
      icon: "map",
      titleKey: "onboarding.step2_title",
      bodyKeys: ["onboarding.step2_body1", "onboarding.step2_body2"],
      scheme: true,
    },
    {
      icon: "sparkles",
      titleKey: "onboarding.step3_title",
      bodyKeys: ["onboarding.step3_body1", "onboarding.step3_body2", "onboarding.step3_body3"],
    },
    {
      icon: "play",
      titleKey: "onboarding.step4_title",
      bodyKeys: ["onboarding.step4_body1", "onboarding.step4_body2"],
    },
    {
      icon: "folder",
      titleKey: "onboarding.step5_title",
      bodyKeys: ["onboarding.step5_body1", "onboarding.step5_body2"],
    },
    {
      icon: "wrench",
      titleKey: "onboarding.step6_title",
      bodyKeys: ["onboarding.step6_body1", "onboarding.step6_body2"],
    },
    {
      icon: "shield",
      titleKey: "onboarding.step7_title",
      bodyKeys: ["onboarding.step7_body1", "onboarding.step7_body2"],
    },
  ];

  let stepIndex = $state(0);

  const step = $derived(STEPS[stepIndex]);
  const isLast = $derived(stepIndex === STEPS.length - 1);
</script>

{#if $onboarding.open}
  <div class="sp-ob-backdrop" role="presentation">
    <div
      class="sp-ob-panel"
      role="dialog"
      aria-modal="true"
      aria-label="Getting started"
      tabindex="-1"
    >
      <div class="sp-ob-accent" aria-hidden="true"></div>

      <div class="sp-ob-body">
        <div class="sp-ob-icon" aria-hidden="true">
          <Icon name={step.icon} size={26} />
        </div>
        <h2 class="sp-ob-title">{i18n.t(step.titleKey as TranslationKey)}</h2>
        {#if step.scheme}
          <div class="sp-ob-text">
            {#each step.bodyKeys as bodyKey}
              <p>{i18n.t(bodyKey as TranslationKey)}</p>
            {/each}
          </div>
          <ol class="sp-ob-scheme">
            {#each SCHEME_NODES as node, i}
              <li class="sp-ob-scheme-node">
                <span class="sp-ob-scheme-node-head">
                  <span class="sp-ob-scheme-icon" aria-hidden="true">
                    <Icon name={node.icon} size={15} />
                  </span>
                  <span class="sp-ob-scheme-title">
                    {i18n.t(node.titleKey as TranslationKey)}
                  </span>
                  {#if i === 0}
                    <span class="sp-ob-scheme-start">
                      {i18n.t("onboarding.scheme_start") as TranslationKey}
                    </span>
                  {/if}
                </span>
                <span class="sp-ob-scheme-body">
                  {i18n.t(node.bodyKey as TranslationKey)}
                </span>
                {#if i < SCHEME_NODES.length - 1}
                  <span class="sp-ob-scheme-arrow" aria-hidden="true">
                    <Icon name="chevronDown" size={14} />
                  </span>
                {/if}
              </li>
            {/each}
          </ol>
          <p class="sp-ob-scheme-tip">{i18n.t("onboarding.step2_tip") as TranslationKey}</p>
        {/if}
      </div>

      <div class="sp-ob-dots" aria-hidden="true">
        {#each STEPS as _, i}
          <span
            class="sp-ob-dot"
            class:sp-ob-dot-active={i === stepIndex}
          ></span>
        {/each}
      </div>

      <footer class="sp-ob-footer">
        <Button
          variant="subtle"
          size="sm"
          onclick={() => {
            skipOnboarding();
          }}
        >
          {i18n.t("onboarding.skip") as TranslationKey}
        </Button>
        <div class="sp-ob-nav">
          {#if stepIndex > 0}
            <Button
              variant="ghost"
              size="sm"
              icon="chevronLeft"
              onclick={() => stepIndex--}
            >
              {i18n.t("onboarding.back") as TranslationKey}
            </Button>
          {/if}
          {#if isLast}
            <Button
              variant="primary"
              size="sm"
              icon="check"
              onclick={() => completeOnboarding()}
            >
              {i18n.t("onboarding.done") as TranslationKey}
            </Button>
          {:else}
            <Button
              variant="primary"
              size="sm"
              iconRight="chevronRight"
              onclick={() => stepIndex++}
            >
              {i18n.t("onboarding.next") as TranslationKey}
            </Button>
          {/if}
        </div>
      </footer>

      <p class="sp-ob-version">v{APP_VERSION}</p>

      <button
        class="sp-ob-close"
        aria-label={i18n.t("onboarding.close") as TranslationKey}
        title={i18n.t("onboarding.close") as TranslationKey}
        onclick={() => hideOnboarding()}
      >
        <Icon name="x" size={16} />
      </button>
    </div>
  </div>
{/if}

<style>
  .sp-ob-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1050;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-6);
    background: rgba(2, 3, 6, 0.72);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    animation: sp-fade-in 0.18s ease;
  }

  .sp-ob-panel {
    position: relative;
    width: min(46rem, 100%);
    max-height: min(44rem, calc(100vh - var(--sp-12)));
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--sp-glass-strong);
    border: 1px solid var(--sp-border-strong);
    border-radius: var(--sp-radius-xl);
    box-shadow: var(--sp-shadow-3);
    animation: sp-zoom-in 0.18s ease;
  }

  .sp-ob-accent {
    height: 0.25rem;
    background: linear-gradient(
      90deg,
      var(--sp-accent-strong),
      var(--sp-accent)
    );
  }

  .sp-ob-body {
    flex: 1 1 auto;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: var(--sp-7) var(--sp-8) var(--sp-3);
  }

  .sp-ob-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 3.5rem;
    height: 3.5rem;
    border-radius: var(--sp-radius-full);
    background: linear-gradient(
      135deg,
      var(--sp-accent-soft),
      var(--sp-info-soft)
    );
    color: var(--sp-accent);
    border: 1px solid var(--sp-accent-border);
    box-shadow: var(--sp-shadow-1);
    margin-bottom: var(--sp-4);
  }

  .sp-ob-title {
    margin: 0;
    font-size: var(--sp-fs-xl);
    font-weight: var(--sp-fw-bold);
    letter-spacing: -0.02em;
    color: var(--sp-text-1);
  }

  .sp-ob-text {
    margin-top: var(--sp-3);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .sp-ob-text p {
    margin: 0;
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-2);
    line-height: var(--sp-lh-normal);
  }

  /* Scheme slide */

  .sp-ob-scheme {
    margin: var(--sp-4) auto 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: 30rem;
    text-align: left;
  }

  .sp-ob-scheme-node {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .sp-ob-scheme-node-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    background: var(--sp-bg-1);
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
  }

  .sp-ob-scheme-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--sp-radius-sm);
    background: var(--sp-accent-soft);
    color: var(--sp-accent);
    flex-shrink: 0;
  }

  .sp-ob-scheme-title {
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-text-1);
    flex: 1;
    min-width: 0;
  }

  .sp-ob-scheme-start {
    font-size: var(--sp-fs-2xs);
    font-weight: var(--sp-fw-semibold);
    color: var(--sp-success);
    background: var(--sp-success-soft);
    border: 1px solid var(--sp-success-border);
    border-radius: var(--sp-radius-full);
    padding: 0.125rem 0.5rem;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .sp-ob-scheme-body {
    padding: 0 var(--sp-3);
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    line-height: var(--sp-lh-normal);
  }

  .sp-ob-scheme-arrow {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--sp-text-3);
    opacity: 0.6;
    margin: var(--sp-1) 0;
  }

  .sp-ob-scheme-tip {
    margin: var(--sp-4) 0 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-warning);
    line-height: var(--sp-lh-normal);
  }

  .sp-ob-dots {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: var(--sp-3) 0;
    flex-shrink: 0;
  }

  .sp-ob-dot {
    width: 0.375rem;
    height: 0.375rem;
    border-radius: var(--sp-radius-full);
    background: var(--sp-border-strong);
    transition: background-color 0.15s ease, width 0.15s ease;
  }

  .sp-ob-dot-active {
    width: 1.25rem;
    background: var(--sp-accent);
  }

  .sp-ob-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-6) var(--sp-6);
    flex-shrink: 0;
  }

  .sp-ob-nav {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .sp-ob-version {
    position: absolute;
    bottom: var(--sp-5);
    right: var(--sp-6);
    margin: 0;
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-3);
    font-family: var(--sp-font-mono);
  }

  .sp-ob-close {
    position: absolute;
    top: var(--sp-3);
    right: var(--sp-3);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border: none;
    border-radius: var(--sp-radius-sm);
    background: transparent;
    color: var(--sp-text-3);
    cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }

  .sp-ob-close:hover {
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
  }
</style>