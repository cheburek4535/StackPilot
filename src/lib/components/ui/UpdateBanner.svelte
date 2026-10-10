<script lang="ts">
  import { updater } from "$lib/core/updater.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";
  import Icon from "./Icon.svelte";

  const release = $derived(updater.latestRelease);
  const visible = $derived(updater.hasUpdate && !updater.dismissed && release !== null);

  function handleOpen() {
    void updater.openRelease();
  }

  function handleDismiss(e: MouseEvent) {
    e.stopPropagation();
    updater.dismiss();
  }
</script>

{#if visible && release}
  <aside
    class="sp-update-banner"
    role="region"
    aria-label="Update notification"
    onclick={handleOpen}
  >
    <div class="sp-update-content">
      <div class="sp-update-badge" aria-hidden="true">
        <Icon name="sparkles" size={14} />
      </div>
      <div class="sp-update-message">
        <span class="sp-update-title">
          {i18n.t("update.banner_text", { version: release.tagName }) as TranslationKey}
        </span>
      </div>
    </div>

    <div class="sp-update-actions">
      <button
        type="button"
        class="sp-update-link-btn"
        onclick={handleOpen}
        title={i18n.t("update.view_release") as TranslationKey}
      >
        <span>{i18n.t("update.view_release") as TranslationKey}</span>
        <Icon name="external" size={13} />
      </button>

      <button
        type="button"
        class="sp-update-close-btn"
        onclick={handleDismiss}
        aria-label={i18n.t("update.dismiss") as TranslationKey}
        title={i18n.t("update.dismiss") as TranslationKey}
      >
        <Icon name="x" size={14} />
      </button>
    </div>
  </aside>
{/if}

<style>
  .sp-update-banner {
    position: relative;
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    min-height: 2.375rem;
    padding: 0.375rem var(--sp-4);
    background: linear-gradient(90deg, rgba(245, 158, 11, 0.18) 0%, rgba(217, 119, 6, 0.12) 100%);
    border-bottom: 1px solid rgba(245, 158, 11, 0.38);
    box-shadow: 0 2px 12px rgba(245, 158, 11, 0.08);
    cursor: pointer;
    z-index: 9;
    user-select: none;
    animation: sp-banner-slide-down 0.22s cubic-bezier(0.16, 1, 0.3, 1);
    transition: background 0.15s ease, border-color 0.15s ease;
  }

  .sp-update-banner:hover {
    background: linear-gradient(90deg, rgba(245, 158, 11, 0.24) 0%, rgba(217, 119, 6, 0.16) 100%);
    border-bottom-color: rgba(245, 158, 11, 0.5);
  }

  @keyframes sp-banner-slide-down {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .sp-update-content {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .sp-update-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--sp-radius-md);
    background: rgba(245, 158, 11, 0.25);
    border: 1px solid rgba(245, 158, 11, 0.5);
    color: #fbbf24;
    flex-shrink: 0;
    box-shadow: 0 0 8px rgba(245, 158, 11, 0.3);
  }

  .sp-update-message {
    font-size: var(--sp-fs-sm);
    color: var(--sp-text-1);
    font-weight: var(--sp-fw-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .sp-update-title {
    letter-spacing: -0.01em;
  }

  .sp-update-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    flex-shrink: 0;
  }

  .sp-update-link-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: 0.25rem 0.625rem;
    font-size: var(--sp-fs-xs);
    font-weight: var(--sp-fw-semibold);
    color: #fef3c7;
    background: rgba(245, 158, 11, 0.3);
    border: 1px solid rgba(245, 158, 11, 0.55);
    border-radius: var(--sp-radius-sm);
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease, transform 0.1s ease;
  }

  .sp-update-link-btn:hover {
    background: rgba(245, 158, 11, 0.45);
    border-color: rgba(245, 158, 11, 0.8);
    transform: translateY(-1px);
  }

  .sp-update-link-btn:active {
    transform: translateY(0);
  }

  .sp-update-close-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    padding: 0;
    border-radius: var(--sp-radius-sm);
    border: 1px solid transparent;
    background: transparent;
    color: rgba(255, 255, 255, 0.7);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .sp-update-close-btn:hover {
    background: rgba(245, 158, 11, 0.25);
    color: #ffffff;
  }
</style>
