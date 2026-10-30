import { test, expect } from "@playwright/test";

// Environment lanes (MPE-153): the seeded stack publishes into dev; qa/uat/prod
// hold promoted rows only. This spec proves the lanes resolve independently —
// prod (empty) prices nothing, then a UI promote dev→qa makes qa behave like
// dev — driven entirely through the admin UI.
test("environments: promote dev→qa via UI, sandbox resolves the qa lane", async ({
  page,
}) => {
  // 1. prod is an empty lane: eligibility runs but nothing is eligible.
  await page.goto("/sandbox");
  await page.getByRole("combobox", { name: "Environment" }).click();
  await page.getByRole("option", { name: "prod" }).click();
  await page.getByRole("button", { name: /run eligibility/i }).click();
  await expect(page.getByText(/Eligibility — revision/).first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByText(/env prod/).first()).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Price 0 products" }),
  ).toBeVisible();

  // 2. Guarantee a dev→qa delta (a re-run's promote would 409 otherwise):
  //    draft a fresh expression and publish the dev lane via the BFF.
  const probe = `e2e-env-${Date.now()}`;
  const put = await page.request.put(`/bff/api/expressions/${probe}`, {
    data: { kind: "standard", expression: "1 + 1" },
  });
  expect(put.ok()).toBeTruthy();
  const publish = await page.request.post("/bff/api/publications", {
    data: { label: `e2e delta ${probe}` },
  });
  expect(publish.ok()).toBeTruthy();

  // 3. Publications → promote the dev lane to qa via the row action + dialog.
  await page.goto("/publications");
  await page
    .getByRole("button", { name: /promote → qa/i })
    .first()
    .click();
  await page.getByLabel("Promotion label").fill("e2e dev-to-qa");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Promote", exact: true })
    .click();
  await expect(page.getByText(/Promoted to qa/)).toBeVisible({
    timeout: 30_000,
  });
  await page.getByRole("button", { name: "Close" }).click();

  // The table now carries a qa-lane publication.
  await expect(
    page.getByRole("cell", { name: "qa", exact: true }).first(),
  ).toBeVisible();

  // 4. Sandbox in qa resolves the promoted catalog — eligible products exist
  //    (same keys as dev, different lane).
  await page.goto("/sandbox");
  await page.getByRole("combobox", { name: "Environment" }).click();
  await page.getByRole("option", { name: "qa" }).click();
  await page.getByRole("button", { name: /run eligibility/i }).click();
  await expect(page.getByText(/Eligibility — revision/).first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByText(/env qa/).first()).toBeVisible();
  await expect(
    page.getByRole("button", { name: /price [1-9]\d* product/i }),
  ).toBeVisible();
});
