import { defineConfig } from "vitest/config";

// Изолированный конфиг для чистых юнит-тестов toolchain (без SvelteKit-
// плагина: тестируются только pure-function модули без $lib-алиасов).
export default defineConfig({
  test: {
    include: ["src/lib/modules/toolchain/*.test.ts"],
    environment: "node",
  },
});
