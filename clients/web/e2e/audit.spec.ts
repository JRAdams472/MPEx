import { expect, test } from "@playwright/test";

/** Audit admin UI (MPE-121): admins see the trail + verify the hash chain;
 *  viewers get no nav affordance and 403 through the BFF. */

async function dexLogin(page: import("@playwright/test").Page, user: string) {
  await page.goto("/");
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
  await page.fill('input[name="login"]', `${user}@mpex.dev`);
  await page.fill('input[name="password"]', `${user}123`);
  await page.click('button[type="submit"]');
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
}

test("audit: admin sees the trail, verifies the chain, expands detail", async ({
  page,
}) => {
  // Seed a guaranteed-audited action via the BFF so the table has a known row.
  const key = `e2e-audit.${Date.now()}`;
  const put = await page.request.put(`/bff/api/expressions/${key}`, {
    data: { kind: "standard", expression: "1 + 1" },
  });
  expect(put.ok()).toBeTruthy();

  await page.goto("/audit");
  // First-ever compile of a new route under next dev can take a while, and
  // the admin guard renders until /auth/me resolves — wait generously.
  await expect(
    page.getByRole("heading", { name: "Audit trail" }),
  ).toBeVisible({ timeout: 30_000 });

  // The seeded artifact.put row is findable via the key filter.
  await page.getByLabel("Artifact key").fill(key);
  const row = page.getByRole("row", { name: new RegExp(key) });
  await expect(row).toBeVisible({ timeout: 15_000 });
  await expect(row.getByText("artifact.put")).toBeVisible();
  await expect(row.getByText(/expressions/)).toBeVisible();

  // Detail expander shows the JSON (content hash, not a payload).
  await row.getByRole("button", { name: "expand detail" }).click();
  await expect(page.getByText(/"contentHash"/)).toBeVisible();

  // Verify chain renders the intact state.
  await page.getByRole("button", { name: "Verify chain" }).click();
  await expect(page.getByText(/Chain intact — \d+ rows verified/)).toBeVisible({
    timeout: 15_000,
  });

  // Clean the draft so the fixture doesn't leak into the dev catalog.
  const del = await page.request.delete(`/bff/api/expressions/${key}`);
  expect(del.status()).toBe(204);
});

test("audit: viewer gets no nav affordance and 403s through the BFF", async ({
  page,
}) => {
  // The sealed admin session is still valid — drop it or the middleware
  // never redirects to Dex for the viewer login.
  await page.context().clearCookies();
  await dexLogin(page, "viewer");
  await expect(
    page.getByRole("link", { name: "Audit" }),
  ).not.toBeVisible();

  // Direct nav renders the client-side admin guard, and the API denies.
  await page.goto("/audit");
  await expect(
    page.getByText(/requires the admin or auditor role/),
  ).toBeVisible({ timeout: 15_000 });
  const res = await page.request.get("/bff/api/audit?limit=1");
  expect(res.status()).toBe(403);
});

test("audit: auditor reads the trail but keeps no write or grants access", async ({
  page,
}) => {
  await page.context().clearCookies();
  await dexLogin(page, "auditor");
  // Audit nav opens for auditors; grants stays admin-only.
  await expect(page.getByRole("link", { name: "Audit" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Grants" })).not.toBeVisible();

  await page.goto("/audit");
  await expect(
    page.getByRole("heading", { name: "Audit trail" }),
  ).toBeVisible({ timeout: 30_000 });
  const res = await page.request.get("/bff/api/audit?limit=1");
  expect(res.ok()).toBeTruthy();

  // Write paths and grants admin still deny the read-only role.
  const put = await page.request.put(
    `/bff/api/expressions/e2e-auditor.${Date.now()}`,
    { data: { kind: "standard", expression: "1 + 1" } },
  );
  expect(put.status()).toBe(403);
  const grants = await page.request.get("/bff/api/admin/grants");
  expect(grants.status()).toBe(403);
});
