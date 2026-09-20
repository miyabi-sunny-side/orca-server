import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testIgnore: process.env.E2E_BASE_URL ? "layout.spec.ts" : "live.spec.ts",
  use: {
    baseURL: process.env.E2E_BASE_URL || "http://127.0.0.1:5187",
    browserName: "chromium",
  },
  webServer: process.env.E2E_BASE_URL
    ? undefined
    : {
        command: "npm run dev -- --port 5187 --strictPort",
        url: "http://127.0.0.1:5187",
      },
});
