<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { listProfiles } from "$lib/modules/devlauncher/api";
  import type { LaunchProfile } from "$lib/modules/devlauncher/types";
  import { notifyError } from "$lib/core/toasts";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let profiles = $state<LaunchProfile[]>([]);
  let loading = $state(true);

  onMount(async () => {
    try {
      profiles = await listProfiles();
    } catch (e) {
      notifyError(i18n.t("devl.profiles"), i18n.t("devl.load_profiles_failed", { err: String(e) }));
    }
    loading = false;
  });

  function goToProfile(name: string) {
    goto(`/devlauncher/profiles/${encodeURIComponent(name)}`);
  }
</script>

<main>
  <h1>{i18n.t("devl.profiles_title") as TranslationKey}</h1>
  <p class="subtitle">{i18n.t("devl.profiles_subtitle") as TranslationKey}</p>

  {#if loading}
    <p class="empty">{i18n.t("devl.profiles_loading") as TranslationKey}</p>
  {:else if profiles.length === 0}
    <div class="empty-state">
      <p class="empty">{i18n.t("devl.profiles_empty") as TranslationKey}</p>
      <p class="hint">{i18n.t("devl.profiles_hint") as TranslationKey}</p>
      <div class="empty-actions">
        <button class="primary" onclick={() => goto("/devlauncher/analyze")}>{i18n.t("devl.analyze_project") as TranslationKey}</button>
        <button class="secondary" onclick={() => goto("/devlauncher")}>{i18n.t("devl.back_to_overview") as TranslationKey}</button>
      </div>
    </div>
  {:else}
    <div class="profile-cards">
      {#each profiles as profile}
        <button class="profile-card" onclick={() => goToProfile(profile.name)}>
          <div class="card-main">
            <strong>{profile.name}</strong>
            <span class="desc">{profile.description}</span>
          </div>
          <div class="card-meta">
            <span class="count">{i18n.t("devl.actions_count", { n: profile.actions.length }) as TranslationKey}</span>
            <span class="arrow">→</span>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</main>

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
    color: var(--sp-text-1);
  }

  h1 {
    margin: 0 0 0.25rem;
    font-size: var(--sp-fs-xl);
    color: var(--sp-text-1);
  }

  .subtitle {
    color: var(--sp-text-3);
    margin: 0 0 1.5rem;
    font-size: var(--sp-fs-sm);
  }

  .empty-state {
    text-align: center;
    padding: 3rem 1rem;
  }

  .empty {
    color: var(--sp-text-3);
    font-style: italic;
  }

  .hint {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-sm);
    margin: 0.5rem 0 1.5rem;
  }

  .empty-actions {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
  }

  .profile-cards {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .profile-card {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: 0.8rem 1rem;
    border: 1px solid var(--sp-border);
    border-radius: var(--sp-radius-md);
    background: var(--sp-bg-1);
    color: var(--sp-text-1);
    cursor: pointer;
    text-align: left;
    font-size: var(--sp-fs-sm);
    font-family: var(--sp-font-sans);
    box-shadow: var(--sp-shadow-1);
    transition: border-color 0.15s;
  }

  .profile-card:hover {
    border-color: var(--sp-accent);
  }

  .card-main {
    flex: 1;
    min-width: 0;
  }

  .card-main strong {
    display: block;
    margin-bottom: 0.15rem;
  }

  .card-main .desc {
    color: var(--sp-text-3);
    font-size: var(--sp-fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin: 0;
  }

  .card-meta {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-shrink: 0;
  }

  .count {
    font-size: var(--sp-fs-xs);
    color: var(--sp-text-2);
  }

  .arrow {
    color: var(--sp-text-3);
    font-size: 1.1rem;
  }

  button.primary {
    padding: 0.6rem 1.4rem;
    border-radius: var(--sp-radius-md);
    border: none;
    background: var(--sp-accent-strong);
    color: #fff;
    font-size: var(--sp-fs-sm);
    font-weight: var(--sp-fw-semibold);
    cursor: pointer;
  }

  button.primary:hover:not(:disabled) {
    background: var(--sp-accent);
  }

  button.secondary {
    padding: 0.6rem 1.4rem;
    border-radius: var(--sp-radius-md);
    border: 1px solid var(--sp-border);
    background: var(--sp-bg-2);
    color: var(--sp-text-1);
    font-size: var(--sp-fs-sm);
    cursor: pointer;
  }

  button.secondary:hover:not(:disabled) {
    background: var(--sp-bg-3);
  }
</style>
