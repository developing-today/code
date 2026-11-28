import { defineConfig } from "vitest/config";

// The Worker handler uses only the standard Fetch API plus env bindings,
// so the proxy contract is tested in a plain Node environment with the
// bridge fetch stubbed; no Workers pool is needed for these tests.
export default defineConfig({
  test: {
    environment: "node",
    include: ["test/**/*.test.ts"],
  },
});
