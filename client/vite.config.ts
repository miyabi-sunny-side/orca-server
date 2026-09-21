import { fileURLToPath, URL } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: [
      {
        find: /^three\/addons\/(.*)$/,
        replacement: fileURLToPath(
          new URL("./vendor/examples/jsm/$1", import.meta.url),
        ),
      },
      {
        find: "three",
        replacement: fileURLToPath(
          new URL("./vendor/build/three.module.js", import.meta.url),
        ),
      },
    ],
  },
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:3000",
    },
  },
});
