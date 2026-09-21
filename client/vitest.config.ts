import vite from "./vite.config";
import { defineConfig, mergeConfig } from "vitest/config";

export default mergeConfig(
  vite,
  defineConfig({
    resolve: {
      conditions: ["browser"],
    },
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.ts"],
    },
  }),
);
