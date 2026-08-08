import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, externalizeDepsPlugin } from "electron-vite";
import { resolve } from "node:path";

const desktopRoot = __dirname;

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
    resolve: {
      alias: {
        "@main": resolve(desktopRoot, "src/main"),
        "@shared": resolve(desktopRoot, "src/shared")
      }
    }
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    resolve: {
      alias: {
        "@shared": resolve(desktopRoot, "src/shared")
      }
    }
  },
  renderer: {
    root: resolve(desktopRoot, "src/renderer"),
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        "@renderer": resolve(desktopRoot, "src/renderer"),
        "@shared": resolve(desktopRoot, "src/shared")
      }
    },
    build: {
      rollupOptions: {
        input: resolve(desktopRoot, "src/renderer/index.html")
      }
    }
  }
});
