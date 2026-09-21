import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    // Ambientes de preview (Arena/sandbox, túneis) servem o dev server por um
    // host externo: sem isto o Vite devolve 403 e a página não carrega.
    allowedHosts: true,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: [
        /[\\/]src-tauri[\\/]/,
        /[\\/]target[\\/]/,
        /[\\/]target_check[\\/]/,
        /[\\/]crates[\\/]/,
        /[\\/]\.git[\\/]/,
        /[\\/]baselines[\\/]/,
        /[\\/]scripts[\\/]/,
        /[\\/]SPRINTS[\\/]/,
        /\.(exe|dll|pdb|rlib|lock)$/,
      ],
    },
  },
}));
