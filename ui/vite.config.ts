import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: "safari16",
    rollupOptions: {
      input: {
        viewer: resolve(__dirname, "viewer.html"),
        library: resolve(__dirname, "library.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
  },
});
