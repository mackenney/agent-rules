import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["tests/integration/**/*.test.ts"],
    environment: "node",
    pool: "forks",
    testTimeout: 120_000,
    hookTimeout: 30_000,
  },
});
