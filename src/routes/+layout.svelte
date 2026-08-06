<script lang="ts">
  import { page } from "$app/stores";
  let { children } = $props();
</script>

<nav class="topbar">
  <a href="/" class="brand">StackPilot</a>
  <div class="links">
    <a href="/" class:active={$page.url.pathname === "/"}>Home</a>
    <a href="/analyze" class:active={$page.url.pathname === "/analyze"}>Analyze</a>
    <a href="/create" class:active={$page.url.pathname.startsWith("/create")}>Create</a>
    <a href="/profiles" class:active={$page.url.pathname.startsWith("/profiles")}>Profiles</a>
    <a href="/processes" class:active={$page.url.pathname.startsWith("/processes")}>Processes</a>
    <a href="/environment" class:active={$page.url.pathname.startsWith("/environment")}>Environment</a>
    <a href="/workspace" class:active={$page.url.pathname.startsWith("/workspace")}>Workspace</a>
    <a href="/settings" class:active={$page.url.pathname === "/settings"}>Settings</a>
  </div>
</nav>

{@render children()}

<style>
  .topbar {
    display: flex;
    align-items: center;
    gap: 2rem;
    padding: 0.6rem 2rem;
    background: #ffffff;
    border-bottom: 1px solid #e0e0e0;
    box-shadow: 0 1px 3px rgba(0,0,0,0.06);
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .brand {
    font-weight: 700;
    font-size: 1.1rem;
    text-decoration: none;
    color: #222;
  }

  .links {
    display: flex;
    gap: 0.25rem;
  }

  .links a {
    text-decoration: none;
    color: #555;
    padding: 0.3rem 0.8rem;
    border-radius: 6px;
    font-size: 0.9rem;
    transition: background 0.15s, color 0.15s;
  }

  .links a:hover {
    background: #f0f0f0;
    color: #222;
  }

  .links a.active {
    background: #e8eaf6;
    color: #283593;
    font-weight: 600;
  }

  @media (prefers-color-scheme: dark) {
    .topbar {
      background: #1e1e1e;
      border-bottom-color: #333;
    }
    .brand {
      color: #eee;
    }
    .links a {
      color: #aaa;
    }
    .links a:hover {
      background: #333;
      color: #eee;
    }
    .links a.active {
      background: #1a237e;
      color: #c5cae9;
    }
  }
</style>
