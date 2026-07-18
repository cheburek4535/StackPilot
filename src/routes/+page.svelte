<script lang="ts">
  import { goto } from "$app/navigation";
</script>

<main>
  <div class="hero">
    <h1>StackPilot</h1>
    <p class="tagline">Development environment manager &mdash; analyze, configure, launch</p>
    <div class="hero-actions">
      <button class="primary" onclick={() => goto("/devlauncher")}>
        Open DevLauncher
      </button>
      <button class="secondary" onclick={() => goto("/settings")}>
        Settings
      </button>
    </div>
  </div>

  <section class="modules">
    <div class="module-card" onclick={() => goto("/devlauncher")} role="button" tabindex="0" onkeydown={(e) => e.key === "Enter" && goto("/devlauncher")}>
      <span class="module-icon">🚀</span>
      <div>
        <strong>DevLauncher</strong>
        <p>Analyze projects and manage launch profiles</p>
      </div>
    </div>
  </section>
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

  .hero {
    text-align: center;
    padding: 3rem 0 2rem;
  }

  h1 {
    margin: 0;
    font-size: 2rem;
  }

  .tagline {
    color: #888;
    font-size: 1rem;
    margin: 0.25rem 0 1.5rem;
  }

  .hero-actions {
    display: flex;
    gap: 0.75rem;
    justify-content: center;
    flex-wrap: wrap;
  }

  .modules {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .module-card {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem 1.25rem;
    background: #fff;
    border: 1px solid #e0e0e0;
    border-radius: 10px;
    cursor: pointer;
    transition: border-color 0.15s, box-shadow 0.15s;
    box-shadow: 0 1px 3px rgba(0,0,0,0.05);
  }

  .module-card:hover {
    border-color: #396cd8;
    box-shadow: 0 2px 8px rgba(57,108,216,0.12);
  }

  .module-icon {
    font-size: 1.8rem;
  }

  .module-card strong {
    display: block;
    font-size: 1rem;
    margin-bottom: 0.15rem;
  }

  .module-card p {
    margin: 0;
    font-size: 0.85rem;
    color: #888;
  }

  button.primary {
    padding: 0.6rem 1.4rem;
    border-radius: 8px;
    border: none;
    background: #396cd8;
    color: #fff;
    font-size: 0.95rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
  }

  button.primary:hover {
    background: #2b5ab0;
  }

  button.secondary {
    padding: 0.6rem 1.4rem;
    border-radius: 8px;
    border: 1px solid #ccc;
    background: #fff;
    color: #444;
    font-size: 0.95rem;
    cursor: pointer;
    transition: background 0.15s;
  }

  button.secondary:hover {
    background: #f0f0f0;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      color: #f6f6f6;
      background-color: #2f2f2f;
    }

    .module-card {
      background: #0f0f0f98;
      border-color: #444;
    }

    button.secondary {
      background: #2a2a2a;
      color: #ccc;
      border-color: #555;
    }
  }
</style>
