import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

export default defineConfig(async ({ mode }) => {
  const host = process.env.TAURI_DEV_HOST;
  return {
    plugins: [sveltekit()],
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      host: "0.0.0.0",
      allowedHosts: ["*"],
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
