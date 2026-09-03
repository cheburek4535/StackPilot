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
  };

  const STEPS: Step[] = [
    {
      icon: "rocket",
      titleKey: "onboarding.step1_title",
      bodyKeys: ["onboarding.step1_body1", "onboarding.step1_body2"],
    },
    {
      icon: "layers",
      titleKey: "onboarding.step2_title",
      bodyKeys: ["onboarding.step2_body1", "onboarding.step2_body2"],
    },
    {
      icon: "clock",
      titleKey: "onboarding.step3_title",
      bodyKeys: ["onboarding.step3_body1", "onboarding.step3_body2"],
    },
    {
      icon: "folder",
      titleKey: "onboarding.step4_title",
      bodyKeys: ["onboarding.step4_body1", "onboarding.step4_body2"],
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
        <div class="sp-ob-text">
          {#each step.bodyKeys as bodyKey}
            <p>{i18n.t(bodyKey as TranslationKey)}</p>
          {/each}
        </div>
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
    background: rgba(5, 6, 10, 0.68);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    animation: sp-fade-in 0.18s ease;
  }

  .sp-ob-panel {
    position: relative;
    width: min(30rem, 100%);
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
      var(--sp-violet-strong),
      var(--sp-cyan),
      var(--sp-lime)
    );
  }

  .sp-ob-body {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: var(--sp-8) var(--sp-8) var(--sp-4);
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
      rgba(6, 182, 212, 0.1)
    );
    color: var(--sp-accent);
    border: 1px solid var(--sp-accent-border);
    box-shadow: var(--sp-shadow-1);
    margin-bottom: var(--sp-5);
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

  .sp-ob-dots {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    padding: var(--sp-3) 0;
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