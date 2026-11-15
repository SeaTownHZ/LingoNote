import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      ignored: [
        "**/src-tauri/**",
        "**/target/**",
        "**/.tools/**",
        "**/node_modules/**",
        "**/*.tmp",
        "**/*.tmpdir/**",
      ],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: "index.html",
        settings: "settings.html",
      },
    },
  },
});
