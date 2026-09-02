import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/features/library/**/*.test.{ts,tsx}"],
  },
});

