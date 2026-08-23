// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info

import { initTauriMock } from "$lib/core/tauriMock";

if (typeof window !== "undefined") {
  initTauriMock();
}

export const ssr = false;
export const prerender = false;
