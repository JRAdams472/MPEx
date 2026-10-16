import { defineConfig } from "@playwright/test";

// e2e runs against the dev stack: Postgres + the Rust API must be up
// (docker compose up db api, or cargo run) and seeded — the specs assert on
// the demo catalog. The webServer block starts `next dev` if needed.
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  retries: 0,
  // github = CI annotations; html = artifact uploaded on failure.
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "http://localhost:3000",
  },
  webServer: {
    command: "npm run dev",
    url: process.env.E2E_BASE_URL ?? "http://localhost:3000",
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
