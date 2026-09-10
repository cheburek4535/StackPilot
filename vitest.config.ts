import { defineConfig } from "vitest/config";
import { sveltekit } from "@sveltejs/kit/vite";

// Изолированный конфиг для чистых юнит-тестов: sveltekit()-плагин
// даёт $lib/$app-алиасы и компиляцию runes-модулей (i18n.svelte.ts).
export default defineConfig({
  plugins: [sveltekit()],
  test: {
    include: [
      "src/lib/modules/toolchain/*.test.ts",
      "src/lib/modules/devlauncher/*.test.ts",
      "src/lib/modules/project_creator/*.test.ts",
    ],
    environment: "node",
  },
});
