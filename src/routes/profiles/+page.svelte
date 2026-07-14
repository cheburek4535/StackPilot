<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { listProfiles, getDemoProfile } from "$lib/api";
  import type { LaunchProfile } from "$lib/types";

  let profiles = $state<LaunchProfile[]>([]);
  let loading = $state(true);

  onMount(async () => {
    profiles = await listProfiles();
    loading = false;
  });

  function goToProfile(name: string) {
    goto(`/profiles/${encodeURIComponent(name)}`);
  }
</script>

<main>
  <h1>📋 Все профили</h1>
  <p class="subtitle">Сохранённые конфигурации запуска проектов</p>

  {#if loading}
    <p class="empty">Загрузка...</p>
  {:else if profiles.length === 0}
    <div class="empty-state">
      <p class="empty">Нет сохранённых профилей.</p>
      <p class="hint">Профили появятся здесь после того, как вы проанализируете проект или создадите профиль вручную.</p>
      <button class="primary" onclick={() => goto("/")}>На главную</button>
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
            <span class="count">{profile.actions.length} действий</span>
            <span class="arrow">→</span>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</main>

<style>
  :root {
    font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
    font-size: 16px;
    color: #0f0f0f;
    background-color: #f6f6f6;
  }

  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 2rem;
  }

  h1 {
    margin: 0 0 0.25rem;
    font-size: 1.3rem;
  }

  .subtitle {
    color: #888;
    margin: 0 0 1.5rem;
    font-size: 0.9rem;
  }

  .empty-state {
    text-align: center;
    padding: 3rem 1rem;
  }

  .empty {
    color: #999;
    font-style: italic;
  }

  .hint {
    color: #aaa;
    font-size: 0.85rem;
    margin: 0.5rem 0 1.5rem;
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
    border: 1px solid #e0e0e0;
    border-radius: 8px;
    background: #fff;
    cursor: pointer;
    text-align: left;
    font-size: 0.9rem;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
    transition: border-color 0.15s;
  }

  .profile-card:hover {
    border-color: #396cd8;
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
    color: #999;
    font-size: 0.8rem;
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
    font-size: 0.8rem;
    color: #888;
  }

  .arrow {
    color: #ccc;
    font-size: 1.1rem;
  }

  button.primary {
    padding: 0.6rem 1.4rem;
    border-radius: 8px;
    border: none;
    background: #396cd8;
    color: #fff;
    font-size: 0.95rem;
    cursor: pointer;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    .profile-card {
      background: #0f0f0f98;
      border-color: #444;
    }

    .profile-card:hover {
      border-color: #5b8def;
    }
  }
</style>
