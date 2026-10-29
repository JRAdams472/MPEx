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
  // Auth setup logs in via Dex once; other specs reuse the session cookie.
  // auth.spec exercises the login flow itself, so it runs state-free.
  projects: [
    { name: "setup", testMatch: /auth\.setup\.ts/ },
    {
      name: "chromium-auth",
      testMatch: /auth\.spec\.ts/,
    },
    {
      name: "chromium",
      dependencies: ["setup"],
      testIgnore: /auth\.(spec|setup)\.ts/,
      use: { storageState: "e2e/.auth/admin.json" },
    },
  ],
  webServer: {
    command: "npm run dev",
    url: process.env.E2E_BASE_URL ?? "http://localhost:3000",
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
