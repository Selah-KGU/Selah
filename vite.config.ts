import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  publicDir: "static",
  esbuild: {
    drop: process.env.TAURI_DEBUG ? [] : ["console", "debugger"],
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "esnext",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      output: {
        // Split heavy vendor libs into their own chunk so the browser can
        // parse them in parallel with the main app bundle on cold start.
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (id.includes("marked")) return "vendor-marked";
            if (id.includes("dompurify")) return "vendor-dompurify";
            if (id.includes("html-to-image")) return "vendor-html-to-image";
          }
        },
      },
    },
    modulePreload: false,
    cssCodeSplit: false,
  },
});
