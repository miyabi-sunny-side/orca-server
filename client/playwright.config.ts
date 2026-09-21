import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: process.env.E2E_DEFAULTS_CONTEXT
    ? "**/defaults-live.spec.ts"
    : process.env.E2E_PLATE_CONTEXT
      ? "**/plate-live.spec.ts"
      : process.env.E2E_AMS_CONTEXT
        ? "**/ams-live.spec.ts"
        : process.env.E2E_FILAMENT_CONTEXT
          ? "**/filament-live.spec.ts"
          : process.env.E2E_REGISTRY_SETTINGS
            ? "**/registry-live.spec.ts"
            : process.env.E2E_BASE_URL
              ? "**/queue-live.spec.ts"
              : "**/*.spec.ts",
  testIgnore: process.env.E2E_BASE_URL ? undefined : /live\.spec\.ts$/,
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
